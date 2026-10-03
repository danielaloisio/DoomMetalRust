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
//	Do all the WAD I/O, get map description,
//	set up initial state and misc. LUTs.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_setup.c` / `p_setup.h` (partial, see note
//! below). Do all the WAD I/O, get map description, set up initial
//! state and misc. LUTs.
//!
//! # Scope
//!
//! Ported: [`Level::load`], which reproduces `P_SetupLevel`'s map-data
//! loading sequence — `P_LoadBlockMap`, `P_LoadVertexes`,
//! `P_LoadSectors`, `P_LoadSideDefs`, `P_LoadLineDefs`,
//! `P_LoadSubsectors`, `P_LoadNodes`, `P_LoadSegs`, the reject matrix
//! cache, and `P_GroupLines`.
//!
//! Also ported (Phase 6b): [`p_load_things`]/[`p_spawn_map_thing`]
//! (`P_LoadThings`/`P_SpawnMapThing`), now that [`crate::p_mobj`] and
//! [`crate::p_tick::Thinkers`] give them a real mobj pool to spawn into.
//! Scoped to single-player (no netgame/deathmatch — those branches are
//! `unreachable!`, since `doomstat`'s defaults make them dead code
//! today and nothing yet sets `netgame`/`deathmatch`), and
//! [`p_spawn_map_thing`] stops short of the original's `P_SpawnPlayer`
//! call for a player start (`type <= 4`) — that needs `player_t`
//! (`doomstat`'s excluded `players` field) to link `mo`/`p->mo` both
//! ways, so it's left to whichever later phase ports `player_t`; for
//! now player starts are spawned as plain `MT_PLAYER` mobjs via
//! [`crate::p_mobj::p_spawn_mobj`], matching everything
//! `P_SpawnPlayer` does except the player-struct linkage.
//!
//! Not ported (deferred, with the original function/reason noted):
//! - `R_TextureNumForName`/`R_FlatNumForName` — texture/flat lookup;
//!   `r_data` is Phase 5c. `Side::toptexture`/`bottomtexture`/
//!   `midtexture` and `Sector::floorpic`/`ceilingpic` are populated with
//!   the raw not-yet-resolved lump name bytes as a placeholder sentinel
//!   (`-1`) for now — see [`Level::load`]'s body for exactly where.
//! - The rest of `P_SetupLevel` (score/player reset, `S_Start`,
//!   `Z_FreeTags`, `P_InitThinkers`, `W_Reload`, deathmatch spawn,
//!   `P_SpawnSpecials`, `R_PrecacheLevel`) all call into subsystems not
//!   yet ported (sound, thinkers, specials, precaching) or global state
//!   (`doomstat`'s `players`, excluded there for the same `mobj_t`
//!   reason) — left for the phases that port those subsystems.
//! - `P_Init` (`P_InitSwitchList`/`P_InitPicAnims`/`R_InitSprites`) —
//!   all three depend on data this phase doesn't have yet.
//!
//! # Zone allocation
//!
//! The original allocates each array with `Z_Malloc(..., PU_LEVEL, 0)`
//! (no owner — these arrays are switched out wholesale by
//! `Z_FreeTags(PU_LEVEL, ...)` on the next level load, never
//! individually reference-counted). This port just uses plain `Vec`s
//! owned by [`Level`] — a new `Level` naturally replaces the old one
//! (and its `Vec`s) when a new one loads, which is the actual behavior
//! `PU_LEVEL` was achieving; no `z_zone::Zone` involvement needed for
//! data that was never individually cached/shared beyond "this level's
//! lifetime" to begin with. Lump *loading* (via `W_CacheLumpNum`)
//! still goes through `w_wad`/`z_zone` as normal, exactly like the
//! original — it's still only the map array *storage* that skips it.

use crate::doomdata::{
    MapLinedef, MapNode, MapSector, MapSeg, MapSidedef, MapSubsector, MapThing, MapVertex,
};
use crate::doomdef::{GameMode, Skill, MTF_AMBUSH};
use crate::doomstat::{self, MAX_DM_STARTS};
use crate::info::{MobjType, MOBJINFO};
use crate::m_bbox::{m_add_to_box, m_clear_box, BBox, BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};
use crate::m_fixed::{fixed_div, FRACBITS};
use crate::p_tick::Thinkers;
use crate::r_defs::{mobj_flag, Line, Node, Sector, Seg, Side, SlopeType, Subsector, Vertex};
use crate::tables::ANG45;
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// (`MAPBLOCKUNITS`)
pub const MAPBLOCKUNITS: i32 = 128;
/// (`MAPBLOCKSHIFT`) = `FRACBITS+7`
pub const MAPBLOCKSHIFT: i32 = FRACBITS + 7;

/// MAXRADIUS is for precalculated sector block boxes. The spider demon
/// is larger, but we do not have any moving sectors nearby (the
/// original's own comment, preserved). (`MAXRADIUS`) = `32*FRACUNIT`
pub const MAXRADIUS: i32 = 32 * (1 << FRACBITS);

/// Sentinel for a not-yet-resolved texture/flat name, standing in for
/// `R_TextureNumForName`/`R_FlatNumForName` (Phase 5c) — see module
/// docs.
pub const TEXTURE_UNRESOLVED: i16 = -1;

