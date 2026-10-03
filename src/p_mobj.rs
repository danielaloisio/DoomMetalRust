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
//	Moving object handling. Spawn functions.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_mobj.h` / `p_mobj.c` (partial, see note below).
//!
//! # Scope
//!
//! Ported: [`p_spawn_mobj`] (`P_SpawnMobj`), [`p_set_mobj_state`]
//! (`P_SetMobjState`), [`p_remove_mobj`] (`P_RemoveMobj`, minus the
//! item-respawn queue — deathmatch-only, later phase),
//! [`p_set_thing_position`]/[`p_unset_thing_position`]
//! (`P_SetThingPosition`/`P_UnsetThingPosition`, from `p_maputl.c`, kept
//! here since they only make sense alongside the mobj they link/unlink).
//!
//! Not ported yet (later 6-substages/Phase 7): `P_MobjThinker`'s
//! momentum/Z movement branches (`P_XYMovement`/`P_ZMovement`, 6c);
//! nightmare respawn, `P_RespawnSpecials`, `P_SpawnPlayer`,
//! `P_SpawnMapThing` (`p_setup.rs` calls belong to this sub-stage too,
//! see there).
//!
//! As of Phase 7d1: [`p_spawn_puff`]/[`p_spawn_blood`]
//! (`P_SpawnPuff`/`P_SpawnBlood`), [`p_check_missile_spawn`]
//! (`P_CheckMissileSpawn`), and [`p_spawn_missile`]/
//! [`p_spawn_player_missile`] (`P_SpawnMissile`/`P_SpawnPlayerMissile`)
//! are ported — the missile-spawning half of Phase 7c/7d's shooting
//! infrastructure. `P_SpawnPlayerMissile`'s `linetarget` read (set by
//! `crate::p_map::p_aim_line_attack`, another file-scope scratch global
//! in the original) is threaded here as that function's own return
//! value instead — see `p_map.rs`'s module docs on `AimContext`.
//!
//! # Action functions aren't dispatched yet
//!
//! [`p_set_mobj_state`] runs the same `do { ... } while (!tics)` loop as
//! the original, but the `st->action.acp1(mobj)` call is a deliberate
//! no-op for any [`crate::info::StateAction`] other than `None` — the 74
//! `A_*` action functions (`p_enemy.c`, `p_pspr.c`, ...) are later-phase
//! work (mostly Phase 7). This only matters once code sets a state whose
//! action is non-`None`; every mobj [`crate::p_setup`] spawns statically
//! from `E1M1`'s THINGS lump starts (and, before ticking, stays) in its
//! `spawnstate`, which is `StateAction::None` for every mobj type this
//! test map uses, so today's callers never hit the gap — but it *is* a
//! tracked divergence, not a silent one: [`p_set_mobj_state`] documents
//! exactly which states/mobjs it would matter for.

use crate::info::{StateAction, MOBJINFO, STATES};
use crate::m_fixed::{fixed_mul, Fixed, FRACBITS};
use crate::m_random::p_random;
use crate::p_setup::{Level, MAPBLOCKSHIFT};
use crate::p_tick::{ThinkFn, ThinkerData, ThinkerId, Thinkers};
use crate::r_defs::{mobj_flag, Mobj};
use crate::r_main::RMain;

/// Where to place a newly spawned mobj on the Z axis — stands in for the
/// original's `ONFLOORZ`/`ONCEILINGZ` sentinel values (`MININT`/`MAXINT`
/// passed as an ordinary `fixed_t z` to `P_SpawnMobj`, then compared
/// against). An explicit enum is clearer than reusing those magic
/// constants and just as faithful, since `P_SpawnMobj` only ever
/// branches on "is `z` one of these two sentinels, or a real height".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnZ {
    OnFloor,
    OnCeiling,
    At(Fixed),
}

/// Port of `P_SetThingPosition`. Links `mobj` into its subsector's
/// sector `thinglist` and its blockmap cell, matching the original
/// exactly (head-inserted chains) but through [`ThinkerId`]s instead of
/// raw pointers.
///
/// `id` must already be a live mobj thinker in `thinkers` (its position
/// fields are read to compute the subsector/blockmap cell).
pub fn p_set_thing_position(thinkers: &mut Thinkers, level: &mut Level, id: ThinkerId) {
    let (x, y, flags) = {
        let mobj = thinkers.mobj(id).expect("live mobj");
        (mobj.x, mobj.y, mobj.flags)
    };

    let ss = RMain::point_in_subsector(level, x, y);
    thinkers.mobj_mut(id).unwrap().subsector = Some(ss);

    if flags & crate::r_defs::mobj_flag::NOSECTOR == 0 {
        let sec = level.subsectors[ss].sector;
        let old_head = level.sectors[sec].thinglist;
        {
            let mobj = thinkers.mobj_mut(id).unwrap();
            mobj.sprev = None;
            mobj.snext = old_head;
        }
        if let Some(head) = old_head {
            thinkers.mobj_mut(head).unwrap().sprev = Some(id);
        }
        level.sectors[sec].thinglist = Some(id);
    }

    if flags & crate::r_defs::mobj_flag::NOBLOCKMAP == 0 {
        let blockx = (x - level.bmaporgx) >> MAPBLOCKSHIFT;
        let blocky = (y - level.bmaporgy) >> MAPBLOCKSHIFT;

        if blockx >= 0 && blockx < level.bmapwidth && blocky >= 0 && blocky < level.bmapheight {
            let cell = (blocky * level.bmapwidth + blockx) as usize;
            let old_head = level.blocklinks[cell];
            {
                let mobj = thinkers.mobj_mut(id).unwrap();
                mobj.bprev = None;
                mobj.bnext = old_head;
            }
            if let Some(head) = old_head {
                thinkers.mobj_mut(head).unwrap().bprev = Some(id);
            }
            level.blocklinks[cell] = Some(id);
        } else {
            // thing is off the map (the original's own comment,
            // preserved).
            let mobj = thinkers.mobj_mut(id).unwrap();
            mobj.bnext = None;
            mobj.bprev = None;
        }
    }
}

