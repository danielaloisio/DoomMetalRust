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
//      Refresh/rendering module, shared data struct definitions.
//
//-----------------------------------------------------------------------------

//! Rust port of `r_defs.h`. Refresh/rendering module, shared data
//! struct definitions.
//!
//! # Representation: indices instead of raw pointers
//!
//! The original's map data structs form a mutually-referential graph via
//! raw pointers (`line_t.frontsector: sector_t*`, `seg_t.sidedef:
//! side_t*`, `sector_t.lines: line_t**`, etc.) into shared, globally
//! owned arrays (`sectors[]`, `lines[]`, `sides[]`, ... — populated by
//! `p_setup.c`, ported in Phase 5b). This port replaces every such
//! pointer with a plain index (`usize`/`Option<usize>`) into the
//! corresponding `Vec`, following the same pattern already used for
//! `w_wad`'s zone handles and the plan's general "index instead of raw
//! pointer into a shared array" approach. Resolving an index back to
//! data means indexing into the owning `Vec` (typically held by a
//! not-yet-ported `p_setup`-owned level-data struct in Phase 5b) — the
//! same implicit-global-array access the C code already relied on, just
//! made explicit at each call site instead of hidden behind a pointer.
//!
//! `-1`/`NULL` in the original (e.g. `sidenum[1] == -1` for a one-sided
//! line, `backsector == NULL`) becomes `None`.
//!
//! # `mobj_t`/thinker references use `ThinkerId`
//!
//! `sector_t.thinglist`/`soundtarget`/`specialdata`, `line_t.specialdata`
//! and `mobj_t.target`/`tracer` all pointed at a `thinker_t`/`mobj_t` in
//! the original. Since Phase 6b (`p_tick`'s mobj/thinker arena), those
//! become [`crate::p_tick::ThinkerId`] — see that module's docs for why
//! a generational index replaces the raw pointer. `mobj_t.type`/`info`/
//! `state`/`sprite` are now the real `info.rs` enum/index types (Phase
//! 6a), not placeholders.

use crate::doomdata::MapThing;
use crate::info::{MobjInfo, MobjType, SpriteNum, StateNum};
use crate::m_fixed::Fixed;
use crate::p_tick::ThinkerId;
use crate::tables::Angle;

// Silhouette, needed for clipping Segs (mainly) and sprites representing
// things.
pub const SIL_NONE: i32 = 0;
pub const SIL_BOTTOM: i32 = 1;
pub const SIL_TOP: i32 = 2;
pub const SIL_BOTH: i32 = 3;

pub const MAXDRAWSEGS: usize = 256;

// ---------------------------------------------------------------------
// INTERNAL MAP TYPES used by play and refresh
// ---------------------------------------------------------------------

/// Your plain vanilla vertex (`vertex_t`). Note: transformed values not
/// buffered locally, like some DOOM-alikes ("wt", "WebView") did (the
/// original's own comment, preserved).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Vertex {
    pub x: Fixed,
    pub y: Fixed,
}

/// Each sector has a `degenmobj_t` in its center for sound origin
/// purposes (`degenmobj_t`). "I suppose this does not handle sound from
/// moving objects (doppler), because position is prolly just buffered,
/// not updated." (the original's own comment, preserved).
///
/// The original's `thinker: thinker_t` field is explicitly noted as
/// "not used for anything" by the original itself, so it's not ported —
/// [`Thinker`][crate::d_think::Thinker] also isn't `Copy`/`Clone`/`Debug`
/// (it holds raw linked-list pointers, appropriately so for a type
/// that's actually threaded into a live list — which this dead field
/// never is), so including it here would need those derives dropped
/// crate-wide or a manual impl for a field that does nothing. Dropping
/// the field entirely is the simpler, equally faithful choice.
#[derive(Debug, Clone, Copy, Default)]
pub struct DegenMobj {
    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,
}