/// A loaded map level: the `P_Load*`-populated arrays plus the blockmap/
/// reject data `P_SetupLevel` sets up alongside them.
///
/// Field names/shapes mirror the original's module-level globals
/// (`vertexes`, `numsectors`, `lines`, ...) as a struct instead of a set
/// of free-standing globals — see the plan's general globals strategy;
/// this one isn't behind a `GlobalCell`/`static` (unlike
/// `doomstat`/`r_state`) because nothing outside this phase owns/needs
/// a single shared instance yet, matching `w_wad::WadFiles`'s precedent
/// (also plain-owned, not global, for the same "no caller yet" reason).
#[derive(Debug, Default)]
pub struct Level {
    /// `itemrespawnque[]`/`itemrespawntime[]` (a ring in the original,
    /// `ITEMQUESIZE` long; the oldest entry is dropped when it fills):
    /// the spawn points of picked-up items and the `leveltime` they
    /// vanished at, for deathmatch-2 item respawn.
    pub itemrespawnque: std::collections::VecDeque<(MapThing, i32)>,
    pub vertexes: Vec<Vertex>,
    pub sectors: Vec<Sector>,
    pub sides: Vec<Side>,
    pub lines: Vec<Line>,
    pub subsectors: Vec<Subsector>,
    pub nodes: Vec<Node>,
    pub segs: Vec<Seg>,

    // BLOCKMAP. Created from axis aligned bounding box of the map, a
    // rectangular array of blocks of size ... Used to speed up
    // collision detection by spatial subdivision in 2D (the original's
    // own comment, preserved).
    pub bmapwidth: i32,
    /// Size in mapblocks.
    pub bmapheight: i32,
    /// (`blockmap`) — offsets in blockmap are from here (the original's
    /// own comment; `blockmap` is `blockmaplump + 4` in the original,
    /// skipping the 4-`short` header read by [`Level::load`] below).
    pub blockmap: Vec<i16>,
    /// (`blockmaplump`) — the full raw lump, header included. Kept
    /// alongside `blockmap` (rather than only storing the
    /// header-skipped slice) purely to mirror the original's two
    /// distinctly-named views into the same data; `blockmap`'s
    /// `Vec<i16>` doesn't alias `blockmaplump`'s the way the original's
    /// pointer-into-the-same-buffer did (Rust `Vec`s always own their
    /// data), but reproducing "two names for the same bytes, offset by
    /// one header" isn't worth reintroducing aliasing for.
    pub blockmaplump: Vec<i16>,
    /// Origin of block map.
    pub bmaporgx: i32,
    pub bmaporgy: i32,
    /// For thing chains — head of each blockmap cell's `bnext`/`bprev`
    /// chain (see `p_mobj`'s `P_SetThingPosition`/`P_UnsetThingPosition`).
    pub blocklinks: Vec<Option<crate::p_tick::ThinkerId>>,

    /// REJECT. For fast sight rejection. Speeds up enemy AI by skipping
    /// detailed Line of Sight calculation. Without special effect, this
    /// could be used as a PVS lookup as well (the original's own
    /// comment, preserved).
    pub rejectmatrix: Vec<u8>,

    /// Raw, not-yet-resolved texture/flat lump names read straight off
    /// the SIDEDEFS/SECTORS lumps — kept only long enough for
    /// [`Level::resolve_textures`] to turn them into real indices via
    /// `r_data`'s lookups (Phase 5c, already ported; this glue between
    /// it and `p_setup` was the missing piece). Not part of the
    /// original's `side_t`/`sector_t` (which never store the raw name
    /// once resolved) — kept private/parallel here rather than as
    /// fields on [`crate::r_defs::Side`]/[`crate::r_defs::Sector`]
    /// themselves, since those structs are used well beyond load time
    /// and shouldn't carry load-only scratch data.
    raw_names: RawTextureNames,
}

/// `(toptexture, bottomtexture, midtexture)` raw 8-byte lump names, one
/// per sidedef — see [`Level::raw_names`]'s docs.
type SideNames = (Byte8, Byte8, Byte8);
/// `(floorpic, ceilingpic)` raw 8-byte lump names, one per sector — see
/// [`Level::raw_names`]'s docs.
type SectorNames = (Byte8, Byte8);
type Byte8 = [u8; 8];

/// See [`Level::raw_names`]'s docs.
#[derive(Debug, Default)]
struct RawTextureNames {
    /// Parallel to `sides`.
    side_names: Vec<SideNames>,
    /// Parallel to `sectors`.
    sector_names: Vec<SectorNames>,
}