/// Port of `P_UnsetThingPosition`. Unlinks `mobj` from its sector
/// `thinglist` and blockmap cell.
pub fn p_unset_thing_position(thinkers: &mut Thinkers, level: &mut Level, id: ThinkerId) {
    let (flags, subsector, x, y, snext, sprev, bnext, bprev) = {
        let mobj = thinkers.mobj(id).expect("live mobj");
        (
            mobj.flags,
            mobj.subsector,
            mobj.x,
            mobj.y,
            mobj.snext,
            mobj.sprev,
            mobj.bnext,
            mobj.bprev,
        )
    };

    if flags & crate::r_defs::mobj_flag::NOSECTOR == 0 {
        if let Some(next) = snext {
            thinkers.mobj_mut(next).unwrap().sprev = sprev;
        }
        if let Some(prev) = sprev {
            thinkers.mobj_mut(prev).unwrap().snext = snext;
        } else {
            let sec = level.subsectors[subsector.expect("linked mobj has a subsector")].sector;
            level.sectors[sec].thinglist = snext;
        }
    }

    if flags & crate::r_defs::mobj_flag::NOBLOCKMAP == 0 {
        if let Some(next) = bnext {
            thinkers.mobj_mut(next).unwrap().bprev = bprev;
        }
        if let Some(prev) = bprev {
            thinkers.mobj_mut(prev).unwrap().bnext = bnext;
        } else {
            let blockx = (x - level.bmaporgx) >> MAPBLOCKSHIFT;
            let blocky = (y - level.bmaporgy) >> MAPBLOCKSHIFT;
            if blockx >= 0 && blockx < level.bmapwidth && blocky >= 0 && blocky < level.bmapheight {
                let cell = (blocky * level.bmapwidth + blockx) as usize;
                level.blocklinks[cell] = bnext;
            }
        }
    }
}

/// Port of `P_SpawnMobj`. Allocates the mobj (here: a new arena slot via
/// [`Thinkers::add_thinker`]), fills it from `mobjinfo[type]`/its
/// spawnstate, links it into the world with [`p_set_thing_position`],
/// resolves `z` (see [`SpawnZ`]), and adds it to the thinker list.
///
/// Returns the new mobj's id.
pub fn p_spawn_mobj(
    thinkers: &mut Thinkers,
    level: &mut Level,
    x: Fixed,
    y: Fixed,
    z: SpawnZ,
    mobj_type: crate::info::MobjType,
) -> ThinkerId {
    let info = &MOBJINFO[mobj_type as usize];
    let st = STATES[info.spawnstate as usize];

    // do not set the state with P_SetMobjState, because action routines
    // can not be called yet (the original's own comment, preserved).
    let mut mobj = Mobj::blank(mobj_type);
    mobj.x = x;
    mobj.y = y;
    mobj.radius = info.radius;
    mobj.height = info.height;
    mobj.flags = info.flags;
    mobj.health = info.spawnhealth;
    // gameskill != sk_nightmare check deferred: nightmare skill
    // selection isn't wired up yet (Phase 6d/g_game); every caller today
    // behaves like non-nightmare, matching the original's default.
    mobj.reactiontime = info.reactiontime;
    mobj.lastlook = p_random() % crate::doomdef::MAXPLAYERS;
    mobj.state = info.spawnstate;
    mobj.tics = st.tics;
    mobj.sprite = st.sprite;
    mobj.frame = st.frame;

    let id = thinkers.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(mobj));

    // set subsector and/or block links (the original's own comment,
    // preserved).
    p_set_thing_position(thinkers, level, id);

    let sector = level.subsectors[thinkers.mobj(id).unwrap().subsector.unwrap()].sector;
    let floorz = level.sectors[sector].floorheight;
    let ceilingz = level.sectors[sector].ceilingheight;

    let mobj = thinkers.mobj_mut(id).unwrap();
    mobj.floorz = floorz;
    mobj.ceilingz = ceilingz;
    mobj.z = match z {
        SpawnZ::OnFloor => floorz,
        SpawnZ::OnCeiling => ceilingz - mobj.info.height,
        SpawnZ::At(z) => z,
    };

    id
}

/// (`ITEMQUESIZE`)
pub const ITEMQUESIZE: usize = 128;

/// Port of `P_RemoveMobj`, minus the item-respawn queue (`itemrespawnque`
/// — deathmatch-only bookkeeping, later phase). Unlinks the mobj from
/// its sector/blockmap and marks its thinker slot removed (see
/// `p_tick`'s docs on deferred removal — the slot itself isn't freed
/// until the next `run_thinkers` sweep, matching the original's
/// `Z_Free` happening inside `P_RunThinkers`, not here).
pub fn p_remove_mobj(thinkers: &mut Thinkers, level: &mut Level, id: ThinkerId) {
    if let Some(m) = thinkers.mobj(id) {
        if m.flags & mobj_flag::SPECIAL != 0
            && m.flags & mobj_flag::DROPPED == 0
            && m.mobj_type != crate::info::MobjType::MtInv
            && m.mobj_type != crate::info::MobjType::MtIns
        {
            level
                .itemrespawnque
                .push_back((m.spawnpoint, crate::doomstat::state().leveltime));
            // lose one off the end? (the original's own comment,
            // preserved)
            if level.itemrespawnque.len() >= ITEMQUESIZE {
                level.itemrespawnque.pop_front();
            }
        }
    }
    p_unset_thing_position(thinkers, level, id);
    // stop any playing sound (the original's own comment, preserved)
    crate::s_sound::s_stop_sound(Some(crate::s_sound::SoundOrigin::Mobj(id)));
    thinkers.remove(id);
}

/// Port of `P_SetMobjState`. Returns `true` if the mobj is still present
/// (`false` if the state chain reached `S_NULL`, which removes it — the
/// original's `boolean` return, used by `P_MobjThinker` to bail out
/// after a self-removal).
///
/// `dispatch` stands in for the original's `st->action.acp1(mobj)` —
/// called once per state transition in the chain (a single call can walk
/// through several zero-tic states, each running its own action, exactly
/// like the original's `do { ... } while (!mobj->tics)` loop). Most
/// callers don't have a monster/combat context in scope and pass a
/// no-op (`|_, _, _, _| {}`); [`p_mobj_thinker`] is the one caller that
/// runs every tic with the full world in scope, so it's the only one
/// that dispatches for real (`AFall`/`AKeenDie`/`ALook`/`AChase`, Phase
/// 7c — the rest of the 74 `A_*` functions remain documented no-ops,
/// see module docs).
pub fn p_set_mobj_state(
    thinkers: &mut Thinkers,
    level: &mut Level,
    id: ThinkerId,
    mut state: crate::info::StateNum,
    mut dispatch: impl FnMut(&mut Thinkers, &mut Level, ThinkerId, StateAction),
) -> bool {
    loop {
        if state == crate::info::StateNum::SNull {
            thinkers.mobj_mut(id).unwrap().state = crate::info::StateNum::SNull;
            p_remove_mobj(thinkers, level, id);
            return false;
        }

        let st = STATES[state as usize];
        {
            let mobj = thinkers.mobj_mut(id).unwrap();
            mobj.state = state;
            mobj.tics = st.tics;
            mobj.sprite = st.sprite;
            mobj.frame = st.frame;
        }

        // Modified handling. Call action functions when the state is
        // set (the original's own comment, preserved).
        if !matches!(st.action, StateAction::None) {
            dispatch(thinkers, level, id, st.action);
            if !thinkers.is_live(id) {
                // the action removed the mobj itself (e.g. a future
                // A_* could call P_RemoveMobj) — nothing this phase's
                // dispatched actions do, but guard it like the
                // original implicitly would (acp1 running arbitrary
                // code before the loop reads `mobj->tics` again).
                return false;
            }
        }

        state = st.nextstate;

        let tics = thinkers.mobj(id).unwrap().tics;
        if tics != 0 {
            break;
        }
    }
    true
}