/// The SECTORS record, at runtime (`sector_t`). Stores things/mobjs.
///
/// `thinglist`/`soundtarget`/`specialdata` reference `mobj_t`/
/// `thinker_t` via [`ThinkerId`] — see module docs.
#[derive(Debug, Clone, Default)]
pub struct Sector {
    pub floorheight: Fixed,
    pub ceilingheight: Fixed,
    /// Flat (floor texture) index — resolved by `r_data` (Phase 5c),
    /// `R_FlatNumForName`.
    pub floorpic: i16,
    pub ceilingpic: i16,
    pub lightlevel: i16,
    pub special: i16,
    pub tag: i16,

    /// 0 = untraversed, 1,2 = sndlines -1
    pub soundtraversed: i32,

    /// Thing that made a sound (or `None`).
    pub soundtarget: Option<ThinkerId>,

    /// Mapblock bounding box for height changes.
    pub blockbox: [i32; 4],

    /// Origin for any sounds played by the sector.
    pub soundorg: DegenMobj,

    /// If == validcount, already checked.
    pub validcount: i32,

    /// List of mobjs in sector (head of the `snext`/`sprev` chain
    /// threaded through [`Mobj::snext`]/[`Mobj::sprev`]).
    pub thinglist: Option<ThinkerId>,

    /// Thinker_t for reversible actions (e.g. a moving floor/door/
    /// platform's thinker). Placeholder: `p_*` special thinkers are
    /// later-phase work (Phase 7); `None` for now.
    pub specialdata: Option<ThinkerId>,

    /// (`lines: struct line_s**`) — indices into the level's `lines`
    /// `Vec` (owned by `p_setup`, Phase 5b), replacing the original's
    /// pointer-into-shared-buffer.
    pub lines: Vec<usize>,
}

/// The SideDef (`side_t`).
#[derive(Debug, Clone, Copy)]
pub struct Side {
    /// Add this to the calculated texture column.
    pub textureoffset: Fixed,
    /// Add this to the calculated texture top.
    pub rowoffset: Fixed,

    // Texture indices. We do not maintain names here (the original's own
    // comment, preserved) — resolved by r_data (Phase 5c),
    // R_TextureNumForName.
    pub toptexture: i16,
    pub bottomtexture: i16,
    pub midtexture: i16,

    /// Sector the SideDef is facing. Index into the level's `sectors`
    /// `Vec`.
    pub sector: usize,
}

/// Move clipping aid for LineDefs (`slopetype_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlopeType {
    Horizontal,
    Vertical,
    Positive,
    Negative,
}

/// (`line_t`)
#[derive(Debug, Clone, Copy)]
pub struct Line {
    /// Vertices, from v1 to v2. Indices into the level's `vertexes`
    /// `Vec`.
    pub v1: usize,
    pub v2: usize,

    /// Precalculated v2 - v1 for side checking.
    pub dx: Fixed,
    pub dy: Fixed,

    // Animation related.
    pub flags: i16,
    pub special: i16,
    pub tag: i16,

    /// Visual appearance: SideDefs. `sidenum[1]` is `None` if one sided.
    /// Indices into the level's `sides` `Vec`.
    pub sidenum: [Option<usize>; 2],

    /// Neat. Another bounding box, for the extent of the LineDef.
    pub bbox: [Fixed; 4],

    /// To aid move clipping.
    pub slopetype: SlopeType,

    /// Front and back sector. Note: redundant? Can be retrieved from
    /// SideDefs (the original's own comment, preserved). Indices into
    /// the level's `sectors` `Vec`; `backsector` is `None` for one-sided
    /// lines.
    pub frontsector: Option<usize>,
    pub backsector: Option<usize>,

    /// If == validcount, already checked.
    pub validcount: i32,

    /// Thinker_t for reversible actions. Placeholder: `p_*` special
    /// thinkers are later-phase work (Phase 7); `None` for now.
    pub specialdata: Option<ThinkerId>,
}