impl Level {
    /// Port of the map-loading portion of `P_SetupLevel` — see module
    /// docs for exactly what's included/excluded. `lumpname` is the map
    /// marker lump's name (e.g. `"E1M1"` or `"MAP01"`), already resolved
    /// by the caller — the original's episode/map-number-to-lumpname
    /// logic (`P_SetupLevel`'s `sprintf`/`ExMy` branch) isn't
    /// reproduced here since it depends on `doomstat.gamemode`, callable
    /// separately by whatever later-phase code drives this.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if the map lump isn't found,
    /// same as `W_GetNumForName` would.
    pub fn load(wad: &mut WadFiles, lumpname: &str) -> Level {
        let lumpnum = wad.get_num_for_name(lumpname);

        // note: most of this ordering is important (the original's own
        // comment, preserved) — P_LoadSideDefs/P_LoadLineDefs resolve
        // sector-by-index before P_GroupLines needs frontsector/
        // backsector populated, etc.
        let (blockmap, blockmaplump, bmapwidth, bmapheight, bmaporgx, bmaporgy, blocklinks) =
            load_blockmap(wad, lumpnum + MapLumpOffset::BLOCKMAP as usize);

        let vertexes = load_vertexes(wad, lumpnum + MapLumpOffset::VERTEXES as usize);
        let (mut sectors, sector_names) =
            load_sectors(wad, lumpnum + MapLumpOffset::SECTORS as usize);
        let (sides, side_names) =
            load_sidedefs(wad, lumpnum + MapLumpOffset::SIDEDEFS as usize, &sectors);
        let lines = load_linedefs(
            wad,
            lumpnum + MapLumpOffset::LINEDEFS as usize,
            &vertexes,
            &sides,
        );
        let mut subsectors = load_subsectors(wad, lumpnum + MapLumpOffset::SSECTORS as usize);
        let nodes = load_nodes(wad, lumpnum + MapLumpOffset::NODES as usize);
        let segs = load_segs(
            wad,
            lumpnum + MapLumpOffset::SEGS as usize,
            &vertexes,
            &lines,
            &sides,
        );

        let rejectmatrix = load_reject(wad, lumpnum + MapLumpOffset::REJECT as usize);

        group_lines(
            &mut sectors,
            &lines,
            &mut subsectors,
            &segs,
            &sides,
            &vertexes,
            bmapwidth,
            bmapheight,
            bmaporgx,
            bmaporgy,
        );

        Level {
            itemrespawnque: Default::default(),
            vertexes,
            sectors,
            sides,
            lines,
            subsectors,
            nodes,
            segs,
            bmapwidth,
            bmapheight,
            blockmap,
            blockmaplump,
            bmaporgx,
            bmaporgy,
            blocklinks,
            rejectmatrix,
            raw_names: RawTextureNames {
                side_names,
                sector_names,
            },
        }
    }

    /// Port of the texture/flat-name-resolution slice of `P_SetupLevel`
    /// that the original does inline while loading (via
    /// `R_TextureNumForName`/`R_FlatNumForName`, called from
    /// `P_LoadSideDefs`/`P_LoadSectors` themselves) — split out here as
    /// an explicit second step since it depends on
    /// [`crate::r_data::RData`] being initialized first (Phase 5c,
    /// ported independently of `p_setup`), where the original could
    /// just call straight into already-initialized global state.
    ///
    /// Must be called after [`Level::load`] and after
    /// `RData::init`/[`crate::r_data::RData::init`] has run against the
    /// same `wad`. Consumes [`Level::raw_names`] — calling this twice is
    /// harmless (the second call resolves against already-resolved
    /// `TEXTURE_UNRESOLVED`-free data, a no-op in practice since the raw
    /// names are gone) but pointless.
    ///
    /// A sidedef's texture name of `"-"` means "no texture" in the
    /// original (`R_CheckTextureNumForName`'s `NoTexture` marker) —
    /// resolved here as texture index 0, not [`TEXTURE_UNRESOLVED`],
    /// matching `check_texture_num_for_name`'s own documented behavior
    /// (see that method's docs) rather than this port inventing a
    /// separate "no texture" sentinel.
    pub fn resolve_textures(&mut self, rdata: &crate::r_data::RData, wad: &WadFiles) {
        for (sector, (floorpic_name, ceilingpic_name)) in
            self.sectors.iter_mut().zip(&self.raw_names.sector_names)
        {
            let floorpic_name = crate::r_data::lump_name_from_bytes(floorpic_name);
            let ceilingpic_name = crate::r_data::lump_name_from_bytes(ceilingpic_name);
            sector.floorpic = rdata.flat_num_for_name(wad, &floorpic_name) as i16;
            sector.ceilingpic = rdata.flat_num_for_name(wad, &ceilingpic_name) as i16;
        }

        for (side, (top_name, bottom_name, mid_name)) in
            self.sides.iter_mut().zip(&self.raw_names.side_names)
        {
            let top_name = crate::r_data::lump_name_from_bytes(top_name);
            let bottom_name = crate::r_data::lump_name_from_bytes(bottom_name);
            let mid_name = crate::r_data::lump_name_from_bytes(mid_name);
            // R_TextureNumForName (the panicking variant, not
            // R_CheckTextureNumForName) — the original itself calls
            // this one from P_LoadSideDefs, so an unresolvable texture
            // name is a fatal error here too, same as the original;
            // "-" (no texture) is handled inside
            // check_texture_num_for_name (called by this), not treated
            // as an error.
            side.toptexture = rdata.texture_num_for_name(&top_name) as i16;
            side.bottomtexture = rdata.texture_num_for_name(&bottom_name) as i16;
            side.midtexture = rdata.texture_num_for_name(&mid_name) as i16;
        }
    }
}

