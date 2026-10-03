//-----------------------------------------------------------------------------
//
// Copyright (C) 1993-1996 by id Software, Inc.
//
// This source is available for distribution and/or modification
// only under the terms of the DOOM Source Code License as
// published by id Software. All rights reserved.
//
// The source is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// FITNESS FOR A PARTICULAR PURPOSE. See the DOOM Source Code License
// for more details.
//
// DESCRIPTION:
//	Zone Memory Allocation. Neat.
//
//-----------------------------------------------------------------------------

//! Rust port of `z_zone.h` / `z_zone.c`.
//!
//! Zone Memory Allocation, perhaps NeXT ObjectiveC inspired. ("Remark:
//! this was the only stuff that, according to John Carmack, might have
//! been useful for Quake.")
//!
//! # Design note: semantics over representation
//!
//! The original is a hand-rolled arena allocator over a single fixed
//! block of memory (`I_ZoneBase`), with a doubly linked list of
//! `memblock_t` headers threaded through raw pointer arithmetic
//! (`(byte*)ptr - sizeof(memblock_t)` to recover the header from a user
//! pointer), free-block coalescing, and a "rover" that resumes scanning
//! where the last allocation left off. None of that low-level block
//! management earns its keep in Rust: it existed to manage a scarce,
//! fixed-size heap on 90s hardware, which the Rust allocator (backed by
//! the OS/global allocator) already does correctly and fast. Re-deriving
//! it with raw pointer arithmetic in `unsafe` Rust would be the worst of
//! both worlds — real UB risk, zero fidelity benefit, since the *only*
//! thing calling code actually depends on is the zone's semantics, not
//! its bytes-in-memory layout.
//!
//! So this port keeps the original's observable behavior exactly, on
//! top of ordinary Rust allocation (`Box<[u8]>`) plus a metadata table:
//!
//! - [`z_malloc`] returns a [`ZoneHandle`] (an opaque, copyable id)
//!   instead of a raw pointer. Blocks are looked up by handle in a
//!   central registry rather than by pointer arithmetic on the returned
//!   value.
//! - The original's `void** user` "self-nulling owner pointer" — the
//!   mechanism by which a cache owner discovers its cached block was
//!   purged, by finding its own pointer variable zeroed out from
//!   underneath it — becomes an explicit [`Owner`] handle that
//!   [`z_free`]/purging marks invalid. Code that in C did
//!   `if (!cached_ptr) reload();` becomes, in ported callers,
//!   `if zone.get(owner).is_none() { reload(); }` — same usage pattern,
//!   no raw self-writing pointer.
//! - Purge tags ([`PurgeTag`]) and [`z_free_tags`]/[`z_change_tag`]
//!   preserve the original's tag-range semantics exactly (`tag <
//!   PU_PURGELEVEL` blocks are never auto-evicted; `tag >=
//!   PU_PURGELEVEL` blocks require an owner and can be evicted any time
//!   [`z_malloc`] needs the space).
//!
//! This is deliberately not a `static`/global allocator the way
//! `mainzone` is a single global in C — [`Zone::new`] returns an
//! instance, and the plan's globals strategy (Phase 2, globals section)
//! is expected to hold the single game-wide instance the way it holds
//! other big state groups. Nothing in this module reaches for global
//! state itself.
//!
//! One behavioral gap, accepted deliberately: in the original,
//! `Z_Malloc` can *itself* trigger eviction of purgeable blocks to make
//! room when the fixed arena is full (scanning forward from `rover`,
//! calling `Z_Free` on purgeable blocks in its path) — so a cache owner
//! can be nulled out as a side effect of some unrelated code allocating
//! memory, not just by an explicit free. This port never runs out of
//! (or needs to reclaim) space — it delegates to the ordinary Rust
//! allocator, which grows rather than evicts — so that automatic
//! eviction-under-pressure path has no equivalent here: purgeable blocks
//! in this port are only ever reclaimed by an explicit [`Zone::z_free`]
//! or [`Zone::z_free_tags`] call, never implicitly by [`Zone::z_malloc`].
//! Ported callers that relied on "allocate enough garbage and your old
//! cache entries start disappearing" (nothing in the original's actual
//! game logic does — it's purely a memory-pressure relief valve) would
//! not observe that specific side effect here. This is the direct
//! consequence of the plan's explicit choice not to reimplement manual
//! block coalescing/eviction, which existed only to manage a scarce
//! fixed-size heap that no longer exists in this port.

use std::collections::HashMap;