impl Default for Line {
    fn default() -> Self {
        Line {
            v1: 0,
            v2: 0,
            dx: 0,
            dy: 0,
            flags: 0,
            special: 0,
            tag: 0,
            sidenum: [None, None],
            bbox: [0; 4],
            slopetype: SlopeType::Horizontal,
            frontsector: None,
            backsector: None,
            validcount: 0,
            specialdata: None,
        }
    }
}

/// A SubSector (`subsector_t`). References a Sector. Basically, this is
/// a list of LineSegs, indicating the visible walls that define (all or
/// some) sides of a convex BSP leaf.
#[derive(Debug, Clone, Copy, Default)]
pub struct Subsector {
    /// Index into the level's `sectors` `Vec`. `usize::MAX` sentinel
    /// (via `Default`) until `p_setup` (Phase 5b) fills it in — mirrors
    /// the original's `memset(subsectors, 0, ...)` leaving `sector`
    /// NULL until `P_GroupLines` sets it.
    pub sector: usize,
    pub numlines: i16,
    pub firstline: i16,
}

/// The LineSeg (`seg_t`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Seg {
    /// Indices into the level's `vertexes` `Vec`.
    pub v1: usize,
    pub v2: usize,

    pub offset: Fixed,
    pub angle: Angle,

    /// Index into the level's `sides` `Vec`.
    pub sidedef: usize,
    /// Index into the level's `lines` `Vec`.
    pub linedef: usize,

    /// Sector references. Could be retrieved from linedef, too (the
    /// original's own comment, preserved). `backsector` is `None` for
    /// one-sided lines. Indices into the level's `sectors` `Vec`.
    pub frontsector: usize,
    pub backsector: Option<usize>,
}

/// BSP node (`node_t`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Node {
    // Partition line.
    pub x: Fixed,
    pub y: Fixed,
    pub dx: Fixed,
    pub dy: Fixed,

    /// Bounding box for each child.
    pub bbox: [[Fixed; 4]; 2],

    /// If `NF_SUBSECTOR` its a subsector (see [`crate::doomdata::NF_SUBSECTOR`]).
    pub children: [u16; 2],
}

/// Posts are runs of non-masked source pixels (`post_t`).
#[derive(Debug, Clone, Copy)]
pub struct Post {
    /// -1 (`0xff`) is the last post in a column.
    pub topdelta: u8,
    /// Length data bytes follows.
    pub length: u8,
}

/// (`column_t`) — a list of 0 or more [`Post`]s, `0xff`-terminated. The
/// original spells this as `typedef post_t column_t` (i.e. `column_t`
/// and `post_t` are the same type; a column is really just its first
/// post, chained via `length`-based offsets through raw memory). Kept as
/// a type alias for the same reason.
pub type Column = Post;

/// This could be wider for >8 bit display. Indeed, true color support is
/// possible precalculating 24bpp lightmap/colormap LUT from darkening
/// PLAYPAL to all black. Could even use more than 32 levels (the
/// original's own comment, preserved).
pub type LightTable = crate::doomtype::Byte;

/// Where a [`DrawSeg`]'s `sprtopclip`/`sprbottomclip` points to. In the
/// original, a `short*` that is either one of `r_things.c`'s two
/// constant arrays (`screenheightarray`, every entry `viewheight`;
/// `negonearray`, every entry `-1`) or a slice of `r_plane.c`'s shared
/// `openings[]` scratch buffer, offset so that `[x]` is the value for
/// screen column `x`. Kept as an explicit enum rather than copying the
/// constant arrays into `openings` too, so the "points at a constant
/// array" case stays visible, same as in the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpriteClip {
    /// (`screenheightarray`) — `viewheight` at every column.
    ScreenHeight,
    /// (`negonearray`) — `-1` at every column.
    NegOne,
    /// Slice of [`crate::r_plane::RPlane::openings`]; the value is the
    /// index of the owning drawseg's `x1` column (the original's
    /// `lastopening - start`, stored un-offset since a `usize` index
    /// can't hold the negative bias).
    Openings(usize),
}