/// Port of `P_LoadThings`. Spawns every thing in the map's THINGS lump
/// via [`p_spawn_map_thing`], into `thinkers`/`level`.
///
/// The original's "don't spawn cool, new monsters if !commercial" guard
/// (skips Doom II-only monster types on non-`Commercial` `gamemode`,
/// breaking the loop entirely once one is seen, since THINGS entries
/// for a single episode/map are contiguous in practice) is ported
/// as-is, reading `doomstat::state().gamemode`.
pub fn p_load_things(thinkers: &mut Thinkers, level: &mut Level, wad: &mut WadFiles, lump: usize) {
    let data = read_lump_bytes(wad, lump);
    let thing_size = std::mem::size_of::<MapThing>();
    let numthings = data.len() / thing_size;

    for i in 0..numthings {
        let off = i * thing_size;
        let read_i16 = |at: usize| i16::from_le_bytes([data[off + at], data[off + at + 1]]);
        let mthing = MapThing {
            x: read_i16(0),
            y: read_i16(2),
            angle: read_i16(4),
            thing_type: read_i16(6),
            options: read_i16(8),
        };

        // Do not spawn cool, new monsters if !commercial (the
        // original's own comment, preserved).
        if doomstat::state().gamemode != GameMode::Commercial
            && matches!(
                mthing.thing_type,
                68 | 64 | 88 | 89 | 67 | 71 | 65 | 66 | 84
            )
        {
            break;
        }

        p_spawn_map_thing(thinkers, level, &mthing);
    }
}

/// The `MT_PLAYER` mobj half of `P_SpawnPlayer` (the `player_t` linkage
/// is `g_game::p_spawn_player`'s): spawned on the floor at the start,
/// facing its angle, tagged with its player index and — for players 2..4
/// — the colour translation of their sprites.
pub fn p_spawn_player_mobj(
    thinkers: &mut Thinkers,
    level: &mut Level,
    mthing: &MapThing,
) -> crate::p_tick::ThinkerId {
    let x = (mthing.x as i32) << FRACBITS;
    let y = (mthing.y as i32) << FRACBITS;
    let id = crate::p_mobj::p_spawn_mobj(
        thinkers,
        level,
        x,
        y,
        crate::p_mobj::SpawnZ::OnFloor,
        MobjType::MtPlayer,
    );
    let mobj = thinkers.mobj_mut(id).unwrap();
    mobj.player = Some((mthing.thing_type - 1) as usize);
    if mthing.thing_type > 1 {
        mobj.flags |= (mthing.thing_type as i32 - 1) << mobj_flag::TRANSSHIFT;
    }
    mobj.angle = ANG45.wrapping_mul((mthing.angle / 45) as u32);
    id
}

/// Port of `P_SpawnMapThing`. See module docs on the player-start
/// scoping (`P_SpawnPlayer`'s player-struct half isn't ported yet).
///
/// # Panics
/// Panics (standing in for `I_Error`) on a `doomednum` matching no
/// [`crate::info::MobjType`], same as the original.
pub fn p_spawn_map_thing(thinkers: &mut Thinkers, level: &mut Level, mthing: &MapThing) {
    let state = doomstat::state_mut();

    // count deathmatch start positions (the original's own comment,
    // preserved). `deathmatch_p` is reset to `Some(0)` by
    // `P_SetupLevel` before the THINGS are loaded.
    if mthing.thing_type == 11 {
        if let Some(slot) = state.deathmatch_p {
            if slot < MAX_DM_STARTS {
                state.deathmatchstarts[slot] = *mthing;
                state.deathmatch_p = Some(slot + 1);
            }
        }
        return;
    }

    // check for players specially (the original's own comment,
    // preserved).
    if mthing.thing_type <= 4 {
        // save spots for respawning in network games (the original's
        // own comment, preserved).
        state.playerstarts[(mthing.thing_type - 1) as usize] = *mthing;
        // `P_SpawnPlayer` returns early when `!playeringame[type-1]`; the
        // `player_t` linkage half lives in `g_game::p_spawn_player`
        // (called after the THINGS are loaded), found through the
        // `mobj.player` index set here.
        if !state.deathmatch && state.playeringame[(mthing.thing_type - 1) as usize] {
            p_spawn_player_mobj(thinkers, level, mthing);
        }
        return;
    }

    // check for appropriate skill level (the original's own comment,
    // preserved). Skill selection isn't wired up yet (Phase 6d/g_game);
    // `gameskill` defaults to `Skill::Medium` (see `doomstat`), matching
    // the original's own `startskill` default.
    if !state.netgame && mthing.options & 16 != 0 {
        return;
    }

    let bit = match state.gameskill {
        Skill::Baby => 1,
        Skill::Nightmare => 4,
        other => 1 << (other as i32 - 1),
    };
    if mthing.options & bit == 0 {
        return;
    }

    // find which type to spawn (the original's own comment, preserved).
    let mobj_type = MOBJINFO
        .iter()
        .position(|m| m.doomednum == mthing.thing_type as i32)
        .unwrap_or_else(|| {
            panic!(
                "P_SpawnMapThing: Unknown type {} at ({}, {})",
                mthing.thing_type, mthing.x, mthing.y
            )
        });
    let info = &MOBJINFO[mobj_type];
    let mobj_type = MobjType::from_index(mobj_type).expect("index came from MOBJINFO itself");

    // don't spawn keycards and players in deathmatch (the original's
    // own comment, preserved).
    if state.deathmatch && info.flags & mobj_flag::NOTDMATCH != 0 {
        return;
    }

    // don't spawn any monsters if -nomonsters (the original's own
    // comment, preserved).
    if state.nomonsters
        && (mobj_type == MobjType::MtSkull || info.flags & mobj_flag::COUNTKILL != 0)
    {
        return;
    }

    // spawn it (the original's own comment, preserved).
    let x = (mthing.x as i32) << FRACBITS;
    let y = (mthing.y as i32) << FRACBITS;
    let z = if info.flags & mobj_flag::SPAWNCEILING != 0 {
        crate::p_mobj::SpawnZ::OnCeiling
    } else {
        crate::p_mobj::SpawnZ::OnFloor
    };

    let id = crate::p_mobj::p_spawn_mobj(thinkers, level, x, y, z, mobj_type);
    let mobj = thinkers.mobj_mut(id).unwrap();
    mobj.spawnpoint = *mthing;

    if mobj.tics > 0 {
        mobj.tics = 1 + (crate::m_random::p_random() % mobj.tics);
    }
    if mobj.flags & mobj_flag::COUNTKILL != 0 {
        state.totalkills += 1;
    }
    if mobj.flags & mobj_flag::COUNTITEM != 0 {
        state.totalitems += 1;
    }

    mobj.angle = ANG45.wrapping_mul((mthing.angle / 45) as u32);
    if mthing.options & (MTF_AMBUSH as i16) != 0 {
        mobj.flags |= mobj_flag::AMBUSH;
    }
}

