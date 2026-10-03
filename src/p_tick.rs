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
//	Archiving: SaveGame I/O.
//	Thinker, Ticker.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_tick.h` / `p_tick.c`.
//!
//! Thinker list management: `P_InitThinkers`, `P_AddThinker`,
//! `P_RemoveThinker`, `P_RunThinkers`, plus [`p_ticker`] (`P_Ticker`,
//! completed in Phase 6e — see its own docs for what's still
//! `TODO(Phase 7)`).
//!
//! # Representation
//!
//! The original threads a doubly linked list (`thinker_t.prev`/`next`,
//! raw pointers) through heap blocks of several distinct structs —
//! `mobj_t` plus the specials' movers (`vldoor_t`, `ceiling_t`, ...,
//! ported in later phases) — all sharing the same `thinker_t` header,
//! identified at runtime by comparing `function` against each handler's
//! address (`P_MobjThinker`, `T_MoveCeiling`, ...).
//!
//! This port uses a single arena ([`Thinkers`]) of slots, each holding
//! one [`ThinkerData`] payload and a [`ThinkFn`] tag standing in for the
//! original's function-pointer identity. [`ThinkerId`] (generational
//! index) replaces every raw `thinker_t*`/`mobj_t*` a thinker or mobj
//! could hold (`target`, `tracer`, `specialdata`, sector `thinglist`,
//! blockmap links, ...) — see `r_defs::Mobj`'s docs for the fields that
//! now use it.
//!
//! The linked list itself (`prev`/`next`) is reproduced with slot
//! indices, in the same insertion order the original uses (`P_AddThinker`
//! always inserts right before the sentinel, i.e. at the tail) — this
//! order matters for demo-compatible `P_Random` call sequencing once
//! later phases add thinkers mid-tic.
//!
//! Removal is deferred exactly like the original: [`Thinkers::remove`]
//! only tags the slot [`ThinkFn::Removed`] (the C `-1` sentinel); the
//! slot is actually freed when [`Thinkers::run_thinkers`] walks past it
//! — so a thinker removing itself or another thinker mid-tic behaves
//! the same as in C.
//!
//! # Divergence: stale references become `None`, not use-after-free
//!
//! In the original, a `target`/`tracer` pointing at an already-freed
//! `mobj_t` is a dangling pointer into `z_zone` memory that may have
//! been reused — reading it is undefined behavior the original engine
//! happens to get away with. Here, [`ThinkerId`]'s generation counter
//! means [`Thinkers::get`]/`get_mut` on a freed-and-possibly-reused slot
//! returns `None` instead of aliasing a different live thinker. Callers
//! treat `None` the way the original's code treats a still-valid-looking
//! but logically-gone pointer (skip/ignore) — documented here since it's
//! the one place this port can't reproduce the original's actual memory
//! behavior, only its intent.

use crate::p_ceilng::Ceiling;
use crate::p_doors::VlDoor;
use crate::p_floor::FloorMove;
use crate::p_lights::{FireFlicker, Glow, LightFlash, Strobe};
use crate::p_plats::Plat;
use crate::r_defs::Mobj;

/// A generational index into [`Thinkers`]' arena, replacing every
/// `thinker_t*`/`mobj_t*` the original threads through mobj/thinker
/// fields (`target`, `tracer`, sector `thinglist`, blockmap links,
/// `specialdata`, ...). See module docs on why this needs a generation
/// (not just a slot index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThinkerId {
    index: u32,
    generation: u32,
}

impl ThinkerId {
    /// An id that never resolves (slot 0 is the list's sentinel and no
    /// live slot ever has this generation) — `players[i].mo` of a player
    /// who isn't in the game (the original's `mo == NULL`).
    pub const NONE: ThinkerId = ThinkerId {
        index: 0,
        generation: u32::MAX,
    };
}