impl SpriteClip {
    /// The clip value at screen column `x` (the original's
    /// `ds->sprtopclip[x]`). `ds_x1` is the owning drawseg's `x1`.
    pub fn at(self, x: i32, ds_x1: i32, openings: &[i32], viewheight: i32) -> i32 {
        match self {
            SpriteClip::ScreenHeight => viewheight,
            SpriteClip::NegOne => -1,
            SpriteClip::Openings(base) => openings[base + (x - ds_x1) as usize],
        }
    }
}

/// (`drawseg_t`)
///
/// `sprtopclip`/`sprbottomclip` are [`SpriteClip`]s (see its docs);
/// `maskedtexturecol` is the index into
/// [`crate::r_plane::RPlane::openings`] of the `x1` column's entry,
/// same un-offset convention as [`SpriteClip::Openings`]. `None` stands
/// in for `NULL` in all three.
#[derive(Debug, Clone, Copy)]
pub struct DrawSeg {
    /// Index into the level's `segs` `Vec`.
    pub curline: usize,
    pub x1: i32,
    pub x2: i32,

    pub scale1: Fixed,
    pub scale2: Fixed,
    pub scalestep: Fixed,

    /// 0=none, 1=bottom, 2=top, 3=both — see `SIL_*` constants.
    pub silhouette: i32,

    /// Do not clip sprites above this.
    pub bsilheight: Fixed,
    /// Do not clip sprites below this.
    pub tsilheight: Fixed,

    /// Pointers to lists for sprite clipping, all three adjusted so
    /// `[x1]` is first value (the original's own comment, preserved) —
    /// see struct docs on the placeholder representation.
    pub sprtopclip: Option<SpriteClip>,
    pub sprbottomclip: Option<SpriteClip>,
    pub maskedtexturecol: Option<usize>,
}

/// Patches. A patch holds one or more columns. Patches are used for
/// sprites and all masked pictures, and we compose textures from the
/// TEXTURE1/2 lists of patches (`patch_t`).
#[derive(Debug, Clone)]
pub struct Patch {
    /// Bounding box size.
    pub width: i16,
    pub height: i16,
    /// Pixels to the left of origin.
    pub leftoffset: i16,
    /// Pixels below the origin.
    pub topoffset: i16,
    /// (`columnofs[8]`) — only `[width]` used in the original (the fixed
    /// 8-element array is an over-allocation quirk of the original
    /// struct; real patches can be wider than 8 columns, and the
    /// original relies on reading past the declared array bound into
    /// the rest of the lump's raw bytes). Ported as an owned `Vec<i32>`
    /// sized to the patch's actual width instead, sidestepping that
    /// out-of-bounds-by-design read — see `r_data`(Phase 5c)'s patch
    /// loader for where this gets populated from raw lump bytes.
    pub columnofs: Vec<i32>,
}

/// A `vissprite_t` is a thing that will be drawn during a refresh
/// (`vissprite_t`). I.e. a sprite object that is partly visible.
///
/// The original's doubly linked list (`prev`/`next`) threading live
/// vissprites together is not reproduced structurally — Phase 5e (which
/// actually populates/traverses these) is expected to hold them in a
/// `Vec`/similar instead, per the plan's general preference for owned
/// collections over hand-rolled linked lists once Rust makes that free.
#[derive(Debug, Clone, Copy)]
pub struct VisSprite {
    pub x1: i32,
    pub x2: i32,

    /// For line side calculation.
    pub gx: Fixed,
    pub gy: Fixed,

    /// Global bottom / top for silhouette clipping.
    pub gz: Fixed,
    pub gzt: Fixed,

    /// Horizontal position of x1.
    pub startfrac: Fixed,

    pub scale: Fixed,

    /// Negative if flipped.
    pub xiscale: Fixed,

    pub texturemid: Fixed,
    pub patch: i32,

    /// For color translation and shadow draw, maxbright frames as well.
    /// Index into a colormap table (Phase 5c/5e).
    pub colormap: Option<usize>,

    pub mobjflags: i32,
}

