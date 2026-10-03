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
//	Movement, collision handling.
//	Shooting and aiming.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_map.c` (partial, see note below). Movement,
//! collision handling (the original's own description, preserved; its
//! shooting/aiming half is a later-phase scope note below).
//!
//! # Scope
//!
//! Ported: [`p_check_position`] (`P_CheckPosition`), [`p_try_move`]
//! (`P_TryMove`), [`p_thing_height_clip`] (`P_ThingHeightClip`),
//! [`p_slide_move`]/[`p_hit_slide_line`] (`P_SlideMove`/
//! `P_HitSlideLine`), [`p_teleport_move`] (`P_TeleportMove`, called by
//! [`crate::p_telept::ev_teleport`] since Phase 7b4),
//! [`pit_change_sector`]/[`p_change_sector`] (Phase 7b2, needed by
//! `T_MovePlane`'s crush check).
//!
//! As of Phase 7b4: [`pit_stomp_thing`]/[`pit_check_thing`] deal real
//! damage via `crate::p_inter::p_damage_mobj` (telefrag, skull-slam,
//! missile impact) and real pickups via
//! `crate::p_inter::p_touch_special_thing` — both were stubbed through
//! Phase 7a. [`p_try_move`] returns `(bool, Vec<usize>)` instead of a
//! bare `bool`: the `Vec<usize>` is `ctx.spechit` (special lines the
//! move crossed), since crossing them needs
//! `crate::p_spec::p_cross_special_line`, which needs a
//! `crate::p_spec::SpecialsCtx` this module intentionally doesn't
//! depend on (would create a `p_map` <-> `p_spec` cycle) — every caller
//! that owns a `SpecialsCtx` drains the returned vec itself.
//! `P_UseLines`/`PTR_UseTraverse` — despite living in the original's
//! `p_map.c` — ported into `p_user.rs` instead (it's `P_PlayerThink`'s
//! only caller, and needs the same `SpecialsCtx` `p_player_think` now
//! takes), not here.
//!
//! As of Phase 7d1: [`p_aim_line_attack`]/[`p_line_attack`] (`P_AimLineAttack`/
//! `P_LineAttack`, with `PTR_AimTraverse`/`PTR_ShootTraverse` as their
//! private traverser closures) and [`p_radius_attack`] (`P_RadiusAttack`,
//! with `PIT_RadiusAttack` as its private per-thing closure) are ported.
//! Like [`p_use_lines`](crate::p_user::p_use_lines)'s `PTR_UseTraverse`,
//! `PTR_ShootTraverse` can't mutate anything itself — `p_path_traverse`'s
//! callback only lends `&Level`/`&Thinkers` — so it records "what got
//! hit, and where" into a local `ShootHit`, and [`p_line_attack`] applies
//! the real effect (`p_spawn_puff`/`p_spawn_blood`/`p_damage_mobj`/
//! `p_shoot_special_line`) afterward, outside the closure. Both
//! traversers need `shootz`/`attackrange`/`aimslope` etc, the original's
//! file-scope scratch globals for one call — bundled into a local
//! `AimContext`/`ShootContext`, same convention as `p_sight.rs`'s
//! `SightTrace`. `p_shoot_special_line` needs `crate::p_spec::SpecialsCtx`,
//! which this module intentionally doesn't depend on (see the
//! `p_try_move`/spechit note above) — [`p_line_attack`] takes an
//! `Option<&mut crate::p_spec::SpecialsCtx>` instead of requiring one
//! (`None` from callers that don't own one yet, e.g. Phase 7d2's
//! monster attacks called from `p_mobj_thinker`, matching that same
//! documented gap).
//! # `R_PointToAngle2`'s viewx/viewy side effect
//!
//! `P_HitSlideLine` calls the original's `R_PointToAngle2`, which (see
//! [`crate::r_main::RMain::point_to_angle2`]'s docs) mutates
//! `viewx`/`viewy` as a side effect. Calling that from simulation code
//! would leak into the renderer's own state (which normally only
//! touches `viewx`/`viewy` once per frame, in `R_SetupFrame`). This port
//! saves and restores `rmain.viewx`/`viewy` around the calls
//! [`p_hit_slide_line`] needs, so play simulation never leaves the
//! renderer's view position mutated after a tic — a divergence in favor
//! of not leaking state, not in the actual angle math (both calls here
//! pass `0, 0` as the first point, so the math is unaffected either
//! way).
//!
//! # `PIT_CheckThing`'s skull-slam branch doesn't remove mobjs
//!
//! The original's `PIT_CheckThing` calls `P_SetMobjState` (which can
//! free the *chasing* mobj if its `spawnstate` chain reaches `S_NULL`).
//! No monster's `spawnstate` does that in practice (it would mean a
//! monster spawns already dead), and skull-slamming itself is dead
//! monster AI this phase doesn't drive yet (Phase 7) — so
//! [`pit_check_thing`] resets the skull's momentum/flags but doesn't
//! call `p_set_mobj_state`, avoiding a `&mut Level` borrow this
//! function's callers don't have to hand it for a path nothing
//! exercises yet. Documented rather than silently simplified.

use crate::m_bbox::{BBox, BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};
use crate::m_fixed::{fixed_mul, Fixed, FRACUNIT};
use crate::p_maputl::{
    p_block_lines_iterator, p_block_things_iterator, p_box_on_line_side, p_line_opening,
    p_path_traverse, p_point_on_line_side, Intercept, InterceptTarget, PT_ADDLINES,
};
use crate::p_mobj::{p_set_thing_position, p_unset_thing_position};
use crate::p_setup::{Level, MAPBLOCKSHIFT, MAXRADIUS};
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_defs::mobj_flag;
use crate::r_main::RMain;
use crate::tables::{fine_cosine, ANG180, ANGLETOFINESHIFT, FINESINE};

/// Max special lines hit in one move, matching the original's
/// `MAXSPECIALCROSS`.
pub const MAXSPECIALCROSS: usize = 8;

/// The state of an in-progress move check, standing in for the
/// original's `tmthing`/`tmflags`/`tmx`/`tmy`/`tmbbox`/`tmfloorz`/
/// `tmceilingz`/`tmdropoffz`/`ceilingline`/`spechit`/`numspechit`
/// globals — passed explicitly through [`p_check_position`]/
/// [`p_try_move`]/their `PIT_*` helpers instead of module statics (same
/// approach [`crate::p_maputl::PathTraverse`] took for `p_maputl.c`'s
/// globals).
#[derive(Debug, Clone)]
pub struct MoveContext {
    pub thing: ThinkerId,
    pub flags: i32,
    pub is_player: bool,
    pub bbox: BBox,
    pub x: Fixed,
    pub y: Fixed,

    /// If true, move would be ok if within `floorz`-`ceilingz` (the
    /// original's own comment, preserved).
    pub floatok: bool,
    pub floorz: Fixed,
    pub ceilingz: Fixed,
    pub dropoffz: Fixed,

    /// Keep track of the line that lowers the ceiling, so missiles
    /// don't explode against sky hack walls (the original's own
    /// comment, preserved).
    pub ceilingline: Option<usize>,

    /// Keep track of special lines as they are hit, but don't process
    /// them until the move is proven valid (the original's own
    /// comment, preserved). Collected but not drained — see module
    /// docs.
    pub spechit: Vec<usize>,
}

impl MoveContext {
    fn new(
        thinkers: &Thinkers,
        level: &Level,
        thing: ThinkerId,
        x: Fixed,
        y: Fixed,
    ) -> MoveContext {
        let mobj = thinkers.mobj(thing).unwrap();
        let mut ctx = MoveContext {
            thing,
            flags: mobj.flags,
            is_player: mobj.player.is_some(),
            // [BOXTOP, BOXBOTTOM, BOXLEFT, BOXRIGHT]
            bbox: [
                y + mobj.radius,
                y - mobj.radius,
                x - mobj.radius,
                x + mobj.radius,
            ],
            x,
            y,
            floatok: false,
            floorz: 0,
            ceilingz: 0,
            dropoffz: 0,
            ceilingline: None,
            spechit: Vec::new(),
        };

        let ss = RMain::point_in_subsector(level, x, y);
        let sector = level.subsectors[ss].sector;
        ctx.floorz = level.sectors[sector].floorheight;
        ctx.dropoffz = ctx.floorz;
        ctx.ceilingz = level.sectors[sector].ceilingheight;
        ctx
    }
}