/// (`STOPSPEED`, `p_mobj.c`'s own `#define`).
const STOPSPEED: Fixed = 0x1000;
/// (`FRICTION`, `p_mobj.c`'s own `#define`).
const FRICTION: Fixed = 0xe800;
/// (`MAXMOVE`, `p_local.h`).
const MAXMOVE: Fixed = 30 * crate::m_fixed::FRACUNIT;
/// (`GRAVITY`, `p_local.h`).
const GRAVITY: Fixed = crate::m_fixed::FRACUNIT;
/// (`FLOATSPEED`, `p_local.h`).
const FLOATSPEED: Fixed = crate::m_fixed::FRACUNIT * 4;
/// (`VIEWHEIGHT`, `p_local.h`). The original defines this once in a
/// shared header; this port has no `p_local`-equivalent module both
/// `p_mobj.rs` and `p_user.rs` pull from, so it's duplicated (same
/// value) at both call sites — see `p_user::VIEWHEIGHT`.
const VIEWHEIGHT: Fixed = 41 * crate::m_fixed::FRACUNIT;

/// Port of `P_ExplodeMissile`.
pub fn p_explode_missile(thinkers: &mut Thinkers, level: &mut Level, id: ThinkerId) {
    {
        let mobj = thinkers.mobj_mut(id).unwrap();
        mobj.momx = 0;
        mobj.momy = 0;
        mobj.momz = 0;
    }

    let deathstate = thinkers.mobj(id).unwrap().info.deathstate;
    p_set_mobj_state(thinkers, level, id, deathstate, |_, _, _, _| {});

    let mobj = thinkers.mobj_mut(id).unwrap();
    mobj.tics -= p_random() & 3;
    if mobj.tics < 1 {
        mobj.tics = 1;
    }
    mobj.flags &= !crate::r_defs::mobj_flag::MISSILE;

    let deathsound = mobj.info.deathsound;
    if !matches!(deathsound, crate::sounds::Sfx::SfxNone) {
        crate::s_sound::s_start_sound(Some(crate::s_sound::SoundOrigin::Mobj(id)), deathsound);
    }
}

/// Port of `P_SpawnPuff`.
/// `attackrange` stands in for the original's file-scope
/// `attackrange` global (see `p_map.rs`'s `p_line_attack` docs) — passed
/// explicitly since this function doesn't otherwise need any of
/// `p_map.rs`'s other shooting state.
pub fn p_spawn_puff(
    thinkers: &mut Thinkers,
    level: &mut Level,
    attackrange: Fixed,
    x: Fixed,
    y: Fixed,
    z: Fixed,
) -> ThinkerId {
    let z = z + ((p_random() - p_random()) << 10);

    let th = p_spawn_mobj(
        thinkers,
        level,
        x,
        y,
        SpawnZ::At(z),
        crate::info::MobjType::MtPuff,
    );
    {
        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.momz = crate::m_fixed::FRACUNIT;
        mobj.tics -= p_random() & 3;
        if mobj.tics < 1 {
            mobj.tics = 1;
        }
    }

    // don't make punches spark on the wall (the original's own comment,
    // preserved).
    if attackrange == crate::p_map::MELEERANGE {
        p_set_mobj_state(
            thinkers,
            level,
            th,
            crate::info::StateNum::SPuff3,
            |_, _, _, _| {},
        );
    }

    th
}

/// Port of `P_SpawnBlood`.
pub fn p_spawn_blood(
    thinkers: &mut Thinkers,
    level: &mut Level,
    x: Fixed,
    y: Fixed,
    z: Fixed,
    damage: i32,
) -> ThinkerId {
    let z = z + ((p_random() - p_random()) << 10);

    let th = p_spawn_mobj(
        thinkers,
        level,
        x,
        y,
        SpawnZ::At(z),
        crate::info::MobjType::MtBlood,
    );
    {
        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.momz = crate::m_fixed::FRACUNIT * 2;
        mobj.tics -= p_random() & 3;
        if mobj.tics < 1 {
            mobj.tics = 1;
        }
    }

    if (9..=12).contains(&damage) {
        p_set_mobj_state(
            thinkers,
            level,
            th,
            crate::info::StateNum::SBlood2,
            |_, _, _, _| {},
        );
    } else if damage < 9 {
        p_set_mobj_state(
            thinkers,
            level,
            th,
            crate::info::StateNum::SBlood3,
            |_, _, _, _| {},
        );
    }

    th
}

/// Port of `P_CheckMissileSpawn`. Moves the missile forward a bit and
/// possibly explodes it right there (the original's own comment,
/// preserved).
#[allow(clippy::too_many_arguments)]
pub fn p_check_missile_spawn(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    th: ThinkerId,
) {
    {
        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.tics -= p_random() & 3;
        if mobj.tics < 1 {
            mobj.tics = 1;
        }

        // move a little forward so an angle can be computed if it
        // immediately explodes (the original's own comment, preserved).
        mobj.x += mobj.momx >> 1;
        mobj.y += mobj.momy >> 1;
        mobj.z += mobj.momz >> 1;
    }

    let (x, y) = {
        let mobj = thinkers.mobj(th).unwrap();
        (mobj.x, mobj.y)
    };
    let (moved, _spechit) =
        crate::p_map::p_try_move(thinkers, level, players, rmain, validcount, th, x, y);
    if !moved {
        p_explode_missile(thinkers, level, th);
    }
}