/// Sprites are patches with a special naming convention so they can be
/// recognized by `R_InitSprites` (`spriteframe_t`). The base name is
/// NNNNFx or NNNNFxFx, with x indicating the rotation, x = 0, 1-7. The
/// sprite and frame specified by a `thing_t` is range checked at run
/// time. A sprite is a `patch_t` that is assumed to represent a
/// three-dimensional object and may have multiple rotations pre drawn.
/// Horizontal flipping is used to save space, thus NNNNF2F5 defines a
/// mirrored patch. Some sprites will only have one picture used for all
/// views: NNNNF0 (the original's own comment, preserved).
#[derive(Debug, Clone, Copy)]
pub struct SpriteFrame {
    /// If false use 0 for any position. Note: as eight entries are
    /// available, we might as well insert the same name eight times
    /// (the original's own comment, preserved).
    pub rotate: bool,

    /// Lump to use for view angles 0-7.
    pub lump: [i16; 8],

    /// Flip bit (1 = flip) to use for view angles 0-7.
    pub flip: [u8; 8],
}

/// A sprite definition: a number of animation frames (`spritedef_t`).
#[derive(Debug, Clone, Default)]
pub struct SpriteDef {
    pub spriteframes: Vec<SpriteFrame>,
}

/// Now what is a visplane, anyway? (the original's own comment,
/// preserved) (`visplane_t`)
///
/// The original pads `top`/`bottom` with `pad1`..`pad4` bytes to leave
/// room for `[minx-1]`/`[maxx+1]` out-of-bounds writes in the
/// rasterizer (a real, intentional overrun the C code relies on).
/// Ported as `Vec<u8>` sized `SCREENWIDTH + 2`, with every original
/// `top[x]`/`bottom[x]` access shifted to `top[x + 1]` — i.e. index 0
/// stands in for the original's `[-1]` slot, and `SCREENWIDTH + 1`
/// stands in for `[SCREENWIDTH]` (`[maxx+1]` when `maxx ==
/// SCREENWIDTH-1`). [`VisPlane::top`]/[`VisPlane::bottom`] are private
/// with [`VisPlane::top_at`]/[`VisPlane::set_top_at`] (and the `bottom`
/// equivalents) doing the `+1` translation, so callers use the
/// original's own `[-1..=SCREENWIDTH]` index range directly instead of
/// re-deriving the offset at every call site.
#[derive(Debug, Clone)]
pub struct VisPlane {
    pub height: Fixed,
    pub picnum: i32,
    pub lightlevel: i32,
    pub minx: i32,
    pub maxx: i32,

    top: Vec<u8>,
    bottom: Vec<u8>,
}

impl VisPlane {
    pub fn new(screenwidth: usize) -> Self {
        VisPlane {
            height: 0,
            picnum: 0,
            lightlevel: 0,
            minx: 0,
            maxx: 0,
            top: vec![0; screenwidth + 2],
            bottom: vec![0; screenwidth + 2],
        }
    }

    /// `top[x]` for `x` in `-1..=screenwidth` (the original's own valid
    /// range, see struct docs on the `+1` offset).
    pub fn top_at(&self, x: i32) -> u8 {
        self.top[(x + 1) as usize]
    }

    pub fn set_top_at(&mut self, x: i32, value: u8) {
        self.top[(x + 1) as usize] = value;
    }

    pub fn bottom_at(&self, x: i32) -> u8 {
        self.bottom[(x + 1) as usize]
    }

    pub fn set_bottom_at(&mut self, x: i32, value: u8) {
        self.bottom[(x + 1) as usize] = value;
    }

    /// Resets every `top[x]` (`-1..=screenwidth`) to
    /// [`crate::r_plane::PLANE_UNSET`], matching the original's
    /// `memset(check->top, 0xff, sizeof(check->top))` (which, being a
    /// whole-array memset including the padding bytes, also touches the
    /// `[-1]`/`[screenwidth]` slots — reproduced here by resetting the
    /// entire backing `Vec`, not just the `0..screenwidth` range).
    pub fn reset_top(&mut self) {
        self.top.iter_mut().for_each(|t| *t = 0xff);
    }
}