/// Port of `PIT_StompThing`. Whether `thing` blocks a teleport-move to
/// `tmx`/`tmy`; if it stomps a shootable thing on the boss level, deals
/// the telefrag damage.
fn pit_stomp_thing(
    ctx: &MoveContext,
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    id: ThinkerId,
) -> bool {
    let thing = *thinkers.mobj(id).unwrap();
    if thing.flags & mobj_flag::SHOOTABLE == 0 {
        return true;
    }

    let tmthing = *thinkers.mobj(ctx.thing).unwrap();
    let blockdist = thing.radius + tmthing.radius;
    if (thing.x - ctx.x).wrapping_abs() >= blockdist
        || (thing.y - ctx.y).wrapping_abs() >= blockdist
    {
        return true; // didn't hit it
    }

    if id == ctx.thing {
        return true; // don't clip against self
    }

    // monsters don't stomp things except on boss level (the original's
    // own comment, preserved). `gamemap` isn't wired up yet — treated as
    // never map 30 (never the boss level), matching every non-MAP30
    // playthrough.
    if !ctx.is_player {
        return false;
    }

    crate::p_inter::p_damage_mobj(
        thinkers,
        level,
        players,
        rmain,
        id,
        Some(ctx.thing),
        Some(ctx.thing),
        10000,
    );
    true
}

/// Port of `P_TeleportMove`.
pub fn p_teleport_move(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    thing: ThinkerId,
    x: Fixed,
    y: Fixed,
) -> bool {
    let ctx = MoveContext::new(thinkers, level, thing, x, y);

    // stomp on any things contacted (the original's own comment,
    // preserved). `p_block_things_iterator` only needs `&Level`, but
    // `pit_stomp_thing` needs `&mut Level` for `P_DamageMobj` — same
    // two-pass collect-then-process shape as `p_change_sector` (see its
    // docs for why).
    let xl = (ctx.bbox[BOXLEFT] - level.bmaporgx - MAXRADIUS) >> MAPBLOCKSHIFT;
    let xh = (ctx.bbox[BOXRIGHT] - level.bmaporgx + MAXRADIUS) >> MAPBLOCKSHIFT;
    let yl = (ctx.bbox[BOXBOTTOM] - level.bmaporgy - MAXRADIUS) >> MAPBLOCKSHIFT;
    let yh = (ctx.bbox[BOXTOP] - level.bmaporgy + MAXRADIUS) >> MAPBLOCKSHIFT;

    let mut ids: Vec<ThinkerId> = Vec::new();
    for bx in xl..=xh {
        for by in yl..=yh {
            p_block_things_iterator(thinkers, level, bx, by, |_, id| {
                ids.push(id);
                true
            });
        }
    }

    for id in ids {
        if thinkers.mobj(id).is_none() {
            continue; // already removed earlier in this pass
        }
        if !pit_stomp_thing(&ctx, thinkers, level, players, rmain, id) {
            return false;
        }
    }

    // the move is ok, so link the thing into its new position (the
    // original's own comment, preserved).
    p_unset_thing_position(thinkers, level, thing);
    {
        let mobj = thinkers.mobj_mut(thing).unwrap();
        mobj.floorz = ctx.floorz;
        mobj.ceilingz = ctx.ceilingz;
        mobj.x = x;
        mobj.y = y;
    }
    p_set_thing_position(thinkers, level, thing);

    true
}

/// Port of `PIT_CheckLine`. Adjusts `ctx.floorz`/`ctx.ceilingz` as lines
/// are contacted (the original's own comment, preserved).
fn pit_check_line(ctx: &mut MoveContext, level: &Level, line_idx: usize) -> bool {
    let line = &level.lines[line_idx];

    if ctx.bbox[BOXRIGHT] <= line.bbox[BOXLEFT]
        || ctx.bbox[BOXLEFT] >= line.bbox[BOXRIGHT]
        || ctx.bbox[BOXTOP] <= line.bbox[BOXBOTTOM]
        || ctx.bbox[BOXBOTTOM] >= line.bbox[BOXTOP]
    {
        return true;
    }

    if p_box_on_line_side(&ctx.bbox, level, line) != -1 {
        return true;
    }

    // A line has been hit. The moving thing's destination position will
    // cross the given line. If this should not be allowed, return
    // false. If the line is special, keep track of it to process later
    // if the move is proven ok. NOTE: specials are NOT sorted by order,
    // so two special lines that are only 8 pixels apart could be
    // crossed in either order (the original's own comment, preserved).
    if line.backsector.is_none() {
        return false; // one sided line
    }

    if ctx.flags & mobj_flag::MISSILE == 0 {
        if line.flags & crate::doomdata::ML_BLOCKING != 0 {
            return false; // explicitly blocking everything
        }
        if !ctx.is_player && line.flags & crate::doomdata::ML_BLOCKMONSTERS != 0 {
            return false; // block monsters only
        }
    }

    // set openrange, opentop, openbottom (the original's own comment,
    // preserved).
    let opening = p_line_opening(level, line);

    if opening.opentop < ctx.ceilingz {
        ctx.ceilingz = opening.opentop;
        ctx.ceilingline = Some(line_idx);
    }
    if opening.openbottom > ctx.floorz {
        ctx.floorz = opening.openbottom;
    }
    if opening.lowfloor < ctx.dropoffz {
        ctx.dropoffz = opening.lowfloor;
    }

    // if contacted a special line, add it to the list (the original's
    // own comment, preserved).
    if line.special != 0 && ctx.spechit.len() < MAXSPECIALCROSS {
        ctx.spechit.push(line_idx);
    }

    true
}

/// Port of `PIT_CheckThing`.
fn pit_check_thing(
    ctx: &mut MoveContext,
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    id: ThinkerId,
) -> bool {
    let thing = *thinkers.mobj(id).unwrap();
    if thing.flags & (mobj_flag::SOLID | mobj_flag::SPECIAL | mobj_flag::SHOOTABLE) == 0 {
        return true;
    }

    let tmthing = *thinkers.mobj(ctx.thing).unwrap();
    let blockdist = thing.radius + tmthing.radius;
    if (thing.x - ctx.x).wrapping_abs() >= blockdist
        || (thing.y - ctx.y).wrapping_abs() >= blockdist
    {
        return true; // didn't hit it
    }

    if id == ctx.thing {
        return true; // don't clip against self
    }

    // check for skulls slamming into things (the original's own
    // comment, preserved).
    if tmthing.flags & mobj_flag::SKULLFLY != 0 {
        let damage = (crate::m_random::p_random() % 8 + 1) * tmthing.info.damage;
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            id,
            Some(ctx.thing),
            Some(ctx.thing),
            damage,
        );

        let tmthing_mut = thinkers.mobj_mut(ctx.thing).unwrap();
        tmthing_mut.flags &= !mobj_flag::SKULLFLY;
        tmthing_mut.momx = 0;
        tmthing_mut.momy = 0;
        tmthing_mut.momz = 0;

        let spawnstate = tmthing.info.spawnstate;
        crate::p_mobj::p_set_mobj_state(thinkers, level, ctx.thing, spawnstate, |_, _, _, _| {});

        return false; // stop moving
    }

    // missiles can hit other things (the original's own comment,
    // preserved).
    if tmthing.flags & mobj_flag::MISSILE != 0 {
        if tmthing.z > thing.z + thing.height {
            return true; // overhead
        }
        if tmthing.z + tmthing.height < thing.z {
            return true; // underneath
        }

        if let Some(target) = tmthing.target {
            if let Some(target_mobj) = thinkers.mobj(target) {
                let same_species = target_mobj.mobj_type == thing.mobj_type
                    || (target_mobj.mobj_type == crate::info::MobjType::MtKnight
                        && thing.mobj_type == crate::info::MobjType::MtBruiser)
                    || (target_mobj.mobj_type == crate::info::MobjType::MtBruiser
                        && thing.mobj_type == crate::info::MobjType::MtKnight);
                if same_species {
                    if id == target {
                        return true; // don't hit same species as originator
                    }
                    if thing.mobj_type != crate::info::MobjType::MtPlayer {
                        return false; // explode, but do no damage
                    }
                }
            }
        }

        if thing.flags & mobj_flag::SHOOTABLE == 0 {
            return thing.flags & mobj_flag::SOLID == 0; // didn't do any damage
        }

        // damage / explode (the original's own comment, preserved).
        let damage = (crate::m_random::p_random() % 8 + 1) * tmthing.info.damage;
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            id,
            Some(ctx.thing),
            tmthing.target,
            damage,
        );

        return false; // don't traverse any more
    }

    // check for special pickup (the original's own comment, preserved).
    if thing.flags & mobj_flag::SPECIAL != 0 {
        let solid = thing.flags & mobj_flag::SOLID != 0;
        if ctx.flags & mobj_flag::PICKUP != 0 {
            // can remove thing (the original's own comment, preserved).
            crate::p_inter::p_touch_special_thing(thinkers, level, players, id, ctx.thing);
        }
        return !solid;
    }

    thing.flags & mobj_flag::SOLID == 0
}