/// (`ML_*`) lump offsets from the map marker lump, matching
/// [`crate::doomdata::MapLump`] but scoped locally as `usize` offsets
/// for the `lumpnum + OFFSET` arithmetic `P_SetupLevel` does throughout.
#[repr(usize)]
enum MapLumpOffset {
    #[allow(dead_code)]
    Label = 0,
    Things = 1,
    Linedefs = 2,
    Sidedefs = 3,
    Vertexes = 4,
    Segs = 5,
    Ssectors = 6,
    Nodes = 7,
    Sectors = 8,
    Reject = 9,
    Blockmap = 10,
}

// Uppercase aliases matching the original's `ML_*` spelling at call
// sites above, since `MapLumpOffset::Linedefs` etc. reads awkwardly
// against the rest of this file's ML_* naming.
#[allow(non_upper_case_globals)]
impl MapLumpOffset {
    const LINEDEFS: MapLumpOffset = MapLumpOffset::Linedefs;
    const SIDEDEFS: MapLumpOffset = MapLumpOffset::Sidedefs;
    const VERTEXES: MapLumpOffset = MapLumpOffset::Vertexes;
    const SEGS: MapLumpOffset = MapLumpOffset::Segs;
    const SSECTORS: MapLumpOffset = MapLumpOffset::Ssectors;
    const NODES: MapLumpOffset = MapLumpOffset::Nodes;
    const SECTORS: MapLumpOffset = MapLumpOffset::Sectors;
    const REJECT: MapLumpOffset = MapLumpOffset::Reject;
    const BLOCKMAP: MapLumpOffset = MapLumpOffset::Blockmap;
    const THINGS: MapLumpOffset = MapLumpOffset::Things;
}

/// Read a lump's raw bytes into an owned `Vec<u8>` via
/// [`WadFiles::read_lump`], sized by [`WadFiles::lump_length`] — the
/// common first step of every `P_Load*` function below (the original's
/// `data = W_CacheLumpNum(lump, PU_STATIC); ... Z_Free(data);` pattern:
/// cache long enough to copy out, then free — ported as a direct read
/// into an owned buffer instead, since there's no reason to route
/// through the zone cache for data that's immediately copied out and
/// discarded either way).
fn read_lump_bytes(wad: &mut WadFiles, lump: usize) -> Vec<u8> {
    let len = wad.lump_length(lump);
    let mut buf = vec![0u8; len];
    wad.read_lump(lump, &mut buf);
    buf
}

/// Port of `P_LoadVertexes`.
fn load_vertexes(wad: &mut WadFiles, lump: usize) -> Vec<Vertex> {
    let data = read_lump_bytes(wad, lump);
    let count = data.len() / std::mem::size_of::<MapVertex>();

    let mut vertexes = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * std::mem::size_of::<MapVertex>();
        let x = i16::from_le_bytes([data[off], data[off + 1]]);
        let y = i16::from_le_bytes([data[off + 2], data[off + 3]]);
        vertexes.push(Vertex {
            x: (x as i32) << FRACBITS,
            y: (y as i32) << FRACBITS,
        });
    }
    vertexes
}

/// Port of `P_LoadSectors`.
///
/// `floorpic`/`ceilingpic` are left at [`TEXTURE_UNRESOLVED`] until
/// [`Level::resolve_textures`] runs — the raw name bytes are returned
/// alongside for it to use (see [`Level::raw_names`]'s docs).
fn load_sectors(wad: &mut WadFiles, lump: usize) -> (Vec<Sector>, Vec<SectorNames>) {
    let data = read_lump_bytes(wad, lump);
    let count = data.len() / std::mem::size_of::<MapSector>();

    let mut sectors = Vec::with_capacity(count);
    let mut names = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * std::mem::size_of::<MapSector>();
        let floorheight = i16::from_le_bytes([data[off], data[off + 1]]);
        let ceilingheight = i16::from_le_bytes([data[off + 2], data[off + 3]]);
        let floorpic_name: [u8; 8] = data[off + 4..off + 12].try_into().unwrap();
        let ceilingpic_name: [u8; 8] = data[off + 12..off + 20].try_into().unwrap();
        let lightlevel = i16::from_le_bytes([data[off + 20], data[off + 21]]);
        let special = i16::from_le_bytes([data[off + 22], data[off + 23]]);
        let tag = i16::from_le_bytes([data[off + 24], data[off + 25]]);

        sectors.push(Sector {
            floorheight: (floorheight as i32) << FRACBITS,
            ceilingheight: (ceilingheight as i32) << FRACBITS,
            floorpic: TEXTURE_UNRESOLVED,
            ceilingpic: TEXTURE_UNRESOLVED,
            lightlevel,
            special,
            tag,
            ..Sector::default()
        });
        names.push((floorpic_name, ceilingpic_name));
    }
    (sectors, names)
}