// ---------------------------------------------------------------------
// mobj_t (p_mobj.h) — ported here alongside sector_t/line_t since they
// reference it; see module docs on the info.h placeholder fields.
// ---------------------------------------------------------------------

/// Misc. mobj flags (`mobjflag_t`). Kept as plain `i32` bit constants
/// (matching [`crate::d_event`]'s precedent for C bitflag enums) rather
/// than a Rust `enum`/`bitflags` type — `mobjflag_t` packs unrelated
/// flag groups (including `MF_TRANSLATION`'s 2-bit sub-field at
/// `MF_TRANSSHIFT`) into one 32-bit space with plain OR/AND masking
/// throughout the original, which a typed flags wrapper would fight
/// rather than help with with at this porting stage.
pub mod mobj_flag {
    /// Call P_SpecialThing when touched.
    pub const SPECIAL: i32 = 1;
    /// Blocks.
    pub const SOLID: i32 = 2;
    /// Can be hit.
    pub const SHOOTABLE: i32 = 4;
    /// Don't use the sector links (invisible but touchable).
    pub const NOSECTOR: i32 = 8;
    /// Don't use the blocklinks (inert but displayable).
    pub const NOBLOCKMAP: i32 = 16;
    /// Not to be activated by sound, deaf monster.
    pub const AMBUSH: i32 = 32;
    /// Will try to attack right back.
    pub const JUSTHIT: i32 = 64;
    /// Will take at least one step before attacking.
    pub const JUSTATTACKED: i32 = 128;
    /// On level spawning (initial position), hang from ceiling instead
    /// of stand on floor.
    pub const SPAWNCEILING: i32 = 256;
    /// Don't apply gravity (every tic), that is, object will float,
    /// keeping current height or changing it actively.
    pub const NOGRAVITY: i32 = 512;
    /// This allows jumps from high places.
    pub const DROPOFF: i32 = 0x400;
    /// For players, will pick up items.
    pub const PICKUP: i32 = 0x800;
    /// Player cheat. ???
    pub const NOCLIP: i32 = 0x1000;
    /// Player: keep info about sliding along walls.
    pub const SLIDE: i32 = 0x2000;
    /// Allow moves to any height, no gravity. For active floaters, e.g.
    /// cacodemons, pain elementals.
    pub const FLOAT: i32 = 0x4000;
    /// Don't cross lines ??? or look at heights on teleport.
    pub const TELEPORT: i32 = 0x8000;
    /// Don't hit same species, explode on block. Player missiles as
    /// well as fireballs of various kinds.
    pub const MISSILE: i32 = 0x10000;
    /// Dropped by a demon, not level spawned. E.g. ammo clips dropped by
    /// dying former humans.
    pub const DROPPED: i32 = 0x20000;
    /// Use fuzzy draw (shadow demons or spectres), temporary player
    /// invisibility powerup.
    pub const SHADOW: i32 = 0x40000;
    /// Flag: don't bleed when shot (use puff), barrels and shootable
    /// furniture shall not bleed.
    pub const NOBLOOD: i32 = 0x80000;
    /// Don't stop moving halfway off a step, that is, have dead bodies
    /// slide down all the way.
    pub const CORPSE: i32 = 0x100000;
    /// Floating to a height for a move, ??? don't auto float to
    /// target's height.
    pub const INFLOAT: i32 = 0x200000;
    /// On kill, count this enemy object towards intermission kill
    /// total. Happy gathering.
    pub const COUNTKILL: i32 = 0x400000;
    /// On picking up, count this item object towards intermission item
    /// total.
    pub const COUNTITEM: i32 = 0x800000;
    /// Special handling: skull in flight. Neither a cacodemon nor a
    /// missile.
    pub const SKULLFLY: i32 = 0x1000000;
    /// Don't spawn this object in death match mode (e.g. key cards).
    pub const NOTDMATCH: i32 = 0x2000000;
    /// Player sprites in multiplayer modes are modified using an
    /// internal color lookup table for re-indexing. If 0x4 0x8 or 0xc,
    /// use a translation table for player colormaps.
    pub const TRANSLATION: i32 = 0xc000000;
    /// Hmm ???.
    pub const TRANSSHIFT: i32 = 26;
}