/// Which handler a thinker slot runs, standing in for the original's
/// `thinker_t.function` (a `P_MobjThinker`/`T_MoveCeiling`/.../`NULL`/
/// `-1` function pointer). Only [`ThinkFn::MobjThinker`] is dispatched
/// as of Phase 6b — the specials variants are added when their owning
/// phase (7) ports the corresponding `p_*.c` movers; until then nothing
/// ever constructs them, they just reserve the enum shape `ThinkerData`
/// needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkFn {
    /// In stasis (the original's `function.acv == NULL`) — e.g. a
    /// platform/ceiling briefly paused, still in the list but not
    /// ticking.
    Null,
    /// Removed, awaiting sweep (the original's `function.acv == (void*)
    /// -1`) — see module docs on deferred removal.
    Removed,
    /// `P_MobjThinker`.
    MobjThinker,
    /// `T_FireFlicker`.
    FireFlicker,
    /// `T_LightFlash`.
    LightFlash,
    /// `T_StrobeFlash`.
    StrobeFlash,
    /// `T_Glow`.
    Glow,
    /// `T_MoveFloor`.
    FloorMove,
    /// `T_PlatRaise`.
    PlatRaise,
    /// `T_VerticalDoor`.
    VerticalDoor,
    /// `T_MoveCeiling`.
    MoveCeiling,
}

/// The payload a thinker slot owns, standing in for the original's
/// several distinct heap block shapes sharing a `thinker_t` header. Only
/// [`ThinkerData::Mobj`] exists as of Phase 6b.
#[derive(Debug)]
pub enum ThinkerData {
    Mobj(Mobj),
    FireFlicker(FireFlicker),
    LightFlash(LightFlash),
    StrobeFlash(Strobe),
    Glow(Glow),
    FloorMove(FloorMove),
    Plat(Plat),
    VlDoor(VlDoor),
    Ceiling(Ceiling),
}

impl ThinkerData {
    /// Borrows the payload as a [`Mobj`], if that's what this slot holds.
    pub fn as_mobj(&self) -> Option<&Mobj> {
        match self {
            ThinkerData::Mobj(m) => Some(m),
            _ => None,
        }
    }

    /// Mutably borrows the payload as a [`Mobj`], if that's what this
    /// slot holds.
    pub fn as_mobj_mut(&mut self) -> Option<&mut Mobj> {
        match self {
            ThinkerData::Mobj(m) => Some(m),
            _ => None,
        }
    }
}

/// One arena slot: the thinker header (`function`, list links) plus its
/// payload. `data` is `None` only for the sentinel slot (index 0, never
/// exposed as a [`ThinkerId`]) — every real thinker always holds data,
/// even once [`ThinkFn::Removed`] (freed on the next [`Thinkers::sweep`]
/// pass, matching the original's `Z_Free` happening inside
/// `P_RunThinkers`, not `P_RemoveThinker`).
struct Slot {
    generation: u32,
    prev: u32,
    next: u32,
    function: ThinkFn,
    data: Option<ThinkerData>,
}

/// The arena + list, standing in for `thinkercap` (the original's
/// sentinel `thinker_t` — `prev`/`next` here play the same role via
/// slot 0, which is never handed out as a [`ThinkerId`]). See module
/// docs.
pub struct Thinkers {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

impl Default for Thinkers {
    fn default() -> Self {
        Self::new()
    }
}

impl Thinkers {
    /// Port of `P_InitThinkers`.
    pub fn new() -> Self {
        Thinkers {
            // Slot 0 is the sentinel (`thinkercap`); `prev`/`next` both
            // point at itself, exactly like the original's
            // `thinkercap.prev = thinkercap.next = &thinkercap`.
            slots: vec![Slot {
                generation: 0,
                prev: 0,
                next: 0,
                function: ThinkFn::Null,
                data: None,
            }],
            free: Vec::new(),
        }
    }