/// Port of `P_CheckPosition`. This is purely informative, nothing is
/// modified, except for the pickup-touch side effect `PIT_CheckThing`
/// itself has (the original's own comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn p_check_position(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    _validcount: i32,
    thing: ThinkerId,
    x: Fixed,
    y: Fixed,
) -> (bool, MoveContext) {
    let mut ctx = MoveContext::new(thinkers, level, thing, x, y);

    if ctx.flags & mobj_flag::NOCLIP != 0 {
        return (true, ctx);
    }

    // Check things first, possibly picking things up. The bounding box
    // is extended by MAXRADIUS because mobj_ts are grouped into
    // mapblocks based on their origin point, and can overlap into
    // adjacent blocks by up to MAXRADIUS units (the original's own
    // comment, preserved). `p_block_things_iterator` only needs
    // `&Level`, but `pit_check_thing` needs `&mut Level` (damage/pickup)
    // — same two-pass collect-then-process shape as `p_change_sector`.
    let xl = (ctx.bbox[BOXLEFT] - level.bmaporgx - MAXRADIUS) >> MAPBLOCKSHIFT;
    let xh = (ctx.bbox[BOXRIGHT] - level.bmaporgx + MAXRADIUS) >> MAPBLOCKSHIFT;
    let yl = (ctx.bbox[BOXBOTTOM] - level.bmaporgy - MAXRADIUS) >> MAPBLOCKSHIFT;
    let yh = (ctx.bbox[BOXTOP] - level.bmaporgy + MAXRADIUS) >> MAPBLOCKSHIFT;

    let mut ids: Vec<ThinkerId> = Vec::new();
    for bx in xl..=xh {
        for by in yl..=yh {
            p_block_things_iterator(thinkers, level, bx, by, |_, id| {
                ids.push(id);
                true
            });
        }
    }
    for id in ids {
        if thinkers.mobj(id).is_none() {
            continue; // already removed earlier in this pass
        }
        if !pit_check_thing(&mut ctx, thinkers, level, players, rmain, id) {
            return (false, ctx);
        }
    }

    // check lines (the original's own comment, preserved).
    // (`validcount++` — see `bump_validcount`.)
    let validcount = crate::p_maputl::bump_validcount();
    let xl = (ctx.bbox[BOXLEFT] - level.bmaporgx) >> MAPBLOCKSHIFT;
    let xh = (ctx.bbox[BOXRIGHT] - level.bmaporgx) >> MAPBLOCKSHIFT;
    let yl = (ctx.bbox[BOXBOTTOM] - level.bmaporgy) >> MAPBLOCKSHIFT;
    let yh = (ctx.bbox[BOXTOP] - level.bmaporgy) >> MAPBLOCKSHIFT;

    for bx in xl..=xh {
        for by in yl..=yh {
            let ok = p_block_lines_iterator(level, validcount, bx, by, |level, line_idx| {
                pit_check_line(&mut ctx, level, line_idx)
            });
            if !ok {
                return (false, ctx);
            }
        }
    }

    (true, ctx)
}

/// Port of `P_TryMove`. Attempt to move to a new position, crossing
/// special lines unless `MF_TELEPORT` is set (the original's own
/// comment, preserved). Special-line crossing is handled by the caller
/// via the returned `ctx.spechit` — see
/// [`crate::p_spec::p_cross_special_line`]'s callers (`p_user`'s
/// `p_player_think`/monster movement, once Phase 7c/7d drive it).
#[allow(clippy::too_many_arguments)]
pub fn p_try_move(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    thing: ThinkerId,
    x: Fixed,
    y: Fixed,
) -> (bool, Vec<usize>) {
    let (ok, mut ctx) = p_check_position(thinkers, level, players, rmain, validcount, thing, x, y);
    if !ok {
        return (false, Vec::new()); // solid wall or thing
    }

    let mobj = *thinkers.mobj(thing).unwrap();

    if mobj.flags & mobj_flag::NOCLIP == 0 {
        if ctx.ceilingz - ctx.floorz < mobj.height {
            return (false, Vec::new()); // doesn't fit
        }

        ctx.floatok = true;

        if mobj.flags & mobj_flag::TELEPORT == 0 && ctx.ceilingz - mobj.z < mobj.height {
            return (false, Vec::new()); // mobj must lower itself to fit
        }

        if mobj.flags & mobj_flag::TELEPORT == 0 && ctx.floorz - mobj.z > 24 * FRACUNIT {
            return (false, Vec::new()); // too big a step up
        }

        if mobj.flags & (mobj_flag::DROPOFF | mobj_flag::FLOAT) == 0
            && ctx.floorz - ctx.dropoffz > 24 * FRACUNIT
        {
            return (false, Vec::new()); // don't stand over a dropoff
        }
    }

    // the move is ok, so link the thing into its new position (the
    // original's own comment, preserved).
    p_unset_thing_position(thinkers, level, thing);

    {
        let mobj = thinkers.mobj_mut(thing).unwrap();
        mobj.floorz = ctx.floorz;
        mobj.ceilingz = ctx.ceilingz;
        mobj.x = x;
        mobj.y = y;
    }

    p_set_thing_position(thinkers, level, thing);

    // if any special lines were hit, do the effect (the original's own
    // comment, preserved) — returned as `ctx.spechit` instead of drained
    // in place, since crossing them needs `P_CrossSpecialLine`
    // (`crate::p_spec`), which this module doesn't depend on (avoiding a
    // p_map <-> p_spec circular dependency); every caller that has a
    // `SpecialsCtx` in scope drains it via
    // `p_spec::p_cross_special_line` right after a successful move,
    // matching the original's call inside `P_TryMove` in spirit if not
    // call-site location.
    (
        true,
        if mobj.flags & mobj_flag::TELEPORT == 0 {
            ctx.spechit
        } else {
            Vec::new()
        },
    )
}