/// Port of `P_LoadSideDefs`.
///
/// `toptexture`/`bottomtexture`/`midtexture` are left at
/// [`TEXTURE_UNRESOLVED`] until [`Level::resolve_textures`] runs — the
/// raw name bytes are returned alongside for it to use (see
/// [`Level::raw_names`]'s docs).
///
/// # Panics
/// Panics if a sidedef references a sector index out of range (the
/// original has no explicit check here either — it would read/write out
/// of bounds; this port panics instead of doing that, strictly safer
/// while remaining equally "fatal on bad data").
fn load_sidedefs(
    wad: &mut WadFiles,
    lump: usize,
    sectors: &[Sector],
) -> (Vec<Side>, Vec<SideNames>) {
    let data = read_lump_bytes(wad, lump);
    let count = data.len() / std::mem::size_of::<MapSidedef>();

    let mut sides = Vec::with_capacity(count);
    let mut names = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * std::mem::size_of::<MapSidedef>();
        let textureoffset = i16::from_le_bytes([data[off], data[off + 1]]);
        let rowoffset = i16::from_le_bytes([data[off + 2], data[off + 3]]);
        let toptexture_name: [u8; 8] = data[off + 4..off + 12].try_into().unwrap();
        let bottomtexture_name: [u8; 8] = data[off + 12..off + 20].try_into().unwrap();
        let midtexture_name: [u8; 8] = data[off + 20..off + 28].try_into().unwrap();
        let sector = i16::from_le_bytes([data[off + 28], data[off + 29]]);

        assert!(
            (sector as usize) < sectors.len(),
            "P_LoadSideDefs: sidedef {i} references out-of-range sector {sector}"
        );

        sides.push(Side {
            textureoffset: (textureoffset as i32) << FRACBITS,
            rowoffset: (rowoffset as i32) << FRACBITS,
            toptexture: TEXTURE_UNRESOLVED,
            bottomtexture: TEXTURE_UNRESOLVED,
            midtexture: TEXTURE_UNRESOLVED,
            sector: sector as usize,
        });
        names.push((toptexture_name, bottomtexture_name, midtexture_name));
    }
    (sides, names)
}

/// Port of `P_LoadLineDefs`. Also counts secret lines for
/// intermissions (the original's own comment — but see module docs:
/// that counting itself, tied to `doomstat` globals not populated by
/// this phase, isn't reproduced here; only the per-line data the
/// original comment refers to indirectly through side effects on those
/// globals is out of scope, not this function's core job).
fn load_linedefs(
    wad: &mut WadFiles,
    lump: usize,
    vertexes: &[Vertex],
    sides: &[Side],
) -> Vec<Line> {
    let data = read_lump_bytes(wad, lump);
    let count = data.len() / std::mem::size_of::<MapLinedef>();

    let mut lines = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * std::mem::size_of::<MapLinedef>();
        let v1_idx = i16::from_le_bytes([data[off], data[off + 1]]) as usize;
        let v2_idx = i16::from_le_bytes([data[off + 2], data[off + 3]]) as usize;
        let flags = i16::from_le_bytes([data[off + 4], data[off + 5]]);
        let special = i16::from_le_bytes([data[off + 6], data[off + 7]]);
        let tag = i16::from_le_bytes([data[off + 8], data[off + 9]]);
        let side0 = i16::from_le_bytes([data[off + 10], data[off + 11]]);
        let side1 = i16::from_le_bytes([data[off + 12], data[off + 13]]);

        let v1 = vertexes[v1_idx];
        let v2 = vertexes[v2_idx];
        let dx = v2.x - v1.x;
        let dy = v2.y - v1.y;

        let slopetype = if dx == 0 {
            SlopeType::Vertical
        } else if dy == 0 {
            SlopeType::Horizontal
        } else if fixed_div(dy, dx) > 0 {
            SlopeType::Positive
        } else {
            SlopeType::Negative
        };

        let mut bbox: BBox = [0; 4];
        if v1.x < v2.x {
            bbox[BOXLEFT] = v1.x;
            bbox[BOXRIGHT] = v2.x;
        } else {
            bbox[BOXLEFT] = v2.x;
            bbox[BOXRIGHT] = v1.x;
        }
        if v1.y < v2.y {
            bbox[BOXBOTTOM] = v1.y;
            bbox[BOXTOP] = v2.y;
        } else {
            bbox[BOXBOTTOM] = v2.y;
            bbox[BOXTOP] = v1.y;
        }

        let sidenum = [
            if side0 != -1 {
                Some(side0 as usize)
            } else {
                None
            },
            if side1 != -1 {
                Some(side1 as usize)
            } else {
                None
            },
        ];

        let frontsector = sidenum[0].map(|s| sides[s].sector);
        let backsector = sidenum[1].map(|s| sides[s].sector);

        lines.push(Line {
            v1: v1_idx,
            v2: v2_idx,
            dx,
            dy,
            flags,
            special,
            tag,
            sidenum,
            bbox,
            slopetype,
            frontsector,
            backsector,
            validcount: 0,
            specialdata: None,
        });
    }
    lines
}