/// Port of `P_SpawnMissile`.
#[allow(clippy::too_many_arguments)]
pub fn p_spawn_missile(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    source: ThinkerId,
    dest: ThinkerId,
    mobj_type: crate::info::MobjType,
) -> ThinkerId {
    let (sx, sy, sz) = {
        let s = thinkers.mobj(source).unwrap();
        (s.x, s.y, s.z)
    };

    let th = p_spawn_mobj(
        thinkers,
        level,
        sx,
        sy,
        SpawnZ::At(sz + 4 * 8 * crate::m_fixed::FRACUNIT),
        mobj_type,
    );

    let seesound = thinkers.mobj(th).unwrap().info.seesound;
    if !matches!(seesound, crate::sounds::Sfx::SfxNone) {
        crate::s_sound::s_start_sound(Some(crate::s_sound::SoundOrigin::Mobj(th)), seesound);
    }

    thinkers.mobj_mut(th).unwrap().target = Some(source); // where it came from

    let (dx, dy, dz) = {
        let d = thinkers.mobj(dest).unwrap();
        (d.x, d.y, d.z)
    };
    let mut an = rmain.point_to_angle2(sx, sy, dx, dy);

    // fuzzy player (the original's own comment, preserved).
    if thinkers.mobj(dest).unwrap().flags & crate::r_defs::mobj_flag::SHADOW != 0 {
        an = an.wrapping_add(((p_random() - p_random()) << 20) as u32);
    }

    let speed = thinkers.mobj(th).unwrap().info.speed;
    {
        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.angle = an;
    }
    let fine_an = (an >> crate::tables::ANGLETOFINESHIFT) as usize;
    let momx = fixed_mul(speed, crate::tables::fine_cosine(fine_an));
    let momy = fixed_mul(speed, crate::tables::FINESINE[fine_an]);

    let mut dist = crate::p_maputl::p_aprox_distance(dx - sx, dy - sy);
    dist /= speed;
    if dist < 1 {
        dist = 1;
    }
    let momz = (dz - sz) / dist;

    {
        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.momx = momx;
        mobj.momy = momy;
        mobj.momz = momz;
    }

    p_check_missile_spawn(thinkers, level, players, rmain, validcount, th);

    th
}

/// Port of `P_SpawnPlayerMissile`. Tries to aim at a nearby monster (the
/// original's own comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn p_spawn_player_missile(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    source: ThinkerId,
    mobj_type: crate::info::MobjType,
) -> ThinkerId {
    // see which target is to be aimed at (the original's own comment,
    // preserved).
    let mut an = thinkers.mobj(source).unwrap().angle;
    let (mut slope, mut target) = crate::p_map::p_aim_line_attack(
        thinkers,
        level,
        validcount,
        source,
        an,
        16 * 64 * crate::m_fixed::FRACUNIT,
    );

    if target.is_none() {
        an = an.wrapping_add(1 << 26);
        (slope, target) = crate::p_map::p_aim_line_attack(
            thinkers,
            level,
            validcount,
            source,
            an,
            16 * 64 * crate::m_fixed::FRACUNIT,
        );

        if target.is_none() {
            an = an.wrapping_sub(2 << 26);
            (slope, target) = crate::p_map::p_aim_line_attack(
                thinkers,
                level,
                validcount,
                source,
                an,
                16 * 64 * crate::m_fixed::FRACUNIT,
            );
        }

        if target.is_none() {
            an = thinkers.mobj(source).unwrap().angle;
            slope = 0;
        }
    }

    let (x, y, z) = {
        let s = thinkers.mobj(source).unwrap();
        (s.x, s.y, s.z + 4 * 8 * crate::m_fixed::FRACUNIT)
    };

    let th = p_spawn_mobj(thinkers, level, x, y, SpawnZ::At(z), mobj_type);

    let seesound = thinkers.mobj(th).unwrap().info.seesound;
    if !matches!(seesound, crate::sounds::Sfx::SfxNone) {
        crate::s_sound::s_start_sound(Some(crate::s_sound::SoundOrigin::Mobj(th)), seesound);
    }

    thinkers.mobj_mut(th).unwrap().target = Some(source);
    let speed = thinkers.mobj(th).unwrap().info.speed;
    let fine_an = (an >> crate::tables::ANGLETOFINESHIFT) as usize;
    {
        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.angle = an;
        mobj.momx = fixed_mul(speed, crate::tables::fine_cosine(fine_an));
        mobj.momy = fixed_mul(speed, crate::tables::FINESINE[fine_an]);
        mobj.momz = fixed_mul(speed, slope);
    }

    p_check_missile_spawn(thinkers, level, players, rmain, validcount, th);

    th
}