/// Map Object definition (`mobj_t`).
///
/// `type`/`info`/`state`/`sprite` are the real `info.rs` types as of
/// Phase 6b (`info: &'static MobjInfo` for the original's
/// `&mobjinfo[mobj->type]`, resolved once at spawn and kept alongside
/// `mobj_type` exactly like the original keeps both `type` and `info`).
///
/// `snext`/`sprev` (sector thing-list links) and `bnext`/`bprev`
/// (blockmap links) are, in the original, raw pointers threading a
/// doubly linked list through the mobj pool. Here they're
/// [`ThinkerId`]s into the same `p_tick::Thinkers` arena that owns this
/// `Mobj` itself — `P_SetThingPosition`/`P_UnsetThingPosition`
/// (`p_mobj.rs`) thread/unthread them exactly as the original does, just
/// with generational indices instead of pointers.
#[derive(Debug, Clone, Copy)]
pub struct Mobj {
    // Info for drawing: position.
    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,

    /// Links in sector's `thinglist` (head-inserted, newest first, like
    /// the original).
    pub snext: Option<ThinkerId>,
    pub sprev: Option<ThinkerId>,

    // More drawing info: to determine current sprite.
    /// Orientation.
    pub angle: Angle,
    /// Used to find patch_t and flip value.
    pub sprite: SpriteNum,
    /// Might be ORed with FF_FULLBRIGHT.
    pub frame: i32,

    /// Links in the blockmap cell's chain (head-inserted, like the
    /// original).
    pub bnext: Option<ThinkerId>,
    pub bprev: Option<ThinkerId>,

    /// Index into the level's `subsectors` `Vec` (or `None` before first
    /// placement).
    pub subsector: Option<usize>,

    /// The closest interval over all contacted Sectors.
    pub floorz: Fixed,
    pub ceilingz: Fixed,

    /// For movement checking.
    pub radius: Fixed,
    pub height: Fixed,

    /// Momentums, used to update position.
    pub momx: Fixed,
    pub momy: Fixed,
    pub momz: Fixed,

    /// If == validcount, already checked.
    pub validcount: i32,

    pub mobj_type: MobjType,
    /// `&mobjinfo[mobj->type]` in the original.
    pub info: &'static MobjInfo,

    /// State tic counter.
    pub tics: i32,
    pub state: StateNum,
    /// See [`mobj_flag`].
    pub flags: i32,
    pub health: i32,

    /// Movement direction, movement generation (zig-zagging). 0-7.
    pub movedir: i32,
    /// When 0, select a new dir.
    pub movecount: i32,

    /// Thing being chased/attacked (or `None`), also the originator for
    /// missiles.
    pub target: Option<ThinkerId>,

    /// Reaction time: if non 0, don't attack yet. Used by player to
    /// freeze a bit after teleporting.
    pub reactiontime: i32,

    /// If >0, the target will be chased no matter what (even if shot).
    pub threshold: i32,

    /// Additional info record for player avatars only. Only valid if
    /// `type == MT_PLAYER`. Placeholder: index of the owning player
    /// (0..MAXPLAYERS), resolved against
    /// [`crate::doomstat::GameState`] once `players` is ported
    /// (currently excluded there too — see `doomstat`'s module docs).
    pub player: Option<usize>,

    /// Player number last looked for.
    pub lastlook: i32,

    /// For nightmare respawn.
    pub spawnpoint: MapThing,

    /// Thing being chased/attacked for tracers.
    pub tracer: Option<ThinkerId>,
}