/// Port of `P_ThingHeightClip`. Takes a valid thing and adjusts the
/// thing's `floorz`, `ceilingz`, and possibly `z`. This is called for
/// all nearby monsters whenever a sector changes height. If the thing
/// doesn't fit, the z will be set to the lowest value and false will be
/// returned (the original's own comment, preserved).
pub fn p_thing_height_clip(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    thing: ThinkerId,
) -> bool {
    let (x, y, z, floorz, height) = {
        let mobj = thinkers.mobj(thing).unwrap();
        (mobj.x, mobj.y, mobj.z, mobj.floorz, mobj.height)
    };
    let onfloor = z == floorz;

    // what about stranding a monster partially off an edge? (the
    // original's own comment, preserved)
    let (_, ctx) = p_check_position(thinkers, level, players, rmain, validcount, thing, x, y);

    let mobj = thinkers.mobj_mut(thing).unwrap();
    mobj.floorz = ctx.floorz;
    mobj.ceilingz = ctx.ceilingz;

    if onfloor {
        // walking monsters rise and fall with the floor (the original's
        // own comment, preserved).
        mobj.z = mobj.floorz;
    } else if mobj.z + height > mobj.ceilingz {
        // don't adjust a floating monster unless forced to (the
        // original's own comment, preserved).
        mobj.z = mobj.ceilingz - height;
    }

    mobj.ceilingz - mobj.floorz >= height
}

/// Slide-move scratch state, standing in for the original's
/// `bestslidefrac`/`bestslideline`/`slidemo`/`tmxmove`/`tmymove`
/// globals. (`secondslidefrac`/`secondslideline` are write-only in the
/// original — set by `PTR_SlideTraverse` but never read anywhere — so
/// they're not reproduced here.)
struct SlideState {
    slidemo: ThinkerId,
    bestslidefrac: Fixed,
    bestslideline: Option<usize>,
    tmxmove: Fixed,
    tmymove: Fixed,
}

/// Port of `P_HitSlideLine`. Adjusts the xmove/ymove so that the next
/// move will slide along the wall (the original's own comment,
/// preserved). See module docs on the `viewx`/`viewy` save/restore
/// around `R_PointToAngle2`.
fn p_hit_slide_line(state: &mut SlideState, thinkers: &Thinkers, rmain: &mut RMain, level: &Level) {
    let line_idx = state
        .bestslideline
        .expect("caller only calls this after a hit");
    let line = &level.lines[line_idx];

    if line.slopetype == crate::r_defs::SlopeType::Horizontal {
        state.tmymove = 0;
        return;
    }
    if line.slopetype == crate::r_defs::SlopeType::Vertical {
        state.tmxmove = 0;
        return;
    }

    let slidemo = thinkers.mobj(state.slidemo).unwrap();
    let side = p_point_on_line_side(slidemo.x, slidemo.y, level, line);

    // R_PointToAngle2 mutates rmain.viewx/viewy as a side effect — save
    // and restore around both calls, see module docs.
    let (saved_viewx, saved_viewy) = (rmain.viewx, rmain.viewy);

    let mut lineangle = rmain.point_to_angle2(0, 0, line.dx, line.dy);
    if side == 1 {
        lineangle = lineangle.wrapping_add(ANG180);
    }
    let moveangle = rmain.point_to_angle2(0, 0, state.tmxmove, state.tmymove);

    rmain.viewx = saved_viewx;
    rmain.viewy = saved_viewy;

    let mut deltaangle = moveangle.wrapping_sub(lineangle);
    if deltaangle > ANG180 {
        deltaangle = deltaangle.wrapping_add(ANG180);
    }

    let lineangle_fine = (lineangle >> ANGLETOFINESHIFT) as usize;
    let deltaangle_fine = (deltaangle >> ANGLETOFINESHIFT) as usize;

    let movelen = crate::p_maputl::p_aprox_distance(state.tmxmove, state.tmymove);
    let newlen = fixed_mul(movelen, fine_cosine(deltaangle_fine));

    state.tmxmove = fixed_mul(newlen, fine_cosine(lineangle_fine));
    state.tmymove = fixed_mul(newlen, FINESINE[lineangle_fine]);
}

/// Port of `PTR_SlideTraverse`.
fn ptr_slide_traverse(
    state: &mut SlideState,
    thinkers: &Thinkers,
    level: &Level,
    intercept: &Intercept,
) -> bool {
    let InterceptTarget::Line(line_idx) = intercept.target else {
        panic!("PTR_SlideTraverse: not a line?");
    };
    let line = &level.lines[line_idx];
    let slidemo = thinkers.mobj(state.slidemo).unwrap();

    let blocking = if line.flags & crate::doomdata::ML_TWOSIDED == 0 {
        if p_point_on_line_side(slidemo.x, slidemo.y, level, line) != 0 {
            return true; // don't hit the back side
        }
        true
    } else {
        let opening = p_line_opening(level, line);
        !(opening.openrange >= slidemo.height
            && opening.opentop - slidemo.z >= slidemo.height
            && opening.openbottom - slidemo.z <= 24 * FRACUNIT)
    };

    if !blocking {
        return true; // this line doesn't block movement
    }

    // the line does block movement, see if it is closer than best so
    // far (the original's own comment, preserved).
    if intercept.frac < state.bestslidefrac {
        state.bestslidefrac = intercept.frac;
        state.bestslideline = Some(line_idx);
    }

    false // stop
}

/// Port of `P_SlideMove`. The momx/momy move is bad, so try to slide
/// along a wall. Find the first line hit, move flush to it, and slide
/// along it. This is a kludgy mess (the original's own comments,
/// preserved).
pub fn p_slide_move(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    mo: ThinkerId,
) {
    let mut state = SlideState {
        slidemo: mo,
        bestslidefrac: 0,
        bestslideline: None,
        tmxmove: 0,
        tmymove: 0,
    };

    let mut hitcount = 0;
    loop {
        hitcount += 1;
        if hitcount == 3 {
            // don't loop forever (the original's own comment, preserved)
            stairstep(thinkers, level, players, rmain, validcount, mo);
            return;
        }

        let (momx, momy, x, y, radius) = {
            let m = thinkers.mobj(mo).unwrap();
            (m.momx, m.momy, m.x, m.y, m.radius)
        };

        // trace along the three leading corners (the original's own
        // comment, preserved).
        let (leadx, trailx) = if momx > 0 {
            (x + radius, x - radius)
        } else {
            (x - radius, x + radius)
        };
        let (leady, traily) = if momy > 0 {
            (y + radius, y - radius)
        } else {
            (y - radius, y + radius)
        };

        state.bestslidefrac = FRACUNIT + 1;
        state.bestslideline = None;

        for (sx, sy) in [(leadx, leady), (trailx, leady), (leadx, traily)] {
            p_path_traverse(
                level,
                thinkers,
                validcount,
                sx,
                sy,
                sx + momx,
                sy + momy,
                PT_ADDLINES,
                |level, thinkers, ic| ptr_slide_traverse(&mut state, thinkers, level, ic),
            );
        }

        // move up to the wall (the original's own comment, preserved).
        if state.bestslidefrac == FRACUNIT + 1 {
            // the move must have hit the middle, so stairstep (the
            // original's own comment, preserved).
            stairstep(thinkers, level, players, rmain, validcount, mo);
            return;
        }

        // fudge a bit to make sure it doesn't hit (the original's own
        // comment, preserved).
        state.bestslidefrac -= 0x800;
        if state.bestslidefrac > 0 {
            let (momx, momy, x, y) = {
                let m = thinkers.mobj(mo).unwrap();
                (m.momx, m.momy, m.x, m.y)
            };
            let newx = fixed_mul(momx, state.bestslidefrac);
            let newy = fixed_mul(momy, state.bestslidefrac);
            let (moved, _spechit) = p_try_move(
                thinkers,
                level,
                players,
                rmain,
                validcount,
                mo,
                x + newx,
                y + newy,
            );
            if !moved {
                stairstep(thinkers, level, players, rmain, validcount, mo);
                return;
            }
        }

        // Now continue along the wall. First calculate remainder (the
        // original's own comment, preserved).
        state.bestslidefrac = FRACUNIT - (state.bestslidefrac + 0x800);
        if state.bestslidefrac > FRACUNIT {
            state.bestslidefrac = FRACUNIT;
        }
        if state.bestslidefrac <= 0 {
            return;
        }

        let (momx, momy) = {
            let m = thinkers.mobj(mo).unwrap();
            (m.momx, m.momy)
        };
        state.tmxmove = fixed_mul(momx, state.bestslidefrac);
        state.tmymove = fixed_mul(momy, state.bestslidefrac);

        p_hit_slide_line(&mut state, thinkers, rmain, level); // clip the moves

        {
            let m = thinkers.mobj_mut(mo).unwrap();
            m.momx = state.tmxmove;
            m.momy = state.tmymove;
        }

        let (x, y) = {
            let m = thinkers.mobj(mo).unwrap();
            (m.x, m.y)
        };
        let (moved, _spechit) = p_try_move(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            mo,
            x + state.tmxmove,
            y + state.tmymove,
        );
        if moved {
            return;
        }
        // retry (the original's own `goto retry`, reproduced as looping
        // back around).
    }
}