/// Port of `P_XYMovement`. See module docs on the `TODO(Phase 7)` gaps
/// this still has (nothing exercises them without monsters/missiles on
/// the map yet, so they're ported here — faithfully, just untested by
/// anything currently spawned on E1M1 — rather than stubbed).
pub fn p_xy_movement(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    id: ThinkerId,
) {
    let (momx0, momy0) = {
        let mobj = thinkers.mobj(id).unwrap();
        (mobj.momx, mobj.momy)
    };

    if momx0 == 0 && momy0 == 0 {
        let mobj = thinkers.mobj_mut(id).unwrap();
        if mobj.flags & crate::r_defs::mobj_flag::SKULLFLY != 0 {
            // the skull slammed into something (the original's own
            // comment, preserved).
            mobj.flags &= !crate::r_defs::mobj_flag::SKULLFLY;
            mobj.momx = 0;
            mobj.momy = 0;
            mobj.momz = 0;
            let spawnstate = mobj.info.spawnstate;
            p_set_mobj_state(thinkers, level, id, spawnstate, |_, _, _, _| {});
        }
        return;
    }

    {
        let mobj = thinkers.mobj_mut(id).unwrap();
        mobj.momx = mobj.momx.clamp(-MAXMOVE, MAXMOVE);
        mobj.momy = mobj.momy.clamp(-MAXMOVE, MAXMOVE);
    }

    let (mut xmove, mut ymove) = {
        let mobj = thinkers.mobj(id).unwrap();
        (mobj.momx, mobj.momy)
    };

    loop {
        let (x, y) = {
            let mobj = thinkers.mobj(id).unwrap();
            (mobj.x, mobj.y)
        };
        let (ptryx, ptryy);
        if xmove > MAXMOVE / 2 || ymove > MAXMOVE / 2 {
            ptryx = x + xmove / 2;
            ptryy = y + ymove / 2;
            xmove >>= 1;
            ymove >>= 1;
        } else {
            ptryx = x + xmove;
            ptryy = y + ymove;
            xmove = 0;
            ymove = 0;
        }

        let (moved, spechit) = crate::p_map::p_try_move(
            thinkers, level, players, rmain, validcount, id, ptryx, ptryy,
        );
        if !spechit.is_empty() {
            // Special-line crossing (`P_CrossSpecialLine`) needs
            // `crate::p_spec::SpecialsCtx`, which this function doesn't
            // have in scope — see `p_map::p_try_move`'s docs. The
            // caller that does own a `SpecialsCtx` (`p_ticker`, once
            // 7c/7d drive monster movement through here) is responsible
            // for draining `spechit` after calling this; nothing in
            // Phase 7b yet calls `p_xy_movement` for a mobj that can
            // cross a tagged line (no monsters/missiles spawn on E1M1),
            // so this is a documented gap, not a silent one.
        }
        if !moved {
            // blocked move (the original's own comment, preserved).
            let mobj = *thinkers.mobj(id).unwrap();
            if mobj.player.is_some() {
                // try to slide along it (the original's own comment,
                // preserved).
                crate::p_map::p_slide_move(thinkers, level, players, rmain, validcount, id);
            } else if mobj.flags & crate::r_defs::mobj_flag::MISSILE != 0 {
                // explode a missile (the original's own comment,
                // preserved). TODO(Phase 7): the sky-hack check reads
                // `ceilingline`, which `p_map`'s `MoveContext` computes
                // per-call and doesn't persist across calls the way the
                // original's global does — no caller of `p_xy_movement`
                // exists yet that spawns missiles (Phase 7), so this
                // exact sky-hack short-circuit is deferred with it;
                // every missile here always explodes via
                // `p_explode_missile` instead of the `P_RemoveMobj`
                // early-out.
                p_explode_missile(thinkers, level, id);
            } else {
                let mobj = thinkers.mobj_mut(id).unwrap();
                mobj.momx = 0;
                mobj.momy = 0;
            }
        }

        if xmove == 0 && ymove == 0 {
            break;
        }
    }

    // slow down (the original's own comment, preserved). The NOMOMENTUM
    // cheat check (`P_XYMovement`'s own `player->cheats & CF_NOMOMENTUM`
    // branch) needs `&mut Player`, which isn't reachable from a bare
    // `ThinkerId` here — [`p_mobj_thinker`] (which has `&mut Player` in
    // scope for the player's own mobj) applies that check itself,
    // before ever calling this function, so it never reaches this point
    // with nonzero momentum in the first place.
    let mobj = *thinkers.mobj(id).unwrap();
    if mobj.flags & (crate::r_defs::mobj_flag::MISSILE | crate::r_defs::mobj_flag::SKULLFLY) != 0 {
        return; // no friction for missiles ever (the original's own comment, preserved)
    }

    if mobj.z > mobj.floorz {
        return; // no friction when airborne (the original's own comment, preserved)
    }

    if mobj.flags & crate::r_defs::mobj_flag::CORPSE != 0 {
        // do not stop sliding if halfway off a step with some momentum
        // (the original's own comment, preserved).
        if (mobj.momx > crate::m_fixed::FRACUNIT / 4
            || mobj.momx < -crate::m_fixed::FRACUNIT / 4
            || mobj.momy > crate::m_fixed::FRACUNIT / 4
            || mobj.momy < -crate::m_fixed::FRACUNIT / 4)
            && mobj.floorz
                != level.sectors[level.subsectors[mobj.subsector.unwrap()].sector].floorheight
        {
            return;
        }
    }

    if mobj.momx > -STOPSPEED
        && mobj.momx < STOPSPEED
        && mobj.momy > -STOPSPEED
        && mobj.momy < STOPSPEED
    {
        // if in a walking frame, stop moving (the original's own
        // comment, preserved). The player-cmd-based extra condition and
        // the S_PLAY_RUN1..4 state reset are applied by the caller,
        // which has `&mut Player` — see `p_mobj_thinker`'s docs.
        let mobj = thinkers.mobj_mut(id).unwrap();
        mobj.momx = 0;
        mobj.momy = 0;
    } else {
        let mobj = thinkers.mobj_mut(id).unwrap();
        mobj.momx = fixed_mul(mobj.momx, FRICTION);
        mobj.momy = fixed_mul(mobj.momy, FRICTION);
    }
}

/// Port of `P_ZMovement`. `mo->player`'s `viewheight`/`deltaviewheight`
/// writes need `&mut Player`, not reachable from a bare `ThinkerId` —
/// see [`p_mobj_thinker`]'s docs for how the caller supplies it.
pub fn p_z_movement(
    thinkers: &mut Thinkers,
    level: &mut Level,
    mut player: Option<&mut crate::d_player::Player>,
    id: ThinkerId,
) {
    if let Some(player) = player.as_deref_mut() {
        // check for smooth step up (the original's own comment,
        // preserved).
        let (z, floorz) = {
            let mobj = thinkers.mobj(id).unwrap();
            (mobj.z, mobj.floorz)
        };
        if z < floorz {
            player.viewheight -= floorz - z;
            player.deltaviewheight = (VIEWHEIGHT - player.viewheight) >> 3;
        }
    }

    // adjust height (the original's own comment, preserved).
    {
        let mobj = thinkers.mobj_mut(id).unwrap();
        mobj.z += mobj.momz;
    }
    p_z_movement_clip(thinkers, level, id, player);
}