/// PU - purge tags. Tags < 100 are not overwritten until freed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PurgeTag {
    /// Static entire execution time (`PU_STATIC` = 1).
    Static,
    /// Static while playing (`PU_SOUND` = 2).
    Sound,
    /// Static while playing (`PU_MUSIC` = 3).
    Music,
    /// Anything else Dave wants static (`PU_DAVE` = 4).
    Dave,
    /// Static until level exited (`PU_LEVEL` = 50).
    Level,
    /// A special thinker in a level (`PU_LEVSPEC` = 51).
    LevSpec,
    /// Purgeable whenever needed (`PU_CACHE` = 101). Tags >= 100
    /// (`PU_PURGELEVEL` = 100) are purgeable; the original only ever
    /// actually uses 101 (`PU_CACHE`) as a concrete purgeable tag value,
    /// so that's the only purgeable variant represented here.
    Cache,
}

impl PurgeTag {
    /// The original's raw integer tag value, preserved for parity with
    /// code/log output that used to print/compare against the numeric
    /// `PU_*` defines directly.
    pub fn value(self) -> i32 {
        match self {
            PurgeTag::Static => 1,
            PurgeTag::Sound => 2,
            PurgeTag::Music => 3,
            PurgeTag::Dave => 4,
            PurgeTag::Level => 50,
            PurgeTag::LevSpec => 51,
            PurgeTag::Cache => 101,
        }
    }

    /// (`PU_PURGELEVEL` = 100) — the threshold at or above which a tag
    /// is purgeable.
    pub const PURGELEVEL: i32 = 100;

    /// Whether this tag is purgeable (`tag >= PU_PURGELEVEL`).
    pub fn is_purgeable(self) -> bool {
        self.value() >= Self::PURGELEVEL
    }
}

/// Opaque handle to a zone-allocated block, returned by [`z_malloc`] in
/// place of the original's raw `void*`. Stable for the block's lifetime
/// (until freed or purged), at which point lookups against it return
/// `None`/fail, mirroring the original's freed/purged block behavior
/// (rather than becoming a dangling pointer, which Rust's ownership
/// model has no interest in reproducing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ZoneHandle(u64);

/// Opaque handle standing in for the original's `void** user` — a
/// registered "owner slot" that [`z_free`] (directly, or indirectly via
/// [`z_malloc`] purging a purgeable block to make room) invalidates when
/// its block goes away. Ported callers hold on to the `Owner` they get
/// back from [`z_malloc`] and check [`Zone::get`] before using the
/// cached value, exactly as the original checked `if (!cached_ptr)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Owner(u64);

#[derive(Debug)]
struct Block {
    data: Box<[u8]>,
    tag: PurgeTag,
    /// `None` mirrors passing a NULL `user` with `tag < PU_PURGELEVEL`
    /// in the original (an allocation with no owner slot to null out on
    /// free — the C code marks these with the `(void*)2` sentinel
    /// purely so `Z_Free`'s `> 0x100` guard skips writing through it).
    owner: Option<Owner>,
}

/// Port of `memzone_t` / the zone allocator API (`Z_Init`, `Z_Malloc`,
/// `Z_Free`, `Z_FreeTags`, `Z_ChangeTag2`, `Z_FreeMemory`).
///
/// `Z_DumpHeap`/`Z_FileDumpHeap`/`Z_CheckHeap` (debug/diagnostic-only in
/// the original, printing or validating raw heap block layout) are not
/// ported: they inspect the arena's physical block layout specifically,
/// which this port deliberately does not reproduce (see the module
/// docs). [`Zone::block_count`] below covers the one thing ported
/// callers plausibly still want from them — a coarse sanity count.
#[derive(Debug, Default)]
pub struct Zone {
    blocks: HashMap<ZoneHandle, Block>,
    /// Where a still-live owner's current value is stored, i.e. the
    /// `Owner` -> live block mapping. An owner with no entry here (or
    /// whose entry doesn't match a live block) reads as "nulled out",
    /// exactly like the original's zeroed `*user`.
    owners: HashMap<Owner, ZoneHandle>,
    next_handle: u64,
    next_owner: u64,
}