/// The `stairstep:` label's body in the original's `P_SlideMove` — try
/// moving on Y then X alone.
fn stairstep(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    mo: ThinkerId,
) {
    let (momx, momy, x, y) = {
        let m = thinkers.mobj(mo).unwrap();
        (m.momx, m.momy, m.x, m.y)
    };
    let (moved, _spechit) =
        p_try_move(thinkers, level, players, rmain, validcount, mo, x, y + momy);
    if !moved {
        p_try_move(thinkers, level, players, rmain, validcount, mo, x + momx, y);
    }
}

/// Port of `PIT_ChangeSector`. Crunches/gibs/removes things that no
/// longer fit after a sector's height changed. `nofit`/`crushchange` are
/// the original's own module statics — [`p_change_sector`] threads them
/// through explicitly instead (same convention as every other Phase 7
/// mover's state in this port).
#[allow(clippy::too_many_arguments)]
fn pit_change_sector(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    leveltime: i32,
    crushchange: bool,
    nofit: &mut bool,
    id: ThinkerId,
) -> bool {
    if p_thing_height_clip(thinkers, level, players, rmain, validcount, id) {
        return true; // keep checking (the original's own comment, preserved)
    }

    let thing = *thinkers.mobj(id).unwrap();

    // crunch bodies to giblets (the original's own comment, preserved).
    if thing.health <= 0 {
        crate::p_mobj::p_set_mobj_state(
            thinkers,
            level,
            id,
            crate::info::StateNum::SGibs,
            |_, _, _, _| {},
        );
        let mobj = thinkers.mobj_mut(id).unwrap();
        mobj.flags &= !mobj_flag::SOLID;
        mobj.height = 0;
        mobj.radius = 0;
        return true; // keep checking
    }

    // crunch dropped items (the original's own comment, preserved).
    if thing.flags & mobj_flag::DROPPED != 0 {
        crate::p_mobj::p_remove_mobj(thinkers, level, id);
        return true; // keep checking
    }

    if thing.flags & mobj_flag::SHOOTABLE == 0 {
        // assume it is bloody gibs or something (the original's own
        // comment, preserved).
        return true;
    }

    *nofit = true;

    if crushchange && leveltime & 3 == 0 {
        crate::p_inter::p_damage_mobj(thinkers, level, players, rmain, id, None, None, 10);

        // spray blood in a random direction (the original's own
        // comment, preserved).
        let thing = *thinkers.mobj(id).unwrap();
        let mo = crate::p_mobj::p_spawn_mobj(
            thinkers,
            level,
            thing.x,
            thing.y,
            crate::p_mobj::SpawnZ::At(thing.z + thing.height / 2),
            crate::info::MobjType::MtBlood,
        );
        let mobj = thinkers.mobj_mut(mo).unwrap();
        mobj.momx = (crate::m_random::p_random() - crate::m_random::p_random()) << 12;
        mobj.momy = (crate::m_random::p_random() - crate::m_random::p_random()) << 12;
    }

    // keep checking (crush other things) (the original's own comment,
    // preserved).
    true
}

/// Port of `P_ChangeSector`. After modifying a sectors floor or ceiling
/// height, call this routine to adjust the positions of all things that
/// touch the sector. If anything doesn't fit anymore, `true` will be
/// returned. If `crunch` is true, they will take damage as they are
/// being crushed. If crunch is false, you should set the sector height
/// back the way it was and call `P_ChangeSector` again to undo the
/// changes (the original's own comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn p_change_sector(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    leveltime: i32,
    sector: usize,
    crunch: bool,
) -> bool {
    let mut nofit = false;
    let blockbox = level.sectors[sector].blockbox;

    // re-check heights for all things near the moving sector (the
    // original's own comment, preserved). `p_block_things_iterator`
    // only needs `&Level` (it walks `blocklinks`, never mutates it), but
    // `pit_change_sector` needs `&mut Level` (spawning blood, removing
    // mobjs, ...) — so every id in range is collected first under a
    // shared borrow, then processed under a unique one. This can visit
    // an id `PIT_ChangeSector` would have removed from the block chain
    // by the time a later block gets to it (e.g. `P_RemoveMobj`
    // unlinking it): [`Thinkers::mobj`]'s stale-id-is-`None` handling
    // (see `p_tick`'s docs) makes that a no-op here, matching the
    // original's own behavior of only ever unlinking forward through
    // the chain it's actively walking.
    let mut ids: Vec<ThinkerId> = Vec::new();
    for x in blockbox[BOXLEFT]..=blockbox[BOXRIGHT] {
        for y in blockbox[BOXBOTTOM]..=blockbox[BOXTOP] {
            p_block_things_iterator(thinkers, level, x, y, |_, id| {
                ids.push(id);
                true
            });
        }
    }

    for id in ids {
        if thinkers.mobj(id).is_none() {
            continue; // already removed by an earlier id in this pass
        }
        pit_change_sector(
            thinkers, level, players, rmain, validcount, leveltime, crunch, &mut nofit, id,
        );
    }

    nofit
}

/// (`MELEERANGE`, `p_enemy.c`'s own `#define`). Duplicated from
/// `p_enemy.rs`'s private copy — see `p_mobj.rs`'s `FLOATSPEED` comment
/// on why there's no shared `p_local`-equivalent module in this port.
pub const MELEERANGE: Fixed = 64 * FRACUNIT;

/// The original's file-scope scratch globals for one
/// [`p_aim_line_attack`]/[`p_line_attack`] call (`shootthing`/`shootz`/
/// `la_damage`/`attackrange`/`aimslope`/`topslope`/`bottomslope`/
/// `linetarget`), bundled together — same convention as `p_sight.rs`'s
/// `SightTrace`.
struct AimContext {
    shootthing: ThinkerId,
    shootz: Fixed,
    attackrange: Fixed,
    topslope: Fixed,
    bottomslope: Fixed,
    aimslope: Fixed,
    linetarget: Option<ThinkerId>,
}