/// The shared tail of `P_ZMovement` (float-toward-target, floor/ceiling
/// clipping) once `mo->z` has already been advanced by `mo->momz` and
/// (for a player) `viewheight` adjusted — split out so
/// [`p_z_movement`]'s player/non-player halves don't duplicate it.
/// `player` (the owning [`crate::d_player::Player`], if any) gets the
/// hard-landing squat (`deltaviewheight`) and its `sfx_oof`.
fn p_z_movement_clip(
    thinkers: &mut Thinkers,
    level: &mut Level,
    id: ThinkerId,
    player: Option<&mut crate::d_player::Player>,
) {
    let mobj = *thinkers.mobj(id).unwrap();

    if mobj.flags & crate::r_defs::mobj_flag::FLOAT != 0 {
        if let Some(target) = mobj.target {
            // float down towards target if too close (the original's
            // own comment, preserved).
            if mobj.flags & crate::r_defs::mobj_flag::SKULLFLY == 0
                && mobj.flags & crate::r_defs::mobj_flag::INFLOAT == 0
            {
                if let Some(target_mobj) = thinkers.mobj(target) {
                    let dist = crate::p_maputl::p_aprox_distance(
                        mobj.x - target_mobj.x,
                        mobj.y - target_mobj.y,
                    );
                    let delta = (target_mobj.z + (mobj.height >> 1)) - mobj.z;

                    let mobj_mut = thinkers.mobj_mut(id).unwrap();
                    if delta < 0 && dist < -(delta * 3) {
                        mobj_mut.z -= FLOATSPEED;
                    } else if delta > 0 && dist < (delta * 3) {
                        mobj_mut.z += FLOATSPEED;
                    }
                }
            }
        }
    }

    let mobj = *thinkers.mobj(id).unwrap();

    // clip movement (the original's own comment, preserved).
    if mobj.z <= mobj.floorz {
        // hit the floor (the original's own comment, preserved).
        let mut momz = mobj.momz;

        // Note (id): somebody left this after the setting momz to 0,
        // kinda useless there (the original's own comment, preserved).
        if mobj.flags & crate::r_defs::mobj_flag::SKULLFLY != 0 {
            momz = -momz;
        }

        if momz < 0 {
            if let Some(player) = player {
                if momz < -GRAVITY * 8 {
                    // Squat down. Decrease viewheight for a moment
                    // after hitting the ground (hard), and utter
                    // appropriate sound (the original's own comment).
                    player.deltaviewheight = momz >> 3;
                    crate::s_sound::s_start_sound(
                        Some(crate::s_sound::SoundOrigin::Mobj(id)),
                        crate::sounds::Sfx::SfxOof,
                    );
                }
            }
            momz = 0;
        }

        {
            let mobj_mut = thinkers.mobj_mut(id).unwrap();
            mobj_mut.momz = momz;
            mobj_mut.z = mobj_mut.floorz;
        }

        if mobj.flags & crate::r_defs::mobj_flag::MISSILE != 0
            && mobj.flags & crate::r_defs::mobj_flag::NOCLIP == 0
        {
            p_explode_missile(thinkers, level, id);
            return;
        }
    } else if mobj.flags & crate::r_defs::mobj_flag::NOGRAVITY == 0 {
        let mobj_mut = thinkers.mobj_mut(id).unwrap();
        if mobj_mut.momz == 0 {
            mobj_mut.momz = -GRAVITY * 2;
        } else {
            mobj_mut.momz -= GRAVITY;
        }
    }

    let mobj = *thinkers.mobj(id).unwrap();
    if mobj.z + mobj.height > mobj.ceilingz {
        // hit the ceiling (the original's own comment, preserved).
        let mut momz = mobj.momz;
        if momz > 0 {
            momz = 0;
        }

        {
            let mobj_mut = thinkers.mobj_mut(id).unwrap();
            mobj_mut.momz = momz;
            mobj_mut.z = mobj_mut.ceilingz - mobj_mut.height;
        }

        if mobj.flags & crate::r_defs::mobj_flag::SKULLFLY != 0 {
            // the skull slammed into something (the original's own
            // comment, preserved).
            thinkers.mobj_mut(id).unwrap().momz = -momz;
        }

        if mobj.flags & crate::r_defs::mobj_flag::MISSILE != 0
            && mobj.flags & crate::r_defs::mobj_flag::NOCLIP == 0
        {
            p_explode_missile(thinkers, level, id);
        }
    }
}

/// Port of `P_MobjThinker` — the function every live mobj's thinker
/// slot dispatches to from [`Thinkers::run_thinkers`]. `players` is
/// every player in the game (needed since Phase 7b: `P_TryMove`'s
/// `PIT_CheckThing` can call `P_DamageMobj` on *any* player a moving
/// mobj collides with, not just `id`'s own owner — a skull-slam or
/// missile hitting a bystander player, say). `mo->player->...` writes
/// specific to `id`'s own owning player (`cheats`, `viewheight`/
/// `deltaviewheight`, the walking-frame `P_SetMobjState` reset) look up
/// that one player via `mobj.player` internally.
///
/// Returns `false` if the mobj removed itself (matching the original's
/// early `return` after a state chain reaches `S_NULL`) — callers don't
/// need to do anything with this beyond stopping further per-tic work
/// on `id`, since [`Thinkers::run_thinkers`] itself already tolerates a
/// removed-mid-tic thinker.
#[allow(clippy::too_many_arguments)]
pub fn p_mobj_thinker(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    validcount: i32,
    playeringame: &[bool],
    rdata: &crate::r_data::RData,
    brain: &mut crate::p_enemy::BrainTargets,
    id: ThinkerId,
) -> bool {
    let player_idx = thinkers.mobj(id).unwrap().player;

    let (momx, momy, flags) = {
        let mobj = thinkers.mobj(id).unwrap();
        (mobj.momx, mobj.momy, mobj.flags)
    };

    // momentum movement (the original's own comment, preserved).
    if momx != 0 || momy != 0 || flags & crate::r_defs::mobj_flag::SKULLFLY != 0 {
        // the NOMOMENTUM cheat check (`P_XYMovement`'s own
        // `player->cheats & CF_NOMOMENTUM` branch) needs `&mut Player`;
        // apply it here, before the movement it would otherwise
        // suppress friction-vs-instant-stop math for.
        if let Some(p) = player_idx.and_then(|i| players.get(i)) {
            if p.cheats & crate::d_player::Cheat::NoMomentum as i32 != 0 {
                let mobj = thinkers.mobj_mut(id).unwrap();
                mobj.momx = 0;
                mobj.momy = 0;
            }
        }

        p_xy_movement(thinkers, level, players, rmain, validcount, id);

        if !thinkers.is_live(id) {
            return false; // mobj was removed (the original's own comment, preserved)
        }

        // if in a walking frame, stop moving -> reset to S_PLAY once
        // momentum settled to exactly 0 and the player isn't pressing
        // forward/side (the original's `P_XYMovement` tail, split here
        // since it needs `&mut Player`/`&mut Level` together).
        if let Some(p) = player_idx.and_then(|i| players.get_mut(i)) {
            let (momx, momy) = {
                let mobj = thinkers.mobj(id).unwrap();
                (mobj.momx, mobj.momy)
            };
            if momx == 0 && momy == 0 && p.cmd.forwardmove == 0 && p.cmd.sidemove == 0 {
                let state = thinkers.mobj(id).unwrap().state;
                let running = matches!(
                    state,
                    crate::info::StateNum::SPlayRun1
                        | crate::info::StateNum::SPlayRun2
                        | crate::info::StateNum::SPlayRun3
                        | crate::info::StateNum::SPlayRun4
                );
                if running {
                    p_set_mobj_state(
                        thinkers,
                        level,
                        id,
                        crate::info::StateNum::SPlay,
                        |_, _, _, _| {},
                    );
                }
            }
        }
    }

    let (z, floorz, momz) = {
        let mobj = thinkers.mobj(id).unwrap();
        (mobj.z, mobj.floorz, mobj.momz)
    };
    if z != floorz || momz != 0 {
        p_z_movement(
            thinkers,
            level,
            player_idx.and_then(|i| players.get_mut(i)),
            id,
        );

        if !thinkers.is_live(id) {
            return false; // mobj was removed (the original's own comment, preserved)
        }
    }

    // cycle through states, calling action functions at transitions
    // (the original's own comment, preserved).
    let tics = thinkers.mobj(id).unwrap().tics;
    if tics != -1 {
        let mobj = thinkers.mobj_mut(id).unwrap();
        mobj.tics -= 1;

        // you can cycle through multiple states in a tic (the
        // original's own comment, preserved).
        if mobj.tics == 0 {
            let nextstate = STATES[mobj.state as usize].nextstate;
            let alive = p_set_mobj_state(
                thinkers,
                level,
                id,
                nextstate,
                |thinkers, level, id, action| {
                    crate::p_enemy::dispatch_mobj_action(
                        thinkers,
                        level,
                        players,
                        rmain,
                        validcount,
                        playeringame,
                        rdata,
                        brain,
                        id,
                        action,
                    );
                },
            );
            if !alive {
                return false; // freed itself (the original's own comment, preserved)
            }
        }
    } else {
        // check for nightmare respawn (the original's own comment,
        // preserved)
        let m = thinkers.mobj(id).unwrap();
        if m.flags & mobj_flag::COUNTKILL == 0 || !crate::doomstat::state().respawnmonsters {
            return true;
        }

        let m = thinkers.mobj_mut(id).unwrap();
        m.movecount += 1;
        if m.movecount < 12 * 35 {
            return true;
        }
        if crate::doomstat::state().leveltime & 31 != 0 {
            return true;
        }
        if crate::m_random::p_random() > 4 {
            return true;
        }
        p_nightmare_respawn(thinkers, level, players, rmain, id);
        return thinkers.is_live(id);
    }

    true
}