    fn alloc_slot(&mut self, function: ThinkFn, data: ThinkerData) -> u32 {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.generation = slot.generation.wrapping_add(1);
            slot.function = function;
            slot.data = Some(data);
            index
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                prev: 0,
                next: 0,
                function,
                data: Some(data),
            });
            index
        }
    }

    /// Port of `P_AddThinker`: links a new thinker in at the tail (right
    /// before the sentinel), like the original.
    pub fn add_thinker(&mut self, function: ThinkFn, data: ThinkerData) -> ThinkerId {
        let index = self.alloc_slot(function, data);
        let tail = self.slots[0].prev;
        self.slots[tail as usize].next = index;
        self.slots[index as usize].next = 0;
        self.slots[index as usize].prev = tail;
        self.slots[0].prev = index;
        ThinkerId {
            index,
            generation: self.slots[index as usize].generation,
        }
    }

    /// Port of `P_RemoveThinker`: tags the slot removed; the actual free
    /// happens when [`Thinkers::run_thinkers`] sweeps past it (see
    /// module docs on deferred removal).
    pub fn remove(&mut self, id: ThinkerId) {
        if let Some(slot) = self.slot_mut(id) {
            slot.function = ThinkFn::Removed;
        }
    }

    fn slot(&self, id: ThinkerId) -> Option<&Slot> {
        let slot = self.slots.get(id.index as usize)?;
        (slot.generation == id.generation).then_some(slot)
    }

    fn slot_mut(&mut self, id: ThinkerId) -> Option<&mut Slot> {
        let slot = self.slots.get_mut(id.index as usize)?;
        (slot.generation == id.generation).then_some(slot)
    }

    /// Borrows a live thinker's payload as a [`Mobj`], or `None` if
    /// `id` is stale/removed/not a mobj — see module docs on the
    /// stale-reference divergence.
    pub fn mobj(&self, id: ThinkerId) -> Option<&Mobj> {
        self.slot(id)?.data.as_ref()?.as_mobj()
    }

    /// Mutable counterpart of [`Thinkers::mobj`].
    pub fn mobj_mut(&mut self, id: ThinkerId) -> Option<&mut Mobj> {
        self.slot_mut(id)?.data.as_mut()?.as_mobj_mut()
    }

    /// Borrows a live thinker's payload as a [`FireFlicker`].
    pub fn fire_flicker(&self, id: ThinkerId) -> Option<&FireFlicker> {
        match self.slot(id)?.data.as_ref()? {
            ThinkerData::FireFlicker(f) => Some(f),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Thinkers::fire_flicker`].
    pub fn fire_flicker_mut(&mut self, id: ThinkerId) -> Option<&mut FireFlicker> {
        match self.slot_mut(id)?.data.as_mut()? {
            ThinkerData::FireFlicker(f) => Some(f),
            _ => None,
        }
    }

    /// Borrows a live thinker's payload as a [`LightFlash`].
    pub fn light_flash(&self, id: ThinkerId) -> Option<&LightFlash> {
        match self.slot(id)?.data.as_ref()? {
            ThinkerData::LightFlash(f) => Some(f),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Thinkers::light_flash`].
    pub fn light_flash_mut(&mut self, id: ThinkerId) -> Option<&mut LightFlash> {
        match self.slot_mut(id)?.data.as_mut()? {
            ThinkerData::LightFlash(f) => Some(f),
            _ => None,
        }
    }

    /// Borrows a live thinker's payload as a [`Strobe`].
    pub fn strobe(&self, id: ThinkerId) -> Option<&Strobe> {
        match self.slot(id)?.data.as_ref()? {
            ThinkerData::StrobeFlash(f) => Some(f),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Thinkers::strobe`].
    pub fn strobe_mut(&mut self, id: ThinkerId) -> Option<&mut Strobe> {
        match self.slot_mut(id)?.data.as_mut()? {
            ThinkerData::StrobeFlash(f) => Some(f),
            _ => None,
        }
    }

    /// Borrows a live thinker's payload as a [`Glow`].
    pub fn glow(&self, id: ThinkerId) -> Option<&Glow> {
        match self.slot(id)?.data.as_ref()? {
            ThinkerData::Glow(g) => Some(g),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Thinkers::glow`].
    pub fn glow_mut(&mut self, id: ThinkerId) -> Option<&mut Glow> {
        match self.slot_mut(id)?.data.as_mut()? {
            ThinkerData::Glow(g) => Some(g),
            _ => None,
        }
    }

    /// Borrows a live thinker's payload as a [`FloorMove`].
    pub fn floor_move(&self, id: ThinkerId) -> Option<&FloorMove> {
        match self.slot(id)?.data.as_ref()? {
            ThinkerData::FloorMove(f) => Some(f),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Thinkers::floor_move`].
    pub fn floor_move_mut(&mut self, id: ThinkerId) -> Option<&mut FloorMove> {
        match self.slot_mut(id)?.data.as_mut()? {
            ThinkerData::FloorMove(f) => Some(f),
            _ => None,
        }
    }

    /// Borrows a live thinker's payload as a [`Plat`].
    pub fn plat(&self, id: ThinkerId) -> Option<&Plat> {
        match self.slot(id)?.data.as_ref()? {
            ThinkerData::Plat(p) => Some(p),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Thinkers::plat`].
    pub fn plat_mut(&mut self, id: ThinkerId) -> Option<&mut Plat> {
        match self.slot_mut(id)?.data.as_mut()? {
            ThinkerData::Plat(p) => Some(p),
            _ => None,
        }
    }

    /// Borrows a live thinker's payload as a [`VlDoor`].
    pub fn vl_door(&self, id: ThinkerId) -> Option<&VlDoor> {
        match self.slot(id)?.data.as_ref()? {
            ThinkerData::VlDoor(d) => Some(d),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Thinkers::vl_door`].
    pub fn vl_door_mut(&mut self, id: ThinkerId) -> Option<&mut VlDoor> {
        match self.slot_mut(id)?.data.as_mut()? {
            ThinkerData::VlDoor(d) => Some(d),
            _ => None,
        }
    }

    /// Borrows a live thinker's payload as a [`Ceiling`].
    pub fn ceiling(&self, id: ThinkerId) -> Option<&Ceiling> {
        match self.slot(id)?.data.as_ref()? {
            ThinkerData::Ceiling(c) => Some(c),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Thinkers::ceiling`].
    pub fn ceiling_mut(&mut self, id: ThinkerId) -> Option<&mut Ceiling> {
        match self.slot_mut(id)?.data.as_mut()? {
            ThinkerData::Ceiling(c) => Some(c),
            _ => None,
        }
    }

    /// Whether `id` still refers to a live (not removed/stale) thinker.
    pub fn is_live(&self, id: ThinkerId) -> bool {
        !matches!(
            self.slot(id).map(|s| s.function),
            None | Some(ThinkFn::Removed)
        )
    }

    /// Every thinker still linked (removed ones excluded), head to tail —
    /// the original's `for (th = thinkercap.next; th != &thinkercap;
    /// th = th->next)` walk, used by the savegame archivers.
    pub fn iter_list(&self) -> impl Iterator<Item = (ThinkerId, ThinkFn, &ThinkerData)> {
        let mut current = self.slots[0].next;
        std::iter::from_fn(move || {
            while current != 0 {
                let slot = &self.slots[current as usize];
                let id = ThinkerId {
                    index: current,
                    generation: slot.generation,
                };
                current = slot.next;
                if !matches!(slot.function, ThinkFn::Removed) {
                    return Some((id, slot.function, slot.data.as_ref()?));
                }
            }
            None
        })
    }

    /// All live mobjs, in list order (oldest-added first) — for callers
    /// (the renderer, `P_RespawnSpecials`'s type scan, ...) that need to
    /// walk every mobj rather than one sector/blockmap cell's chain.
    pub fn iter_mobjs(&self) -> impl Iterator<Item = (ThinkerId, &Mobj)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            if matches!(slot.function, ThinkFn::Removed) {
                return None;
            }
            let mobj = slot.data.as_ref()?.as_mobj()?;
            Some((
                ThinkerId {
                    index: i as u32,
                    generation: slot.generation,
                },
                mobj,
            ))
        })
    }

    /// Port of `P_RunThinkers`: walks the list head to tail, freeing any
    /// slot tagged [`ThinkFn::Removed`] as it passes (unlinking it first,
    /// like the original), and calling each live mobj's `P_MobjThinker`
    /// otherwise. `think_mobj` is called with this arena and the
    /// thinker's id so it can look up/mutate the mobj and reach other
    /// mobjs via [`ThinkerId`] (`target`, ...); it returns `false` if the
    /// mobj removed itself, matching `P_SetMobjState`'s `boolean` return
    /// used the same way in the original.
    pub fn run_thinkers(&mut self, mut think_mobj: impl FnMut(&mut Thinkers, ThinkerId)) {
        self.run_thinkers_dispatch(|thinkers, id, function| {
            if function == ThinkFn::MobjThinker {
                think_mobj(thinkers, id);
            }
        });
    }

    /// Like [`Thinkers::run_thinkers`], but the callback also receives
    /// the slot's [`ThinkFn`] so a single pass can dispatch every
    /// thinker kind (mobjs and the `p_spec.c` movers/lights alike) —
    /// standing in for the original's function-pointer dispatch inside
    /// `P_RunThinkers` (`currentthinker->function.acp1(currentthinker)`),
    /// which doesn't care what kind of thinker it's calling either.
    pub fn run_thinkers_dispatch(
        &mut self,
        mut think: impl FnMut(&mut Thinkers, ThinkerId, ThinkFn),
    ) {
        let mut current = self.slots[0].next;
        while current != 0 {
            let next = self.slots[current as usize].next;
            let function = self.slots[current as usize].function;
            if matches!(function, ThinkFn::Removed) {
                let prev = self.slots[current as usize].prev;
                self.slots[next as usize].prev = prev;
                self.slots[prev as usize].next = next;
                self.slots[current as usize].data = None;
                self.free.push(current);
            } else if function != ThinkFn::Null {
                let id = ThinkerId {
                    index: current,
                    generation: self.slots[current as usize].generation,
                };
                think(self, id, function);
            }
            current = next;
        }
    }
}