/// Port of `PTR_AimTraverse`. Sets `ctx.linetarget`/`ctx.aimslope` when a
/// target is aimed at (the original's own comment, preserved). Returns
/// `false` to stop the traverse, matching the original's boolean.
fn ptr_aim_traverse(
    ctx: &mut AimContext,
    thinkers: &Thinkers,
    level: &Level,
    intercept: &Intercept,
) -> bool {
    match intercept.target {
        InterceptTarget::Line(line_idx) => {
            let line = &level.lines[line_idx];
            if line.flags & crate::doomdata::ML_TWOSIDED == 0 {
                return false; // stop
            }

            // Crosses a two sided line. A two sided line will restrict
            // the possible target ranges (the original's own comment,
            // preserved).
            let opening = p_line_opening(level, line);
            if opening.openbottom >= opening.opentop {
                return false; // stop
            }

            let dist = fixed_mul(ctx.attackrange, intercept.frac);

            let front = &level.sectors[line.frontsector.unwrap()];
            let back = &level.sectors[line.backsector.unwrap()];

            if front.floorheight != back.floorheight {
                let slope = crate::m_fixed::fixed_div(opening.openbottom - ctx.shootz, dist);
                if slope > ctx.bottomslope {
                    ctx.bottomslope = slope;
                }
            }

            if front.ceilingheight != back.ceilingheight {
                let slope = crate::m_fixed::fixed_div(opening.opentop - ctx.shootz, dist);
                if slope < ctx.topslope {
                    ctx.topslope = slope;
                }
            }

            if ctx.topslope <= ctx.bottomslope {
                return false; // stop
            }

            true // shot continues
        }
        InterceptTarget::Thing(th) => {
            if th == ctx.shootthing {
                return true; // can't shoot self
            }
            let thing = thinkers.mobj(th).unwrap();
            if thing.flags & mobj_flag::SHOOTABLE == 0 {
                return true; // corpse or something
            }

            // check angles to see if the thing can be aimed at (the
            // original's own comment, preserved).
            let dist = fixed_mul(ctx.attackrange, intercept.frac);
            let thingtopslope =
                crate::m_fixed::fixed_div(thing.z + thing.height - ctx.shootz, dist);
            if thingtopslope < ctx.bottomslope {
                return true; // shot over the thing
            }

            let mut thingtopslope = thingtopslope;
            let thingbottomslope = crate::m_fixed::fixed_div(thing.z - ctx.shootz, dist);
            if thingbottomslope > ctx.topslope {
                return true; // shot under the thing
            }

            // this thing can be hit! (the original's own comment,
            // preserved).
            if thingtopslope > ctx.topslope {
                thingtopslope = ctx.topslope;
            }
            let mut thingbottomslope = thingbottomslope;
            if thingbottomslope < ctx.bottomslope {
                thingbottomslope = ctx.bottomslope;
            }

            ctx.aimslope = (thingtopslope + thingbottomslope) / 2;
            ctx.linetarget = Some(th);

            false // don't go any farther
        }
    }
}

/// Port of `P_AimLineAttack`. Returns the aim slope, and `linetarget`
/// (`None` if nothing was hit) — the original's own `linetarget` global,
/// threaded back as an explicit return value here (see module docs).
pub fn p_aim_line_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    validcount: i32,
    t1: ThinkerId,
    angle: crate::tables::Angle,
    distance: Fixed,
) -> (Fixed, Option<ThinkerId>) {
    let an = (angle >> ANGLETOFINESHIFT) as usize;
    let (x1, y1, z1, height1) = {
        let mo = thinkers.mobj(t1).unwrap();
        (mo.x, mo.y, mo.z, mo.height)
    };

    let x2 = x1 + (distance >> crate::m_fixed::FRACBITS) * fine_cosine(an);
    let y2 = y1 + (distance >> crate::m_fixed::FRACBITS) * FINESINE[an];

    let mut ctx = AimContext {
        shootthing: t1,
        shootz: z1 + (height1 >> 1) + 8 * FRACUNIT,
        attackrange: distance,
        // can't shoot outside view angles (the original's own comment,
        // preserved).
        topslope: 100 * FRACUNIT / 160,
        bottomslope: -100 * FRACUNIT / 160,
        aimslope: 0,
        linetarget: None,
    };

    p_path_traverse(
        level,
        thinkers,
        validcount,
        x1,
        y1,
        x2,
        y2,
        PT_ADDLINES | crate::p_maputl::PT_ADDTHINGS,
        |level, thinkers, intercept| ptr_aim_traverse(&mut ctx, thinkers, level, intercept),
    );

    if ctx.linetarget.is_some() {
        (ctx.aimslope, ctx.linetarget)
    } else {
        (0, None)
    }
}

/// What [`ptr_shoot_traverse`] decided to do, applied after the
/// traverse finishes — see module docs on why (the closure can't mutate
/// `Thinkers`/`Level`/players itself).
enum ShootHit {
    /// Hit a line at `(x, y, z)` (`None` = no puff, the sky case).
    Line {
        puff_at: Option<(Fixed, Fixed, Fixed)>,
    },
    /// Hit a shootable thing at `(x, y, z)`.
    Thing {
        target: ThinkerId,
        x: Fixed,
        y: Fixed,
        z: Fixed,
        no_blood: bool,
    },
}

/// The original's file-scope scratch globals `PTR_ShootTraverse` reads
/// (`shootthing`/`shootz`/`la_damage`/`attackrange`/`aimslope`, plus
/// `trace`'s `x`/`y`/`dx`/`dy` — `p_path_traverse` doesn't expose its
/// own `trace` to the callback, so this port passes the same `x1,y1,
/// x2,y2` the caller used and recomputes `dx`/`dy` from them, exactly
/// equal to the original's `trace`).
struct ShootContext {
    shootthing: ThinkerId,
    shootz: Fixed,
    la_damage: i32,
    attackrange: Fixed,
    aimslope: Fixed,
    trace_x: Fixed,
    trace_y: Fixed,
    trace_dx: Fixed,
    trace_dy: Fixed,
    skyflatnum: i32,
    hit: Option<ShootHit>,
    /// Every special line the shot crossed or hit, in order — the
    /// original calls `P_ShootSpecialLine` for each as it goes.
    specials: Vec<usize>,
}

/// Port of `PTR_ShootTraverse`.
fn ptr_shoot_traverse(
    ctx: &mut ShootContext,
    thinkers: &Thinkers,
    level: &Level,
    intercept: &Intercept,
) -> bool {
    match intercept.target {
        InterceptTarget::Line(line_idx) => {
            let line = &level.lines[line_idx];
            if line.special != 0 {
                ctx.specials.push(line_idx);
            }

            let mut hitline = line.flags & crate::doomdata::ML_TWOSIDED == 0;
            if !hitline {
                // crosses a two sided line (the original's own comment,
                // preserved).
                let opening = p_line_opening(level, line);
                let dist = fixed_mul(ctx.attackrange, intercept.frac);
                let front = &level.sectors[line.frontsector.unwrap()];
                let back = &level.sectors[line.backsector.unwrap()];

                if front.floorheight != back.floorheight {
                    let slope = crate::m_fixed::fixed_div(opening.openbottom - ctx.shootz, dist);
                    if slope > ctx.aimslope {
                        hitline = true;
                    }
                }
                if !hitline && front.ceilingheight != back.ceilingheight {
                    let slope = crate::m_fixed::fixed_div(opening.opentop - ctx.shootz, dist);
                    if slope < ctx.aimslope {
                        hitline = true;
                    }
                }
            }

            if !hitline {
                return true; // shot continues
            }

            // position a bit closer (the original's own comment,
            // preserved).
            let frac = intercept.frac - crate::m_fixed::fixed_div(4 * FRACUNIT, ctx.attackrange);
            let x = ctx.trace_x + fixed_mul(ctx.trace_dx, frac);
            let y = ctx.trace_y + fixed_mul(ctx.trace_dy, frac);
            let z = ctx.shootz + fixed_mul(ctx.aimslope, fixed_mul(frac, ctx.attackrange));

            let front = &level.sectors[line.frontsector.unwrap()];
            let puff_at = Some((x, y, z));
            if front.ceilingpic as i32 == ctx.skyflatnum {
                // don't shoot the sky! (the original's own comment,
                // preserved).
                if z > front.ceilingheight {
                    ctx.hit = Some(ShootHit::Line { puff_at: None });
                    return false;
                }
                // it's a sky hack wall (the original's own comment,
                // preserved).
                if let Some(back_idx) = line.backsector {
                    if level.sectors[back_idx].ceilingpic as i32 == ctx.skyflatnum {
                        ctx.hit = Some(ShootHit::Line { puff_at: None });
                        return false;
                    }
                }
            }

            ctx.hit = Some(ShootHit::Line { puff_at });
            false // don't go any farther
        }
        InterceptTarget::Thing(th) => {
            if th == ctx.shootthing {
                return true; // can't shoot self
            }
            let thing = thinkers.mobj(th).unwrap();
            if thing.flags & mobj_flag::SHOOTABLE == 0 {
                return true; // corpse or something
            }

            // check angles to see if the thing can be aimed at (the
            // original's own comment, preserved).
            let dist = fixed_mul(ctx.attackrange, intercept.frac);
            let thingtopslope =
                crate::m_fixed::fixed_div(thing.z + thing.height - ctx.shootz, dist);
            if thingtopslope < ctx.aimslope {
                return true; // shot over the thing
            }
            let thingbottomslope = crate::m_fixed::fixed_div(thing.z - ctx.shootz, dist);
            if thingbottomslope > ctx.aimslope {
                return true; // shot under the thing
            }

            // hit thing, position a bit closer (the original's own
            // comment, preserved).
            let frac = intercept.frac - crate::m_fixed::fixed_div(10 * FRACUNIT, ctx.attackrange);
            let x = ctx.trace_x + fixed_mul(ctx.trace_dx, frac);
            let y = ctx.trace_y + fixed_mul(ctx.trace_dy, frac);
            let z = ctx.shootz + fixed_mul(ctx.aimslope, fixed_mul(frac, ctx.attackrange));

            ctx.hit = Some(ShootHit::Thing {
                target: th,
                x,
                y,
                z,
                no_blood: thing.flags & mobj_flag::NOBLOOD != 0,
            });
            false // don't go any farther
        }
    }
}