/// Port of `P_LoadSubsectors`.
fn load_subsectors(wad: &mut WadFiles, lump: usize) -> Vec<Subsector> {
    let data = read_lump_bytes(wad, lump);
    let count = data.len() / std::mem::size_of::<MapSubsector>();

    let mut subsectors = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * std::mem::size_of::<MapSubsector>();
        let numsegs = i16::from_le_bytes([data[off], data[off + 1]]);
        let firstseg = i16::from_le_bytes([data[off + 2], data[off + 3]]);
        subsectors.push(Subsector {
            // sector is filled in by group_lines (P_GroupLines), same
            // as the original leaving it zeroed until then.
            sector: 0,
            numlines: numsegs,
            firstline: firstseg,
        });
    }
    subsectors
}

/// Port of `P_LoadNodes`.
fn load_nodes(wad: &mut WadFiles, lump: usize) -> Vec<Node> {
    let data = read_lump_bytes(wad, lump);
    let count = data.len() / std::mem::size_of::<MapNode>();

    let mut nodes = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * std::mem::size_of::<MapNode>();
        let x = i16::from_le_bytes([data[off], data[off + 1]]);
        let y = i16::from_le_bytes([data[off + 2], data[off + 3]]);
        let dx = i16::from_le_bytes([data[off + 4], data[off + 5]]);
        let dy = i16::from_le_bytes([data[off + 6], data[off + 7]]);

        let mut bbox = [[0i32; 4]; 2];
        let mut bbox_off = off + 8;
        for side_bbox in &mut bbox {
            for coord in side_bbox.iter_mut() {
                let v = i16::from_le_bytes([data[bbox_off], data[bbox_off + 1]]);
                *coord = (v as i32) << FRACBITS;
                bbox_off += 2;
            }
        }

        let children = [
            u16::from_le_bytes([data[bbox_off], data[bbox_off + 1]]),
            u16::from_le_bytes([data[bbox_off + 2], data[bbox_off + 3]]),
        ];

        nodes.push(Node {
            x: (x as i32) << FRACBITS,
            y: (y as i32) << FRACBITS,
            dx: (dx as i32) << FRACBITS,
            dy: (dy as i32) << FRACBITS,
            bbox,
            children,
        });
    }
    nodes
}

/// Port of `P_LoadSegs`.
///
/// # Panics
/// Panics if a seg references an out-of-range linedef/vertex/side index
/// — see [`load_sidedefs`]'s note on why this port panics rather than
/// reading out of bounds like the original would.
fn load_segs(
    wad: &mut WadFiles,
    lump: usize,
    vertexes: &[Vertex],
    lines: &[Line],
    sides: &[Side],
) -> Vec<Seg> {
    let data = read_lump_bytes(wad, lump);
    let count = data.len() / std::mem::size_of::<MapSeg>();

    let mut segs = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * std::mem::size_of::<MapSeg>();
        let v1 = i16::from_le_bytes([data[off], data[off + 1]]) as usize;
        let v2 = i16::from_le_bytes([data[off + 2], data[off + 3]]) as usize;
        let angle = i16::from_le_bytes([data[off + 4], data[off + 5]]);
        let linedef = i16::from_le_bytes([data[off + 6], data[off + 7]]) as usize;
        let side = i16::from_le_bytes([data[off + 8], data[off + 9]]);
        let seg_offset = i16::from_le_bytes([data[off + 10], data[off + 11]]);

        assert!(v1 < vertexes.len(), "P_LoadSegs: seg {i} v1 out of range");
        assert!(v2 < vertexes.len(), "P_LoadSegs: seg {i} v2 out of range");
        assert!(
            linedef < lines.len(),
            "P_LoadSegs: seg {i} linedef out of range"
        );

        let ldef = &lines[linedef];
        let side_index = ldef.sidenum[side as usize]
            .unwrap_or_else(|| panic!("P_LoadSegs: seg {i} references a missing side"));
        let sidedef = &sides[side_index];
        let frontsector = sidedef.sector;

        let backsector = if ldef.flags & crate::doomdata::ML_TWOSIDED != 0 {
            // side^1: the other side of the linedef.
            let other = ldef.sidenum[(side ^ 1) as usize];
            other.map(|s| sides[s].sector)
        } else {
            None
        };

        segs.push(Seg {
            v1,
            v2,
            angle: ((angle as i32) << 16) as u32,
            offset: (seg_offset as i32) << FRACBITS,
            sidedef: side_index,
            linedef,
            frontsector,
            backsector,
        });
    }
    segs
}

/// Port of the reject-matrix load in `P_SetupLevel`:
/// `rejectmatrix = W_CacheLumpNum(lumpnum+ML_REJECT, PU_LEVEL);`.
///
/// Unlike the `P_Load*` functions above, the original keeps this lump
/// cached in the zone (owned, `PU_LEVEL`) rather than copying it out and
/// freeing the cache slot — the reject matrix is read directly from
/// wherever the zone cached it, for the level's whole lifetime. This
/// port still just copies the bytes out into an owned `Vec<u8>` on
/// [`Level`] instead (same reasoning as [`read_lump_bytes`]: `Level`
/// itself now owns this data for its lifetime, so there's no benefit to
/// routing storage through the zone cache on top of that).
fn load_reject(wad: &mut WadFiles, lump: usize) -> Vec<u8> {
    let bytes = wad.cache_lump_num(lump, PurgeTag::Level);
    bytes.to_vec()
}