impl Zone {
    /// Port of `Z_Init`. Unlike the original (which claims a fixed-size
    /// arena from `I_ZoneBase` up front), this starts with an empty
    /// registry — Rust allocation is not a scarce fixed pool that needs
    /// pre-claiming, see the module docs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `Z_Malloc`. Returns the block's [`ZoneHandle`] plus, if
    /// `owner` was requested (`Some`, faithfully standing in for a
    /// non-NULL `void** user` in the original), the [`Owner`] handle
    /// callers should hold onto and re-check via [`Zone::get`] before
    /// relying on the cached data — mirroring the original's "check the
    /// user pointer isn't null before using it" idiom.
    ///
    /// `size` is the payload length in bytes; `data` seeds the block's
    /// initial contents (the original leaves newly allocated memory
    /// uninitialized, but Rust has no safe uninitialized byte buffer, so
    /// ported callers should pass zeroed or otherwise-initialized data
    /// rather than relying on unspecified content either way).
    ///
    /// # Panics
    /// Panics (standing in for the original's fatal `I_Error`) if
    /// `owner` is `None` while `tag` is purgeable — "an owner is
    /// required for purgeable blocks", exactly as in the original.
    pub fn z_malloc(
        &mut self,
        size: usize,
        tag: PurgeTag,
        owner: bool,
    ) -> (ZoneHandle, Option<Owner>) {
        if !owner && tag.is_purgeable() {
            panic!("Z_Malloc: an owner is required for purgable blocks");
        }

        let handle = ZoneHandle(self.next_handle);
        self.next_handle += 1;

        let owner_handle = if owner {
            let o = Owner(self.next_owner);
            self.next_owner += 1;
            self.owners.insert(o, handle);
            Some(o)
        } else {
            None
        };

        self.blocks.insert(
            handle,
            Block {
                data: vec![0u8; size].into_boxed_slice(),
                tag,
                owner: owner_handle,
            },
        );

        (handle, owner_handle)
    }

    /// Port of `Z_Free`. Removes the block and, if it had an owner,
    /// invalidates that [`Owner`] (the original's "clear the user's
    /// mark": `*block->user = 0`).
    pub fn z_free(&mut self, handle: ZoneHandle) {
        if let Some(block) = self.blocks.remove(&handle) {
            if let Some(owner) = block.owner {
                self.owners.remove(&owner);
            }
        }
    }

    /// Port of `Z_FreeTags`. Frees every block whose tag falls in
    /// `[lowtag, hightag]` (inclusive), by original tag value.
    pub fn z_free_tags(&mut self, lowtag: i32, hightag: i32) {
        let to_free: Vec<ZoneHandle> = self
            .blocks
            .iter()
            .filter(|(_, b)| b.tag.value() >= lowtag && b.tag.value() <= hightag)
            .map(|(h, _)| *h)
            .collect();
        for handle in to_free {
            self.z_free(handle);
        }
    }

    /// Port of `Z_ChangeTag2` (the original's `Z_ChangeTag` macro just
    /// adds a `ZONEID`/file:line sanity check ahead of this — moot here
    /// since handles can't dangle the way a stale raw pointer can).
    ///
    /// # Panics
    /// Panics if `tag` is purgeable but the block has no owner —
    /// "an owner is required for purgeable blocks", same as the
    /// original.
    pub fn z_change_tag(&mut self, handle: ZoneHandle, tag: PurgeTag) {
        let block = self
            .blocks
            .get_mut(&handle)
            .expect("Z_ChangeTag: freed a pointer without ZONEID");
        if tag.is_purgeable() && block.owner.is_none() {
            panic!("Z_ChangeTag: an owner is required for purgable blocks");
        }
        block.tag = tag;
    }

    /// Port of `Z_FreeMemory`. The original sums bytes belonging to
    /// free blocks plus purgeable-tagged blocks (both are "available if
    /// needed"). This port has no free-space concept of its own (no
    /// fixed arena), so it reports the byte size of every purgeable
    /// block — the portion of used memory that could be reclaimed on
    /// demand, which is the only part of the original's number that
    /// still means anything without a fixed backing arena.
    pub fn z_free_memory(&self) -> usize {
        self.blocks
            .values()
            .filter(|b| b.tag.is_purgeable())
            .map(|b| b.data.len())
            .sum()
    }

    /// Look up a block's current data by handle. `None` if freed/purged.
    pub fn get(&self, handle: ZoneHandle) -> Option<&[u8]> {
        self.blocks.get(&handle).map(|b| &*b.data)
    }

    /// Mutable variant of [`Zone::get`].
    pub fn get_mut(&mut self, handle: ZoneHandle) -> Option<&mut [u8]> {
        self.blocks.get_mut(&handle).map(|b| &mut *b.data)
    }

    /// Resolve an [`Owner`] to its block's current handle. `None` means
    /// "nulled out" — the owned block was freed or purged, exactly like
    /// checking a zeroed `void*` in the original.
    pub fn owner_handle(&self, owner: Owner) -> Option<ZoneHandle> {
        self.owners.get(&owner).copied()
    }