/// Port of `P_LineAttack`. If `damage == 0`, it is just a test trace
/// that will leave `linetarget` set (the original's own comment,
/// preserved — here, nothing reads a `linetarget` after this call since
/// [`p_aim_line_attack`] already returned it to the caller).
///
/// Returns every special line the shot crossed or hit (in order); the
/// original calls `P_ShootSpecialLine` for each right inside
/// `PTR_ShootTraverse`, but that needs a whole
/// [`crate::p_spec::SpecialsCtx`], which this function's callers can't
/// all provide (it would alias the `thinkers`/`level`/... arguments). A
/// caller that owns one runs [`crate::p_spec::p_shoot_special_line`] on
/// each returned line afterwards (the player's attacks do; monsters'
/// shots, as before, don't — the same gap `p_move`'s `spechit` has).
#[allow(clippy::too_many_arguments)]
pub fn p_line_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    t1: ThinkerId,
    angle: crate::tables::Angle,
    distance: Fixed,
    slope: Fixed,
    damage: i32,
) -> Vec<usize> {
    let an = (angle >> ANGLETOFINESHIFT) as usize;
    let (x1, y1, z1, height1) = {
        let mo = thinkers.mobj(t1).unwrap();
        (mo.x, mo.y, mo.z, mo.height)
    };
    let x2 = x1 + (distance >> crate::m_fixed::FRACBITS) * fine_cosine(an);
    let y2 = y1 + (distance >> crate::m_fixed::FRACBITS) * FINESINE[an];

    let mut ctx = ShootContext {
        shootthing: t1,
        shootz: z1 + (height1 >> 1) + 8 * FRACUNIT,
        la_damage: damage,
        attackrange: distance,
        aimslope: slope,
        trace_x: x1,
        trace_y: y1,
        trace_dx: x2 - x1,
        trace_dy: y2 - y1,
        skyflatnum: crate::doomstat::state().skyflatnum,
        hit: None,
        specials: Vec::new(),
    };

    p_path_traverse(
        level,
        thinkers,
        validcount,
        x1,
        y1,
        x2,
        y2,
        PT_ADDLINES | crate::p_maputl::PT_ADDTHINGS,
        |level, thinkers, intercept| ptr_shoot_traverse(&mut ctx, thinkers, level, intercept),
    );

    match ctx.hit {
        Some(ShootHit::Line {
            puff_at: Some((x, y, z)),
        }) => {
            crate::p_mobj::p_spawn_puff(thinkers, level, distance, x, y, z);
        }
        Some(ShootHit::Line { puff_at: None }) => {}
        Some(ShootHit::Thing {
            target,
            x,
            y,
            z,
            no_blood,
        }) => {
            if no_blood {
                crate::p_mobj::p_spawn_puff(thinkers, level, distance, x, y, z);
            } else {
                crate::p_mobj::p_spawn_blood(thinkers, level, x, y, z, damage);
            }
            if damage != 0 {
                crate::p_inter::p_damage_mobj(
                    thinkers,
                    level,
                    players,
                    rmain,
                    target,
                    Some(t1),
                    Some(t1),
                    damage,
                );
            }
        }
        None => {}
    }

    ctx.specials
}

/// The original's file-scope scratch globals for one [`p_radius_attack`]
/// call (`bombspot`/`bombsource`/`bombdamage`).
struct RadiusContext {
    bombspot: ThinkerId,
    bombsource: Option<ThinkerId>,
    bombdamage: i32,
}

/// Port of `PIT_RadiusAttack`. `bombsource` is the creature that caused
/// the explosion at `bombspot` (the original's own comment, preserved).
#[allow(clippy::too_many_arguments)]
fn pit_radius_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    ctx: &RadiusContext,
    thing: ThinkerId,
) {
    let (flags, mobj_type, x, y, radius) = {
        let t = thinkers.mobj(thing).unwrap();
        (t.flags, t.mobj_type, t.x, t.y, t.radius)
    };
    if flags & mobj_flag::SHOOTABLE == 0 {
        return;
    }

    // Boss spider and cyborg take no damage from concussion (the
    // original's own comment, preserved).
    if matches!(
        mobj_type,
        crate::info::MobjType::MtCyborg | crate::info::MobjType::MtSpider
    ) {
        return;
    }

    let (bombx, bomby) = {
        let b = thinkers.mobj(ctx.bombspot).unwrap();
        (b.x, b.y)
    };
    let dx = (x - bombx).wrapping_abs();
    let dy = (y - bomby).wrapping_abs();
    let dist = dx.max(dy);
    let dist = ((dist - radius) >> crate::m_fixed::FRACBITS).max(0);

    if dist >= ctx.bombdamage {
        return; // out of range
    }

    let (sight, _validcount) =
        crate::p_sight::p_check_sight(thinkers, level, validcount, thing, ctx.bombspot);
    if sight {
        // must be in direct path (the original's own comment,
        // preserved).
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            thing,
            Some(ctx.bombspot),
            ctx.bombsource,
            ctx.bombdamage - dist,
        );
    }
}