/// Port of `P_NightmareRespawn`: a dead monster comes back at its spawn
/// point (with teleport fog at both ends) when nothing occupies it.
pub fn p_nightmare_respawn(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut crate::r_main::RMain,
    id: ThinkerId,
) {
    use crate::s_sound::{s_start_sound, SoundOrigin};
    let mobj = *thinkers.mobj(id).unwrap();
    let x = (mobj.spawnpoint.x as i32) << FRACBITS;
    let y = (mobj.spawnpoint.y as i32) << FRACBITS;

    // somthing is occupying it's position? (the original's own comment,
    // preserved)
    if !crate::p_map::p_check_position(thinkers, level, players, rmain, 0, id, x, y).0 {
        return; // no respwan
    }

    // spawn a teleport fog at old spot because of removal of the body?
    // (the original's own comment, preserved)
    let old_floor = mobj.subsector.map_or(0, |ss| {
        level.sectors[level.subsectors[ss].sector].floorheight
    });
    let mo = p_spawn_mobj(
        thinkers,
        level,
        mobj.x,
        mobj.y,
        SpawnZ::At(old_floor),
        crate::info::MobjType::MtTfog,
    );
    // initiate teleport sound
    s_start_sound(Some(SoundOrigin::Mobj(mo)), crate::sounds::Sfx::SfxTelept);

    // spawn a teleport fog at the new spot
    let ss = crate::r_main::RMain::point_in_subsector(level, x, y);
    let new_floor = level.sectors[level.subsectors[ss].sector].floorheight;
    let mo = p_spawn_mobj(
        thinkers,
        level,
        x,
        y,
        SpawnZ::At(new_floor),
        crate::info::MobjType::MtTfog,
    );
    s_start_sound(Some(SoundOrigin::Mobj(mo)), crate::sounds::Sfx::SfxTelept);

    // spawn the new monster
    let mthing = mobj.spawnpoint;
    let z = if MOBJINFO[mobj.mobj_type as usize].flags & mobj_flag::SPAWNCEILING != 0 {
        SpawnZ::OnCeiling
    } else {
        SpawnZ::OnFloor
    };

    // inherit attributes from deceased one (the original's own comment,
    // preserved)
    let mo = p_spawn_mobj(thinkers, level, x, y, z, mobj.mobj_type);
    let new = thinkers.mobj_mut(mo).unwrap();
    new.spawnpoint = mthing;
    new.angle = crate::tables::ANG45.wrapping_mul((mthing.angle / 45) as u32);
    if mthing.options & (crate::doomdef::MTF_AMBUSH as i16) != 0 {
        new.flags |= mobj_flag::AMBUSH;
    }
    new.reactiontime = 18;

    // remove the old monster, (the original's own comment, preserved)
    p_remove_mobj(thinkers, level, id);
}