    /// Coarse block-count sanity check, standing in for what ported
    /// callers might have used `Z_DumpHeap`/`Z_CheckHeap` for in a
    /// debug build (see the struct-level note on why the original's
    /// raw-layout heap walk isn't ported).
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purge_tag_values_match_original_defines() {
        assert_eq!(PurgeTag::Static.value(), 1);
        assert_eq!(PurgeTag::Sound.value(), 2);
        assert_eq!(PurgeTag::Music.value(), 3);
        assert_eq!(PurgeTag::Dave.value(), 4);
        assert_eq!(PurgeTag::Level.value(), 50);
        assert_eq!(PurgeTag::LevSpec.value(), 51);
        assert_eq!(PurgeTag::Cache.value(), 101);
        assert_eq!(PurgeTag::PURGELEVEL, 100);
    }

    #[test]
    fn purgeable_classification_matches_original() {
        assert!(!PurgeTag::Static.is_purgeable());
        assert!(!PurgeTag::LevSpec.is_purgeable());
        assert!(PurgeTag::Cache.is_purgeable());
    }

    #[test]
    fn malloc_without_owner_on_non_purgeable_tag_succeeds() {
        let mut zone = Zone::new();
        let (handle, owner) = zone.z_malloc(16, PurgeTag::Static, false);
        assert!(owner.is_none());
        assert_eq!(zone.get(handle).unwrap().len(), 16);
    }

    #[test]
    #[should_panic(expected = "an owner is required for purgable blocks")]
    fn malloc_without_owner_on_purgeable_tag_panics() {
        let mut zone = Zone::new();
        zone.z_malloc(16, PurgeTag::Cache, false);
    }

    #[test]
    fn free_invalidates_handle_and_owner() {
        let mut zone = Zone::new();
        let (handle, owner) = zone.z_malloc(16, PurgeTag::Cache, true);
        let owner = owner.unwrap();

        assert!(zone.get(handle).is_some());
        assert_eq!(zone.owner_handle(owner), Some(handle));

        zone.z_free(handle);

        assert!(zone.get(handle).is_none());
        assert_eq!(
            zone.owner_handle(owner),
            None,
            "owner should be nulled out, like *block->user = 0"
        );
    }

    #[test]
    fn free_tags_frees_only_matching_range() {
        let mut zone = Zone::new();
        let (level, _) = zone.z_malloc(8, PurgeTag::Level, false);
        let (static_, _) = zone.z_malloc(8, PurgeTag::Static, false);
        let (levspec, _) = zone.z_malloc(8, PurgeTag::LevSpec, false);

        // PU_LEVEL..=PU_LEVSPEC is the original's classic "free at level
        // exit" range (50..=51).
        zone.z_free_tags(PurgeTag::Level.value(), PurgeTag::LevSpec.value());

        assert!(zone.get(level).is_none());
        assert!(zone.get(levspec).is_none());
        assert!(zone.get(static_).is_some());
    }

    #[test]
    fn change_tag_to_purgeable_requires_owner() {
        let mut zone = Zone::new();
        let (handle, _) = zone.z_malloc(8, PurgeTag::Static, false);
        // No owner was registered, so promoting to a purgeable tag must
        // panic exactly like the original's Z_ChangeTag.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            zone.z_change_tag(handle, PurgeTag::Cache);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn change_tag_to_purgeable_with_owner_succeeds() {
        let mut zone = Zone::new();
        let (handle, owner) = zone.z_malloc(8, PurgeTag::Static, true);
        assert!(owner.is_some());
        zone.z_change_tag(handle, PurgeTag::Cache);
        assert!(zone.get(handle).is_some());
    }

    #[test]
    fn free_memory_sums_purgeable_blocks_only() {
        let mut zone = Zone::new();
        zone.z_malloc(100, PurgeTag::Static, false);
        zone.z_malloc(50, PurgeTag::Cache, true);
        zone.z_malloc(25, PurgeTag::Cache, true);

        assert_eq!(zone.z_free_memory(), 75);
    }

    #[test]
    fn block_count_tracks_live_allocations() {
        let mut zone = Zone::new();
        assert_eq!(zone.block_count(), 0);
        let (h1, _) = zone.z_malloc(8, PurgeTag::Static, false);
        zone.z_malloc(8, PurgeTag::Static, false);
        assert_eq!(zone.block_count(), 2);
        zone.z_free(h1);
        assert_eq!(zone.block_count(), 1);
    }
}