impl Mobj {
    /// A fully zeroed `Mobj` of `mobj_type` — mirrors `memset(mobj, 0,
    /// sizeof(*mobj))` in `P_SpawnMobj` before its real fields are
    /// filled in. For tests/scenes that build a `Mobj` by hand rather
    /// than through [`crate::p_mobj::p_spawn_mobj`]; real spawning
    /// should go through that function instead, which also links the
    /// mobj into the world.
    pub fn blank(mobj_type: MobjType) -> Mobj {
        Mobj {
            x: 0,
            y: 0,
            z: 0,
            snext: None,
            sprev: None,
            angle: 0,
            sprite: SpriteNum::SprTroo,
            frame: 0,
            bnext: None,
            bprev: None,
            subsector: None,
            floorz: 0,
            ceilingz: 0,
            radius: 0,
            height: 0,
            momx: 0,
            momy: 0,
            momz: 0,
            validcount: 0,
            mobj_type,
            info: &crate::info::MOBJINFO[mobj_type as usize],
            tics: 0,
            state: StateNum::SNull,
            flags: 0,
            health: 0,
            movedir: 0,
            movecount: 0,
            target: None,
            reactiontime: 0,
            threshold: 0,
            player: None,
            lastlook: 0,
            spawnpoint: MapThing {
                x: 0,
                y: 0,
                angle: 0,
                thing_type: 0,
                options: 0,
            },
            tracer: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sil_constants_match_original() {
        assert_eq!(SIL_NONE, 0);
        assert_eq!(SIL_BOTTOM, 1);
        assert_eq!(SIL_TOP, 2);
        assert_eq!(SIL_BOTH, 3);
    }

    #[test]
    fn maxdrawsegs_matches_original() {
        assert_eq!(MAXDRAWSEGS, 256);
    }

    #[test]
    fn sector_default_has_no_lines_and_no_thinglist() {
        let s = Sector::default();
        assert!(s.lines.is_empty());
        assert_eq!(s.thinglist, None);
        assert_eq!(s.soundtarget, None);
    }

    #[test]
    fn line_default_one_sided_backsector_is_none() {
        let l = Line::default();
        assert_eq!(l.sidenum, [None, None]);
        assert_eq!(l.backsector, None);
        assert_eq!(l.slopetype, SlopeType::Horizontal);
    }

    #[test]
    fn visplane_new_sizes_top_bottom_to_screenwidth_plus_padding() {
        let vp = VisPlane::new(320);
        // -1..=320 is 322 valid indices via top_at/set_top_at.
        assert_eq!(vp.top_at(-1), 0);
        assert_eq!(vp.top_at(320), 0);
        assert_eq!(vp.bottom_at(-1), 0);
        assert_eq!(vp.bottom_at(320), 0);
    }

    #[test]
    fn visplane_reset_top_sets_full_range_to_unset() {
        let mut vp = VisPlane::new(320);
        vp.reset_top();
        assert_eq!(vp.top_at(-1), 0xff);
        assert_eq!(vp.top_at(0), 0xff);
        assert_eq!(vp.top_at(319), 0xff);
        assert_eq!(vp.top_at(320), 0xff);
    }

    #[test]
    fn mobj_flag_constants_match_original_defines() {
        assert_eq!(mobj_flag::SPECIAL, 1);
        assert_eq!(mobj_flag::SOLID, 2);
        assert_eq!(mobj_flag::SHOOTABLE, 4);
        assert_eq!(mobj_flag::NOSECTOR, 8);
        assert_eq!(mobj_flag::NOBLOCKMAP, 16);
        assert_eq!(mobj_flag::TELEPORT, 0x8000);
        assert_eq!(mobj_flag::TRANSLATION, 0xc000000);
        assert_eq!(mobj_flag::TRANSSHIFT, 26);
    }

    #[test]
    fn column_is_alias_for_post() {
        let c: Column = Post {
            topdelta: 0xff,
            length: 0,
        };
        assert_eq!(c.topdelta, 0xff);
    }
}