/// `(blockmap, blockmaplump, bmapwidth, bmapheight, bmaporgx, bmaporgy,
/// blocklinks)` — see [`load_blockmap`].
type BlockMapData = (
    Vec<i16>,
    Vec<i16>,
    i32,
    i32,
    i32,
    i32,
    Vec<Option<crate::p_tick::ThinkerId>>,
);

/// Port of `P_LoadBlockMap`.
fn load_blockmap(wad: &mut WadFiles, lump: usize) -> BlockMapData {
    let bytes = wad.cache_lump_num(lump, PurgeTag::Level).to_vec();
    let count = bytes.len() / 2;

    let mut blockmaplump = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * 2;
        blockmaplump.push(i16::from_le_bytes([bytes[off], bytes[off + 1]]));
    }

    let bmaporgx = (blockmaplump[0] as i32) << FRACBITS;
    let bmaporgy = (blockmaplump[1] as i32) << FRACBITS;
    let bmapwidth = blockmaplump[2] as i32;
    let bmapheight = blockmaplump[3] as i32;

    // blockmap = blockmaplump+4 (the original's own pointer arithmetic);
    // ported as a separate owned Vec slice-copy rather than aliasing.
    let blockmap = blockmaplump[4..].to_vec();

    let blocklinks_count = (bmapwidth as usize) * (bmapheight as usize);
    let blocklinks = vec![None; blocklinks_count];

    (
        blockmap,
        blockmaplump,
        bmapwidth,
        bmapheight,
        bmaporgx,
        bmaporgy,
        blocklinks,
    )
}

/// Port of `P_GroupLines`. Builds sector line lists and subsector sector
/// numbers. Finds block bounding boxes for sectors.
#[allow(clippy::too_many_arguments)] // mirrors the shape of state P_GroupLines touches in the original
fn group_lines(
    sectors: &mut [Sector],
    lines: &[Line],
    subsectors: &mut [Subsector],
    segs: &[Seg],
    sides: &[Side],
    vertexes: &[Vertex],
    bmapwidth: i32,
    bmapheight: i32,
    bmaporgx: i32,
    bmaporgy: i32,
) {
    // Look up sector number for each subsector.
    for ss in subsectors.iter_mut() {
        let seg = &segs[ss.firstline as usize];
        ss.sector = sides[seg.sidedef].sector;
    }

    // Build line tables for each sector. The original allocates one
    // shared buffer, counts each sector's lines into it up front, and
    // slices it per-sector via pointer arithmetic (`sector->lines =
    // linebuffer; ... linebuffer += ...`), then asserts the final
    // per-sector slice length matches the earlier count ("miscounted"
    // if not — a self-consistency check on its own two-pass logic).
    // Ported as each Sector directly owning its own Vec<usize> instead
    // (see r_defs::Sector::lines's doc comment): a single pass appends
    // matching lines directly, which makes the original's separate
    // count-then-fill-then-compare passes structurally unnecessary here
    // (a Vec doesn't need to know its final length up front the way a
    // pointer-sliced shared buffer did) — so there's nothing left to
    // miscount, and no equivalent assertion to port.
    for (sector_idx, sector) in sectors.iter_mut().enumerate() {
        let mut bbox: BBox = [0; 4];
        m_clear_box(&mut bbox);

        for (line_idx, li) in lines.iter().enumerate() {
            if li.frontsector == Some(sector_idx) || li.backsector == Some(sector_idx) {
                sector.lines.push(line_idx);
                let v1 = vertexes[li.v1];
                let v2 = vertexes[li.v2];
                m_add_to_box(&mut bbox, v1.x, v1.y);
                m_add_to_box(&mut bbox, v2.x, v2.y);
            }
        }

        // Set the degenmobj_t to the middle of the bounding box.
        sector.soundorg.x = (bbox[BOXRIGHT] + bbox[BOXLEFT]) / 2;
        sector.soundorg.y = (bbox[BOXTOP] + bbox[BOXBOTTOM]) / 2;

        // Adjust bounding box to map blocks.
        let mut block = (bbox[BOXTOP] - bmaporgy + MAXRADIUS) >> MAPBLOCKSHIFT;
        block = if block >= bmapheight {
            bmapheight - 1
        } else {
            block
        };
        sector.blockbox[BOXTOP] = block;

        let mut block = (bbox[BOXBOTTOM] - bmaporgy - MAXRADIUS) >> MAPBLOCKSHIFT;
        block = if block < 0 { 0 } else { block };
        sector.blockbox[BOXBOTTOM] = block;

        let mut block = (bbox[BOXRIGHT] - bmaporgx + MAXRADIUS) >> MAPBLOCKSHIFT;
        block = if block >= bmapwidth {
            bmapwidth - 1
        } else {
            block
        };
        sector.blockbox[BOXRIGHT] = block;

        let mut block = (bbox[BOXLEFT] - bmaporgx - MAXRADIUS) >> MAPBLOCKSHIFT;
        block = if block < 0 { 0 } else { block };
        sector.blockbox[BOXLEFT] = block;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapblockshift_matches_original() {
        assert_eq!(MAPBLOCKSHIFT, 23);
    }

    #[test]
    fn maxradius_matches_original() {
        assert_eq!(MAXRADIUS, 32 * 65536);
    }
}