/// Port of `P_RadiusAttack`. `source` is the creature that caused the
/// explosion at `spot` (the original's own comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn p_radius_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    spot: ThinkerId,
    source: Option<ThinkerId>,
    damage: i32,
) {
    let (spot_x, spot_y) = {
        let s = thinkers.mobj(spot).unwrap();
        (s.x, s.y)
    };
    let dist = (damage + MAXRADIUS) << crate::m_fixed::FRACBITS;
    let yh = (spot_y + dist - level.bmaporgy) >> MAPBLOCKSHIFT;
    let yl = (spot_y - dist - level.bmaporgy) >> MAPBLOCKSHIFT;
    let xh = (spot_x + dist - level.bmaporgx) >> MAPBLOCKSHIFT;
    let xl = (spot_x - dist - level.bmaporgx) >> MAPBLOCKSHIFT;

    let ctx = RadiusContext {
        bombspot: spot,
        bombsource: source,
        bombdamage: damage,
    };

    for y in yl..=yh {
        for x in xl..=xh {
            let mut ids: Vec<ThinkerId> = Vec::new();
            p_block_things_iterator(thinkers, level, x, y, |_, id| {
                ids.push(id);
                true
            });
            for id in ids {
                if thinkers.mobj(id).is_none() {
                    continue; // already removed earlier in this pass
                }
                pit_radius_attack(thinkers, level, players, rmain, validcount, &ctx, id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::MobjType;
    use crate::r_defs::Mobj;
    use crate::w_wad::WadFiles;

    fn load_e1m1() -> Option<(WadFiles, Level)> {
        let candidates = ["doom.wad"];
        let path = candidates
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut wad = WadFiles::new();
        wad.init_file(&path);
        let level = Level::load(&mut wad, "E1M1");
        Some((wad, level))
    }

    /// Player 1's start, from E1M1 (see `tests/real_frame.rs`).
    const START_X: Fixed = 1056 * FRACUNIT;
    const START_Y: Fixed = -3616 * FRACUNIT;

    #[test]
    fn try_move_a_short_distance_in_open_space_succeeds() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let id = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = RMain::new();
        let (moved, _spechit) = p_try_move(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            1,
            id,
            START_X + 10 * FRACUNIT,
            START_Y,
        );
        assert!(moved, "a short move in open space should succeed");
        assert_eq!(thinkers.mobj(id).unwrap().x, START_X + 10 * FRACUNIT);
    }

    #[test]
    fn try_move_through_a_solid_one_sided_wall_fails() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let id = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        // Push far enough in every direction that at least one hits a
        // one-sided wall in a reasonably sized room; the point of this
        // test is just that P_TryMove can and does reject a move (not
        // which direction it is), so try all four and require at least
        // one failure. `far` (10,000 map units) is picked to stay clear
        // of i32 overflow in every direction from `START_X`/`START_Y` —
        // the previous `20_000` overflowed `START_X - far` (E1M1's
        // player start is already ~13,856 units into negative x), which
        // is a bug in this test's constant, not in `P_TryMove`/
        // `P_CheckPosition` (nothing in the engine itself is anywhere
        // near this coordinate range on a real map).
        let far = 10_000 * FRACUNIT;
        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = RMain::new();
        let blocked = [(far, 0), (-far, 0), (0, far), (0, -far)]
            .into_iter()
            .any(|(dx, dy)| {
                !p_try_move(
                    &mut thinkers,
                    &mut level,
                    &mut players,
                    &mut rmain,
                    1,
                    id,
                    START_X + dx,
                    START_Y + dy,
                )
                .0
            });
        assert!(
            blocked,
            "moving far enough should hit a solid wall somewhere"
        );
    }

    #[test]
    fn check_position_reports_floor_and_ceiling_of_the_starting_sector() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let id = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = RMain::new();
        let (ok, ctx) = p_check_position(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            1,
            id,
            START_X,
            START_Y,
        );
        assert!(ok);
        let ss = RMain::point_in_subsector(&level, START_X, START_Y);
        let sector = level.subsectors[ss].sector;
        assert_eq!(ctx.floorz, level.sectors[sector].floorheight);
        assert_eq!(ctx.ceilingz, level.sectors[sector].ceilingheight);
    }

    #[test]
    fn thing_height_clip_keeps_a_floor_walker_on_the_floor() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let id = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = RMain::new();
        let fits = p_thing_height_clip(&mut thinkers, &mut level, &mut players, &mut rmain, 1, id);
        assert!(fits);
        let mobj = thinkers.mobj(id).unwrap();
        assert_eq!(mobj.z, mobj.floorz);
    }

    #[test]
    fn slide_move_along_a_wall_does_not_panic_and_keeps_the_mobj_in_the_level() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let id = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        {
            let mobj: &mut Mobj = thinkers.mobj_mut(id).unwrap();
            // Aim at a wall hard enough that a straight P_TryMove would
            // fail and P_SlideMove has to do actual sliding work.
            mobj.momx = 200 * FRACUNIT;
            mobj.momy = 0;
        }

        let mut players: Vec<crate::d_player::Player> = Vec::new();
        p_slide_move(&mut thinkers, &mut level, &mut players, &mut rmain, 1, id);

        // No panic, and the mobj is still a real position inside/near
        // the level (loose bound — this is a smoke test for the sliding
        // machinery, not a specific expected stopping point).
        let mobj = thinkers.mobj(id).unwrap();
        assert!(mobj.x.abs() < i32::MAX / 2);
        assert!(mobj.y.abs() < i32::MAX / 2);
    }

    #[test]
    fn change_sector_reports_fit_when_nothing_is_crushed() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let id = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let sector = level.subsectors[thinkers.mobj(id).unwrap().subsector.unwrap()].sector;

        let nofit = p_change_sector(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            1,
            0,
            sector,
            false,
        );

        assert!(
            !nofit,
            "a mobj standing on its own floor shouldn't get crushed by its own sector"
        );
    }

    #[test]
    fn aim_line_attack_finds_a_nearby_shootable_target() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let shooter = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let target = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 96 * FRACUNIT,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        // Due east, angle 0.
        let (slope, linetarget) =
            p_aim_line_attack(&mut thinkers, &mut level, 0, shooter, 0, 16 * 64 * FRACUNIT);

        assert_eq!(linetarget, Some(target));
        let _ = slope;
    }

    #[test]
    fn aim_line_attack_finds_nothing_when_alone() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let shooter = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let (slope, linetarget) =
            p_aim_line_attack(&mut thinkers, &mut level, 0, shooter, 0, 16 * 64 * FRACUNIT);

        assert_eq!(linetarget, None);
        assert_eq!(slope, 0);
    }

    #[test]
    fn line_attack_damages_a_target_in_the_line_of_fire() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let shooter = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let target = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 96 * FRACUNIT,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut players = [crate::d_player::Player::for_test(target)];
        thinkers.mobj_mut(target).unwrap().player = Some(0);
        let health_before = players[0].health;

        p_line_attack(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            shooter,
            0,
            16 * 64 * FRACUNIT,
            0,
            10,
        );

        assert!(
            players[0].health < health_before,
            "a hitscan attack in the target's direction should damage it"
        );
    }

    #[test]
    fn line_attack_with_zero_damage_only_test_traces() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let shooter = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let target = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 96 * FRACUNIT,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut players = [crate::d_player::Player::for_test(target)];
        let health_before = players[0].health;

        p_line_attack(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            shooter,
            0,
            16 * 64 * FRACUNIT,
            0,
            0,
        );

        assert_eq!(
            players[0].health, health_before,
            "damage == 0 is just a test trace, it must not actually hurt anything"
        );
    }

    #[test]
    fn radius_attack_damages_nearby_shootable_things_but_not_the_spot_itself_type() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let spot = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let nearby = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 40 * FRACUNIT,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut players = [
            crate::d_player::Player::for_test(spot),
            crate::d_player::Player::for_test(nearby),
        ];
        thinkers.mobj_mut(spot).unwrap().player = Some(0);
        thinkers.mobj_mut(nearby).unwrap().player = Some(1);
        let nearby_health_before = players[1].health;

        p_radius_attack(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            spot,
            Some(spot),
            128,
        );

        assert!(
            players[1].health < nearby_health_before,
            "a nearby shootable thing within blast radius and sight should take damage"
        );
    }

    #[test]
    fn radius_attack_spares_things_out_of_range() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let spot = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let far_away = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 2000 * FRACUNIT,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut players = [
            crate::d_player::Player::for_test(spot),
            crate::d_player::Player::for_test(far_away),
        ];
        thinkers.mobj_mut(spot).unwrap().player = Some(0);
        thinkers.mobj_mut(far_away).unwrap().player = Some(1);
        let far_health_before = players[1].health;

        p_radius_attack(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            spot,
            Some(spot),
            64,
        );

        assert_eq!(
            players[1].health, far_health_before,
            "a thing far outside the blast radius must take no damage"
        );
    }

    #[test]
    fn repeated_position_checks_in_one_tic_all_see_the_wall() {
        // The bug this pins: `validcount` used to be bumped once per
        // tic, so the second check at a wall skipped the line the first
        // check had already marked and let the mobj through.
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mo = crate::p_mobj::p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            crate::p_mobj::SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = RMain::new();
        // E1M1's start room has a thin wall across x=1056 at y=-2879.
        let y = -2880 * FRACUNIT;
        for attempt in 0..4 {
            let (ok, _) = p_check_position(
                &mut thinkers,
                &mut level,
                &mut players,
                &mut rmain,
                7, // the same value every time, like a per-tic counter
                mo,
                START_X,
                y,
            );
            assert!(!ok, "attempt {attempt}: the wall must block every time");
        }
    }
}