/// Port of `P_RespawnSpecials`: deathmatch-2 items reappear 30 seconds
/// after they were picked up, with a fog at the spot.
pub fn p_respawn_specials(thinkers: &mut Thinkers, level: &mut Level) {
    use crate::s_sound::{s_start_sound, SoundOrigin};
    // only respawn items in deathmatch (the original's own comment,
    // preserved)
    let st = crate::doomstat::state();
    if !(st.deathmatch && st.altdeath) {
        return;
    }
    // nothing left to respawn? / wait at least 30 seconds (the
    // original's own comments, preserved)
    let Some(&(mthing, since)) = level.itemrespawnque.front() else {
        return;
    };
    if st.leveltime - since < 30 * 35 {
        return;
    }

    let x = (mthing.x as i32) << FRACBITS;
    let y = (mthing.y as i32) << FRACBITS;

    // spawn a teleport fog at the new spot (the original's own comment,
    // preserved)
    let ss = crate::r_main::RMain::point_in_subsector(level, x, y);
    let floor = level.sectors[level.subsectors[ss].sector].floorheight;
    let mo = p_spawn_mobj(
        thinkers,
        level,
        x,
        y,
        SpawnZ::At(floor),
        crate::info::MobjType::MtIfog,
    );
    s_start_sound(Some(SoundOrigin::Mobj(mo)), crate::sounds::Sfx::SfxItmbk);

    // find which type to spawn, then spawn it (the original's own
    // comments, preserved)
    if let Some(i) = MOBJINFO
        .iter()
        .position(|m| m.doomednum == mthing.thing_type as i32)
    {
        let z = if MOBJINFO[i].flags & mobj_flag::SPAWNCEILING != 0 {
            SpawnZ::OnCeiling
        } else {
            SpawnZ::OnFloor
        };
        let ty = crate::info::MobjType::from_index(i).expect("index from MOBJINFO");
        let mo = p_spawn_mobj(thinkers, level, x, y, z, ty);
        let m = thinkers.mobj_mut(mo).unwrap();
        m.spawnpoint = mthing;
        m.angle = crate::tables::ANG45.wrapping_mul((mthing.angle / 45) as u32);
    }

    // pull it from the que (the original's own comment, preserved)
    level.itemrespawnque.pop_front();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::MobjType;
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

    #[test]
    fn spawn_mobj_links_into_sector_thinglist_and_sets_z() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();

        // Player 1's start, from E1M1 (see tests/real_frame.rs).
        let x = 1056 * crate::m_fixed::FRACUNIT;
        let y = -3616 * crate::m_fixed::FRACUNIT;
        let id = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            x,
            y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let mobj = thinkers.mobj(id).expect("spawned mobj is live");
        let sector = level.subsectors[mobj.subsector.unwrap()].sector;
        assert_eq!(mobj.z, level.sectors[sector].floorheight);
        assert_eq!(level.sectors[sector].thinglist, Some(id));
        assert_eq!(mobj.snext, None);
        assert_eq!(mobj.sprev, None);
    }

    #[test]
    fn two_mobjs_in_the_same_sector_chain_newest_first() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let x = 1056 * crate::m_fixed::FRACUNIT;
        let y = -3616 * crate::m_fixed::FRACUNIT;

        let a = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            x,
            y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let b = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            x,
            y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let sector = level.subsectors[thinkers.mobj(a).unwrap().subsector.unwrap()].sector;
        assert_eq!(level.sectors[sector].thinglist, Some(b));
        assert_eq!(thinkers.mobj(b).unwrap().snext, Some(a));
        assert_eq!(thinkers.mobj(a).unwrap().sprev, Some(b));
    }

    #[test]
    fn remove_mobj_unlinks_from_sector_and_marks_removed() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let x = 1056 * crate::m_fixed::FRACUNIT;
        let y = -3616 * crate::m_fixed::FRACUNIT;

        let a = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            x,
            y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let b = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            x,
            y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let sector = level.subsectors[thinkers.mobj(a).unwrap().subsector.unwrap()].sector;

        p_remove_mobj(&mut thinkers, &mut level, b);

        assert_eq!(level.sectors[sector].thinglist, Some(a));
        assert_eq!(thinkers.mobj(a).unwrap().sprev, None);
        assert!(!thinkers.is_live(b));
    }

    #[test]
    fn set_mobj_state_to_null_removes_the_mobj() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let x = 1056 * crate::m_fixed::FRACUNIT;
        let y = -3616 * crate::m_fixed::FRACUNIT;
        let id = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            x,
            y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let alive = p_set_mobj_state(
            &mut thinkers,
            &mut level,
            id,
            crate::info::StateNum::SNull,
            |_, _, _, _| {},
        );

        assert!(!alive);
        assert!(!thinkers.is_live(id));
    }

    // Player 1's start, from E1M1 (see tests/real_frame.rs).
    const START_X: Fixed = 1056 * crate::m_fixed::FRACUNIT;
    const START_Y: Fixed = -3616 * crate::m_fixed::FRACUNIT;

    #[test]
    fn spawn_puff_creates_a_short_lived_puff_mobj() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();

        let id = p_spawn_puff(
            &mut thinkers,
            &mut level,
            200 * crate::m_fixed::FRACUNIT,
            START_X,
            START_Y,
            0,
        );

        let mobj = thinkers.mobj(id).expect("puff is live");
        assert_eq!(mobj.mobj_type, MobjType::MtPuff);
        assert!(mobj.tics >= 1);
    }

    #[test]
    fn spawn_puff_at_melee_range_uses_s_puff3() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();

        let id = p_spawn_puff(
            &mut thinkers,
            &mut level,
            crate::p_map::MELEERANGE,
            START_X,
            START_Y,
            0,
        );

        assert_eq!(
            thinkers.mobj(id).unwrap().state,
            crate::info::StateNum::SPuff3,
            "punches shouldn't spark on the wall (S_PUFF3, no visible spark frames)"
        );
    }

    #[test]
    fn spawn_blood_picks_state_by_damage_amount() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();

        let heavy = p_spawn_blood(&mut thinkers, &mut level, START_X, START_Y, 0, 20);
        assert_eq!(
            thinkers.mobj(heavy).unwrap().state,
            crate::info::StateNum::SBlood1,
            "damage above 12 keeps the default S_BLOOD1 spawnstate"
        );

        let medium = p_spawn_blood(&mut thinkers, &mut level, START_X, START_Y, 0, 10);
        assert_eq!(
            thinkers.mobj(medium).unwrap().state,
            crate::info::StateNum::SBlood2
        );

        let light = p_spawn_blood(&mut thinkers, &mut level, START_X, START_Y, 0, 5);
        assert_eq!(
            thinkers.mobj(light).unwrap().state,
            crate::info::StateNum::SBlood3
        );
    }

    #[test]
    fn spawn_missile_aims_toward_the_destination() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: [crate::d_player::Player; 0] = [];

        let source = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let dest = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 200 * crate::m_fixed::FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let missile = p_spawn_missile(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            source,
            dest,
            MobjType::MtTroopshot,
        );

        let mobj = thinkers.mobj(missile);
        // The missile may have already exploded on spawn (P_CheckMissileSpawn)
        // if the initial forward nudge failed — assert on whichever state
        // it's actually in rather than assuming survival.
        if let Some(mobj) = mobj {
            assert_eq!(mobj.mobj_type, MobjType::MtTroopshot);
            assert_eq!(mobj.target, Some(source));
            // aimed roughly east, toward `dest`.
            assert!(
                mobj.momx > 0,
                "a missile aimed due east should have positive x momentum"
            );
        }
    }

    #[test]
    fn check_missile_spawn_explodes_on_immediate_blockage() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut rmain = RMain::new();
        let mut players: [crate::d_player::Player; 0] = [];

        // Same `far` constant `p_map.rs`'s
        // `try_move_through_a_solid_one_sided_wall_fails` uses — large
        // enough that at least one of the four cardinal directions hits
        // a one-sided wall in a reasonably sized E1M1 room, small enough
        // to stay clear of i32 overflow from `START_X`/`START_Y`.
        let far = 10_000 * crate::m_fixed::FRACUNIT;
        let exploded =
            [(far, 0), (-far, 0), (0, far), (0, -far)]
                .into_iter()
                .any(|(momx, momy)| {
                    let mut thinkers = Thinkers::new();
                    let th = p_spawn_mobj(
                        &mut thinkers,
                        &mut level,
                        START_X,
                        START_Y,
                        SpawnZ::OnFloor,
                        MobjType::MtTroopshot,
                    );
                    {
                        let mobj = thinkers.mobj_mut(th).unwrap();
                        mobj.flags |= crate::r_defs::mobj_flag::MISSILE;
                        mobj.momx = momx;
                        mobj.momy = momy;
                    }

                    p_check_missile_spawn(
                        &mut thinkers,
                        &mut level,
                        &mut players,
                        &mut rmain,
                        0,
                        th,
                    );

                    // An exploded missile has `momx`/`momy`/`momz` zeroed by
                    // `p_explode_missile`; a missile that moved fine keeps
                    // its (large) momentum untouched.
                    thinkers.mobj(th).unwrap().momx == 0
                });
        assert!(
            exploded,
            "at least one direction should immediately block the missile and explode it"
        );
    }
}