/// Port of `P_Ticker`. Run the tic (the original's own comment,
/// preserved).
///
/// `players`/`playeringame` stand in for the original's global
/// `player_t players[MAXPLAYERS]`/`playeringame[MAXPLAYERS]` — index
/// `i` in each is player `i`, same as the original's array indexing.
/// `menuactive`/`paused`/`netgame`/`demoplayback` come from
/// [`crate::doomstat::GameState`] (already ported); callers pass them
/// through explicitly since this function doesn't reach into
/// `doomstat` itself (keeping its dependency surface to exactly what
/// the original reads).
///
/// `specials`/`rdata`/`switches` are [`crate::p_spec::SpecialsState`]/
/// [`crate::r_data::RData`]/[`crate::p_switch::SwitchState`] — needed
/// since Phase 7b4 for [`crate::p_spec::SpecialsState::p_update_specials`]
/// (`P_UpdateSpecials`). `P_RespawnSpecials` (deathmatch item respawn)
/// still isn't ported — it needs monster/AI infrastructure (Phase
/// 7c/7d), not map specials.
#[allow(clippy::too_many_arguments)]
pub fn p_ticker(
    thinkers: &mut Thinkers,
    level: &mut crate::p_setup::Level,
    rmain: &mut crate::r_main::RMain,
    validcount: i32,
    players: &mut [crate::d_player::Player],
    playeringame: &[bool],
    paused: bool,
    menuactive: bool,
    netgame: bool,
    demoplayback: bool,
    specials: &mut crate::p_spec::SpecialsState,
    rdata: &mut crate::r_data::RData,
    wad: &crate::w_wad::WadFiles,
    switches: &mut crate::p_switch::SwitchState,
    active_plats: &mut crate::p_plats::ActivePlats,
    active_ceilings: &mut crate::p_ceilng::ActiveCeilings,
    brain: &mut crate::p_enemy::BrainTargets,
) {
    if paused {
        return;
    }

    // pause if in menu and at least one tic has been run (the
    // original's own comment, preserved). `players[consoleplayer].viewz
    // != 1` is the original's own way of asking "has at least one tic
    // run yet" (`viewz` starts at the sentinel value `1`, set for real
    // once `P_PlayerThink`/`P_CalcHeight` run) — reproduced the same
    // way rather than adding a separate "has ticked" flag.
    if !netgame && menuactive && !demoplayback && players.first().is_some_and(|p| p.viewz != 1) {
        return;
    }

    {
        let mut ctx = crate::p_spec::SpecialsCtx {
            thinkers,
            level,
            players,
            rmain,
            rdata,
            wad,
            active_plats,
            active_ceilings,
            switches,
        };
        for i in 0..ctx.players.len() {
            if playeringame.get(i).copied().unwrap_or(false) {
                crate::p_user::p_player_think(&mut ctx, validcount, i);
            }
        }
    }

    // P_RunThinkers: each thinker dispatched by its function, standing
    // in for `currentthinker->function.acp1(currentthinker)`. Mobjs go
    // to P_MobjThinker (`players` is passed through whole — not just the
    // mobj's own owner — since Phase 7b, see `p_mobj::p_mobj_thinker`'s
    // docs on why: P_TryMove's PIT_CheckThing can damage any player a
    // moving mobj collides with); the `p_spec.c` movers and lights go to
    // their `T_*` functions. (Until this dispatch existed only mobjs
    // ever ticked in the live game: doors, lifts, floors, ceilings and
    // lights never moved.)
    let leveltime = crate::doomstat::state().leveltime;
    thinkers.run_thinkers_dispatch(|thinkers, id, function| match function {
        ThinkFn::MobjThinker => {
            crate::p_mobj::p_mobj_thinker(
                thinkers,
                level,
                players,
                rmain,
                validcount,
                playeringame,
                rdata,
                brain,
                id,
            );
        }
        ThinkFn::FireFlicker => crate::p_lights::t_fire_flicker(thinkers, level, id),
        ThinkFn::LightFlash => crate::p_lights::t_light_flash(thinkers, level, id),
        ThinkFn::StrobeFlash => crate::p_lights::t_strobe_flash(thinkers, level, id),
        ThinkFn::Glow => crate::p_lights::t_glow(thinkers, level, id),
        ThinkFn::FloorMove => {
            crate::p_floor::t_move_floor(thinkers, level, players, rmain, validcount, leveltime, id)
        }
        ThinkFn::PlatRaise => crate::p_plats::t_plat_raise(
            thinkers,
            level,
            players,
            rmain,
            active_plats,
            validcount,
            leveltime,
            id,
        ),
        ThinkFn::VerticalDoor => crate::p_doors::t_vertical_door(
            thinkers, level, players, rmain, validcount, leveltime, id,
        ),
        ThinkFn::MoveCeiling => crate::p_ceilng::t_move_ceiling(
            thinkers,
            level,
            players,
            rmain,
            active_ceilings,
            validcount,
            leveltime,
            id,
        ),
        ThinkFn::Null | ThinkFn::Removed => {}
    });

    specials.p_update_specials(level, rdata, switches, leveltime);

    // P_RespawnSpecials: deathmatch-2 item respawn.
    crate::p_mobj::p_respawn_specials(thinkers, level);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r_defs::Mobj;

    fn blank_mobj() -> Mobj {
        // Minimal zeroed mobj for list-mechanics tests — field values
        // don't matter here, only slot identity/lifetime do.
        Mobj::blank(crate::info::MobjType::MtPlayer)
    }

    #[test]
    fn add_thinker_links_in_insertion_order() {
        let mut t = Thinkers::new();
        let a = t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(blank_mobj()));
        let b = t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(blank_mobj()));
        let mut order = Vec::new();
        t.run_thinkers(|_, id| order.push(id));
        assert_eq!(order, vec![a, b]);
    }

    #[test]
    fn removed_thinker_is_skipped_and_freed_on_next_run() {
        let mut t = Thinkers::new();
        let a = t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(blank_mobj()));
        let b = t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(blank_mobj()));
        t.remove(a);

        let mut order = Vec::new();
        t.run_thinkers(|_, id| order.push(id));
        assert_eq!(order, vec![b], "removed thinker must not run");
        assert!(!t.is_live(a));
        assert!(
            t.mobj(a).is_none(),
            "stale id must read as gone, not alias b"
        );
    }

    #[test]
    fn freed_slot_is_reused_with_a_new_generation() {
        let mut t = Thinkers::new();
        let a = t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(blank_mobj()));
        t.remove(a);
        t.run_thinkers(|_, _| {});

        let c = t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(blank_mobj()));
        assert_eq!(c.index, a.index, "the freed slot should be reused");
        assert_ne!(c.generation, a.generation);
        assert!(
            t.mobj(a).is_none(),
            "old id must not alias the new occupant"
        );
        assert!(t.mobj(c).is_some());
    }

    #[test]
    fn self_removal_mid_run_still_visits_later_thinkers() {
        let mut t = Thinkers::new();
        let a = t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(blank_mobj()));
        let b = t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(blank_mobj()));
        let mut order = Vec::new();
        t.run_thinkers(|thinkers, id| {
            order.push(id);
            if id == a {
                thinkers.remove(a);
            }
        });
        assert_eq!(order, vec![a, b]);
    }
}
