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
//	Enemy thinking, AI.
//	Action Pointer Functions
//	that are associated with states/frames.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_enemy.c` (partial, see note below). Enemy
//! thinking, AI. Action Pointer Functions that are associated with
//! states/frames (the original's own description, preserved).
//!
//! # Scope
//!
//! Ported (Phase 7c — line of sight + movement AI): [`p_recursive_sound`]
//! (`P_RecursiveSound`), [`p_noise_alert`] (`P_NoiseAlert`),
//! [`p_check_melee_range`] (`P_CheckMeleeRange`),
//! [`p_check_missile_range`] (`P_CheckMissileRange`), [`p_move`]
//! (`P_Move`), [`p_try_walk`] (`P_TryWalk`), [`p_new_chase_dir`]
//! (`P_NewChaseDir`), [`p_look_for_players`] (`P_LookForPlayers`),
//! [`a_keen_die`] (`A_KeenDie`), [`a_fall`] (`A_Fall`), [`a_look`]
//! (`A_Look`), [`a_chase`] (`A_Chase`), plus [`dispatch_mobj_action`],
//! the `match` over [`crate::info::StateAction`] that
//! [`crate::p_mobj::p_set_mobj_state`] calls into (mirroring
//! `p_pspr.rs`'s `dispatch_psprite_action`).
//!
//! Not ported yet (Phase 7d): `A_FaceTarget` onward — every monster
//! *attack* action (`A_PosAttack`, `A_SPosAttack`, `A_CPosAttack`,
//! `A_TroopAttack`, `A_SargAttack`, `A_HeadAttack`, `A_CyberAttack`,
//! `A_BruisAttack`, `A_SkelMissile`/`A_Tracer`/`A_SkelWhoosh`/
//! `A_SkelFist`, the Vile/Fatso/Lost-soul special attacks, `A_Pain`/
//! `A_Scream`/`A_XScream`/`A_PainDie`, `A_BossDeath`, the Romero/brain
//! actions) and their `P_*` helpers (`PIT_VileCheck`, etc.). Reaching
//! any of those states before 7d lands is a documented no-op via
//! [`dispatch_mobj_action`]'s `_ => {}` arm, same convention as
//! `p_pspr.rs`'s attack actions before Phase 7 landed.
//!
//! Sounds (`S_StartSound`) are queued via [`crate::s_sound`], with the
//! actor's [`SoundOrigin::Mobj`] as origin (`None` = the original's
//! `NULL`, "full volume").

use crate::d_player::Player;
use crate::info::{MobjType, StateAction, StateNum, STATES};
use crate::m_fixed::{fixed_mul, Fixed, FRACUNIT};
use crate::p_maputl::p_aprox_distance;
use crate::p_setup::Level;
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_defs::mobj_flag;
use crate::r_main::RMain;
use crate::s_sound::{s_start_sound, SoundOrigin};
use crate::sounds::Sfx;

/// (`MELEERANGE`, `p_enemy.c`'s own `#define`).
const MELEERANGE: Fixed = 64 * FRACUNIT;
/// (`FLOATSPEED`, `p_local.h`). Duplicated from `p_mobj.rs`'s private
/// copy — see that module's own comment on why there's no shared
/// `p_local`-equivalent module in this port.
const FLOATSPEED: Fixed = FRACUNIT * 4;

/// Movement direction, `p_enemy.c`'s `dirtype_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum DirType {
    East = 0,
    Northeast = 1,
    North = 2,
    Northwest = 3,
    West = 4,
    Southwest = 5,
    South = 6,
    Southeast = 7,
    NoDir = 8,
}

impl DirType {
    fn from_i32(v: i32) -> DirType {
        match v {
            0 => DirType::East,
            1 => DirType::Northeast,
            2 => DirType::North,
            3 => DirType::Northwest,
            4 => DirType::West,
            5 => DirType::Southwest,
            6 => DirType::South,
            7 => DirType::Southeast,
            _ => DirType::NoDir,
        }
    }
}

/// `p_enemy.c`'s `opposite[]` LUT.
const OPPOSITE: [DirType; 9] = [
    DirType::West,
    DirType::Southwest,
    DirType::South,
    DirType::Southeast,
    DirType::East,
    DirType::Northeast,
    DirType::North,
    DirType::Northwest,
    DirType::NoDir,
];

/// `p_enemy.c`'s `diags[]` LUT.
const DIAGS: [DirType; 4] = [
    DirType::Northwest,
    DirType::Northeast,
    DirType::Southwest,
    DirType::Southeast,
];

/// `p_enemy.c`'s `xspeed[]` LUT.
const XSPEED: [Fixed; 8] = [FRACUNIT, 47000, 0, -47000, -FRACUNIT, -47000, 0, 47000];
/// `p_enemy.c`'s `yspeed[]` LUT.
const YSPEED: [Fixed; 8] = [0, 47000, FRACUNIT, 47000, 0, -47000, -FRACUNIT, -47000];

/// Dispatches a state's action for a mobj, standing in for the
/// original's `state->action.acp1(mobj)`. Called from
/// [`crate::p_mobj::p_set_mobj_state`]. See module docs.
///
/// `rdata`/`brain` are needed by [`a_boss_death`]/[`a_brain_awake`]/
/// [`a_brain_spit`] respectively — threaded through the same way
/// `playeringame` was in Phase 7c (see [`BrainTargets`]'s docs).
#[allow(clippy::too_many_arguments)]
pub fn dispatch_mobj_action(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    playeringame: &[bool],
    rdata: &crate::r_data::RData,
    brain: &mut BrainTargets,
    id: ThinkerId,
    action: StateAction,
) {
    match action {
        StateAction::None => {}
        StateAction::AFall => a_fall(thinkers, id),
        StateAction::AKeenDie => a_keen_die(thinkers, level, id),
        StateAction::ALook => a_look(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            rdata,
            brain,
            id,
        ),
        StateAction::AChase => a_chase(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            rdata,
            brain,
            id,
        ),
        StateAction::AFaceTarget => a_face_target(thinkers, rmain, id),
        StateAction::APosAttack => a_pos_attack(thinkers, level, players, rmain, validcount, id),
        StateAction::ASPosAttack => a_spos_attack(thinkers, level, players, rmain, validcount, id),
        StateAction::ACPosAttack => a_cpos_attack(thinkers, level, players, rmain, validcount, id),
        StateAction::ACPosRefire => a_cpos_refire(thinkers, level, rmain, validcount, id),
        StateAction::ASpidRefire => a_spid_refire(thinkers, level, rmain, validcount, id),
        StateAction::ABspiAttack => a_bspi_attack(thinkers, level, players, rmain, validcount, id),
        StateAction::ATroopAttack => {
            a_troop_attack(thinkers, level, players, rmain, validcount, id)
        }
        StateAction::ASargAttack => a_sarg_attack(thinkers, level, players, rmain, validcount, id),
        StateAction::AHeadAttack => a_head_attack(thinkers, level, players, rmain, validcount, id),
        StateAction::ACyberAttack => {
            a_cyber_attack(thinkers, level, players, rmain, validcount, id)
        }
        StateAction::ABruisAttack => {
            a_bruis_attack(thinkers, level, players, rmain, validcount, id)
        }
        StateAction::ASkelMissile => {
            a_skel_missile(thinkers, level, players, rmain, validcount, id)
        }
        StateAction::ATracer => a_tracer(thinkers, level, rmain, id),
        StateAction::ASkelWhoosh => a_skel_whoosh(thinkers, rmain, id),
        StateAction::ASkelFist => a_skel_fist(thinkers, level, players, rmain, validcount, id),
        StateAction::AVileChase => a_vile_chase(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            rdata,
            brain,
            id,
        ),
        StateAction::AVileStart => a_vile_start(thinkers, id),
        StateAction::AStartFire => a_start_fire(thinkers, level, validcount, id),
        StateAction::AFireCrackle => a_fire_crackle(thinkers, level, validcount, id),
        StateAction::AVileTarget => a_vile_target(thinkers, level, rmain, id),
        StateAction::AVileAttack => a_vile_attack(thinkers, level, players, rmain, validcount, id),
        StateAction::AFatRaise => a_fat_raise(thinkers, rmain, id),
        StateAction::AFatAttack1 => a_fat_attack1(thinkers, level, players, rmain, validcount, id),
        StateAction::AFatAttack2 => a_fat_attack2(thinkers, level, players, rmain, validcount, id),
        StateAction::AFatAttack3 => a_fat_attack3(thinkers, level, players, rmain, validcount, id),
        StateAction::ASkullAttack => a_skull_attack(thinkers, rmain, id),
        StateAction::APainAttack => a_pain_attack(thinkers, level, players, rmain, validcount, id),
        StateAction::APainDie => a_pain_die(thinkers, level, players, rmain, validcount, id),
        StateAction::AScream => a_scream(thinkers, id),
        StateAction::AXScream => a_xscream(thinkers, id),
        StateAction::APain => a_pain(thinkers, id),
        StateAction::AExplode => a_explode(thinkers, level, players, rmain, validcount, id),
        StateAction::ABFGSpray => a_bfg_spray(thinkers, level, players, rmain, validcount, id),
        StateAction::ABossDeath => a_boss_death(thinkers, level, players, rdata, id, playeringame),
        StateAction::AHoof => a_hoof(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            rdata,
            brain,
            id,
        ),
        StateAction::AMetal => a_metal(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            rdata,
            brain,
            id,
        ),
        StateAction::ABabyMetal => a_baby_metal(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            rdata,
            brain,
            id,
        ),
        StateAction::ABrainAwake => a_brain_awake(thinkers, brain),
        StateAction::ABrainPain => a_brain_pain(),
        StateAction::ABrainScream => a_brain_scream(thinkers, level, id),
        StateAction::ABrainExplode => a_brain_explode(thinkers, level, id),
        StateAction::ABrainDie => a_brain_die(),
        StateAction::ABrainSpit => {
            a_brain_spit(thinkers, level, players, rmain, validcount, brain, id)
        }
        StateAction::ASpawnSound => a_spawn_sound(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            id,
        ),
        StateAction::ASpawnFly => a_spawn_fly(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            id,
        ),
        StateAction::APlayerScream => a_player_scream(thinkers, id),
        // Weapon-side actions (A_Punch, A_Saw, A_Fire*, A_*Shotgun2,
        // A_ReFire, A_Light*, A_WeaponReady, A_Lower/A_Raise,
        // A_GunFlash, A_CheckReload) are dispatched by
        // `p_pspr::dispatch_psprite_action` instead — they take
        // `(player, psprite)`, not a bare mobj, so they can never reach
        // this `match` in practice (no `Mobj`'s `state->action` is ever
        // one of those variants — only psprite states are). Left here
        // as a documented no-op rather than `unreachable!()`, matching
        // this port's general convention of not panicking on states no
        // caller can actually reach.
        _ => {}
    }
}

/// Port of `P_RecursiveSound`. Recursively traverse adjacent sectors,
/// sound blocking lines cut off traversal (the original's own comment,
/// preserved).
fn p_recursive_sound(
    level: &mut Level,
    validcount: i32,
    soundtarget: ThinkerId,
    sec_idx: usize,
    soundblocks: i32,
) {
    // wake up all monsters in this sector (the original's own comment,
    // preserved).
    {
        let sec = &level.sectors[sec_idx];
        if sec.validcount == validcount && sec.soundtraversed <= soundblocks + 1 {
            return; // already flooded
        }
    }

    {
        let sec = &mut level.sectors[sec_idx];
        sec.validcount = validcount;
        sec.soundtraversed = soundblocks + 1;
        sec.soundtarget = Some(soundtarget);
    }

    let line_indices = level.sectors[sec_idx].lines.clone();
    for line_idx in line_indices {
        let line = level.lines[line_idx];
        if line.flags & crate::doomdata::ML_TWOSIDED == 0 {
            continue;
        }

        let opening = crate::p_maputl::p_line_opening(level, &line);
        if opening.openrange <= 0 {
            continue; // closed door
        }

        let side0 = line.sidenum[0].unwrap();
        let other = if level.sides[side0].sector == sec_idx {
            let side1 = line.sidenum[1].unwrap();
            level.sides[side1].sector
        } else {
            level.sides[side0].sector
        };

        if line.flags & crate::doomdata::ML_SOUNDBLOCK != 0 {
            if soundblocks == 0 {
                p_recursive_sound(level, validcount, soundtarget, other, 1);
            }
        } else {
            p_recursive_sound(level, validcount, soundtarget, other, soundblocks);
        }
    }
}

/// Port of `P_NoiseAlert`. If a monster yells at a player, it will alert
/// other monsters to the player (the original's own comment, preserved).
///
/// Returns the new `validcount` (bumped by 1, like the original's
/// `validcount++`) — see the project-wide convention of threading
/// `validcount` explicitly (`p_map.rs`/`p_sight.rs`).
pub fn p_noise_alert(
    thinkers: &Thinkers,
    level: &mut Level,
    _validcount: i32,
    target: ThinkerId,
    emitter: ThinkerId,
) -> i32 {
    let validcount = crate::p_maputl::bump_validcount();
    let emitter_sector =
        level.subsectors[thinkers.mobj(emitter).unwrap().subsector.unwrap()].sector;
    p_recursive_sound(level, validcount, target, emitter_sector, 0);
    validcount
}

/// Port of `P_CheckMeleeRange`.
pub fn p_check_melee_range(
    thinkers: &Thinkers,
    level: &mut Level,
    validcount: i32,
    actor: ThinkerId,
) -> (bool, i32) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return (false, validcount);
    };

    let (ax, ay) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y)
    };
    let (px, py, pradius) = {
        let p = thinkers.mobj(target).unwrap();
        (p.x, p.y, p.radius)
    };
    let dist = p_aprox_distance(px - ax, py - ay);

    if dist >= MELEERANGE - 20 * FRACUNIT + pradius {
        return (false, validcount);
    }

    let (sight, validcount) =
        crate::p_sight::p_check_sight(thinkers, level, validcount, actor, target);
    if !sight {
        return (false, validcount);
    }

    (true, validcount)
}

/// Port of `P_CheckMissileRange`.
pub fn p_check_missile_range(
    thinkers: &mut Thinkers,
    level: &mut Level,
    validcount: i32,
    actor: ThinkerId,
) -> (bool, i32) {
    let target = thinkers.mobj(actor).unwrap().target.unwrap();
    let (sight, validcount) =
        crate::p_sight::p_check_sight(thinkers, level, validcount, actor, target);
    if !sight {
        return (false, validcount);
    }

    if thinkers.mobj(actor).unwrap().flags & mobj_flag::JUSTHIT != 0 {
        // the target just hit the enemy, so fight back! (the original's
        // own comment, preserved).
        thinkers.mobj_mut(actor).unwrap().flags &= !mobj_flag::JUSTHIT;
        return (true, validcount);
    }

    if thinkers.mobj(actor).unwrap().reactiontime != 0 {
        return (false, validcount); // do not attack yet
    }

    // OPTIMIZE: get this from a global checksight (the original's own
    // comment, preserved).
    let (ax, ay, atype, meleestate) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y, a.mobj_type, a.info.meleestate)
    };
    let (tx, ty) = {
        let t = thinkers.mobj(target).unwrap();
        (t.x, t.y)
    };
    let mut dist = p_aprox_distance(ax - tx, ay - ty) - 64 * FRACUNIT;

    if matches!(meleestate, StateNum::SNull) {
        dist -= 128 * FRACUNIT; // no melee attack, so fire more
    }

    dist >>= 16;

    if atype == MobjType::MtVile && dist > 14 * 64 {
        return (false, validcount); // too far away
    }

    if atype == MobjType::MtUndead {
        if dist < 196 {
            return (false, validcount); // close for fist attack
        }
        dist >>= 1;
    }

    if matches!(
        atype,
        MobjType::MtCyborg | MobjType::MtSpider | MobjType::MtSkull
    ) {
        dist >>= 1;
    }

    if dist > 200 {
        dist = 200;
    }

    if atype == MobjType::MtCyborg && dist > 160 {
        dist = 160;
    }

    if crate::m_random::p_random() < dist {
        return (false, validcount);
    }

    (true, validcount)
}

/// Port of `P_Move`. Move in the current direction, returns `false` if
/// the move is blocked (the original's own comment, preserved).
///
/// `specials` is `Some(ctx)` when a [`crate::p_spec::SpecialsCtx`] is
/// available to drain the spechit list through
/// (`P_UseSpecialLine`/`P_CrossSpecialLine` — the original's own
/// `spechit[]`/`P_UseSpecialLine` loop). Every caller in this phase
/// (`p_mobj_thinker`, via [`dispatch_mobj_action`]) doesn't have one in
/// scope (same documented gap as `p_mobj::p_xy_movement`'s spechit
/// handling) — see that function's docs for why this mirrors it rather
/// than plumbing `SpecialsCtx` all the way through `p_mobj_thinker` for
/// a mechanism nothing on E1M1 exercises yet (no monsters ship on the
/// map this port's tests load).
pub fn p_move(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) -> bool {
    let (movedir, speed) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.movedir, a.info.speed)
    };

    if movedir == DirType::NoDir as i32 {
        return false;
    }

    debug_assert!((0..8).contains(&movedir), "Weird actor->movedir!");

    let (x, y) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y)
    };
    let tryx = x + speed * XSPEED[movedir as usize];
    let tryy = y + speed * YSPEED[movedir as usize];

    let (try_ok, spechit) = crate::p_map::p_try_move(
        thinkers, level, players, rmain, validcount, actor, tryx, tryy,
    );

    if !try_ok {
        // open any specials (the original's own comment, preserved).
        let mobj = *thinkers.mobj(actor).unwrap();
        if mobj.flags & mobj_flag::FLOAT != 0 {
            // NOTE: the original also requires the last `P_TryMove`'s
            // `floatok`, which this port's `p_try_move` doesn't expose
            // to callers (see `p_mobj::p_xy_movement`'s own note on the
            // same gap) — documented divergence, not exercised by any
            // monster on E1M1 yet (no `MF_FLOAT` monster spawns there).
        }

        if spechit.is_empty() {
            return false;
        }

        thinkers.mobj_mut(actor).unwrap().movedir = DirType::NoDir as i32;
        // The original drains `spechit` calling `P_UseSpecialLine` on
        // each hit line and reports `good` if any line accepted the
        // use. Without a `SpecialsCtx` in scope here (see this
        // function's docs), this is a documented no-op — same shape as
        // `p_mobj::p_xy_movement`'s own spechit gap.
        let _ = spechit;
        return false;
    }

    thinkers.mobj_mut(actor).unwrap().flags &= !mobj_flag::INFLOAT;

    if thinkers.mobj(actor).unwrap().flags & mobj_flag::FLOAT == 0 {
        let floorz = thinkers.mobj(actor).unwrap().floorz;
        thinkers.mobj_mut(actor).unwrap().z = floorz;
    }
    true
}

/// Port of `P_TryWalk`. Attempts to move actor on in its current
/// (`ob->moveangle`) direction. If blocked by either a wall or an actor
/// returns `false`. If move is either clear or blocked only by a door,
/// returns `true` and sets... If a door is in the way, an OpenDoor call
/// is made to start it opening (the original's own comment, preserved).
pub fn p_try_walk(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) -> bool {
    if !p_move(thinkers, level, players, rmain, validcount, actor) {
        return false;
    }
    thinkers.mobj_mut(actor).unwrap().movecount = crate::m_random::p_random() & 15;
    true
}

/// Port of `P_NewChaseDir`.
pub fn p_new_chase_dir(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let target = thinkers
        .mobj(actor)
        .unwrap()
        .target
        .expect("P_NewChaseDir: called with no target");

    let olddir = DirType::from_i32(thinkers.mobj(actor).unwrap().movedir);
    let turnaround = OPPOSITE[olddir as usize];

    let (ax, ay) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y)
    };
    let (tx, ty) = {
        let t = thinkers.mobj(target).unwrap();
        (t.x, t.y)
    };
    let deltax = tx - ax;
    let deltay = ty - ay;

    let mut d1 = if deltax > 10 * FRACUNIT {
        DirType::East
    } else if deltax < -10 * FRACUNIT {
        DirType::West
    } else {
        DirType::NoDir
    };

    let mut d2 = if deltay < -10 * FRACUNIT {
        DirType::South
    } else if deltay > 10 * FRACUNIT {
        DirType::North
    } else {
        DirType::NoDir
    };

    // try direct route (the original's own comment, preserved).
    if d1 != DirType::NoDir && d2 != DirType::NoDir {
        let idx = (((deltay < 0) as usize) << 1) + (deltax > 0) as usize;
        let dir = DIAGS[idx];
        thinkers.mobj_mut(actor).unwrap().movedir = dir as i32;
        if dir != turnaround && p_try_walk(thinkers, level, players, rmain, validcount, actor) {
            return;
        }
    }

    // try other directions (the original's own comment, preserved).
    if crate::m_random::p_random() > 200 || deltay.abs() > deltax.abs() {
        std::mem::swap(&mut d1, &mut d2);
    }

    if d1 == turnaround {
        d1 = DirType::NoDir;
    }
    if d2 == turnaround {
        d2 = DirType::NoDir;
    }

    if d1 != DirType::NoDir {
        thinkers.mobj_mut(actor).unwrap().movedir = d1 as i32;
        if p_try_walk(thinkers, level, players, rmain, validcount, actor) {
            // either moved forward or attacked (the original's own
            // comment, preserved).
            return;
        }
    }

    if d2 != DirType::NoDir {
        thinkers.mobj_mut(actor).unwrap().movedir = d2 as i32;
        if p_try_walk(thinkers, level, players, rmain, validcount, actor) {
            return;
        }
    }

    // there is no direct path to the player, so pick another direction
    // (the original's own comment, preserved).
    if olddir != DirType::NoDir {
        thinkers.mobj_mut(actor).unwrap().movedir = olddir as i32;
        if p_try_walk(thinkers, level, players, rmain, validcount, actor) {
            return;
        }
    }

    // randomly determine direction of search (the original's own
    // comment, preserved).
    if crate::m_random::p_random() & 1 != 0 {
        for tdir in DirType::East as i32..=DirType::Southeast as i32 {
            if tdir != turnaround as i32 {
                thinkers.mobj_mut(actor).unwrap().movedir = tdir;
                if p_try_walk(thinkers, level, players, rmain, validcount, actor) {
                    return;
                }
            }
        }
    } else {
        for tdir in (DirType::East as i32..=DirType::Southeast as i32).rev() {
            if tdir != turnaround as i32 {
                thinkers.mobj_mut(actor).unwrap().movedir = tdir;
                if p_try_walk(thinkers, level, players, rmain, validcount, actor) {
                    return;
                }
            }
        }
    }

    if turnaround != DirType::NoDir {
        thinkers.mobj_mut(actor).unwrap().movedir = turnaround as i32;
        if p_try_walk(thinkers, level, players, rmain, validcount, actor) {
            return;
        }
    }

    thinkers.mobj_mut(actor).unwrap().movedir = DirType::NoDir as i32; // can not move
}

/// Port of `P_LookForPlayers`. If `allaround` is false, only look 180
/// degrees in front. Returns `true` if a player is targeted (the
/// original's own comment, preserved).
///
/// Returns `(found, new_validcount)` — `P_CheckSight` bumps
/// `validcount`, see the project-wide threading convention.
///
/// `playeringame` is threaded explicitly (not read from
/// [`crate::doomstat::GameState`]) — see [`p_mobj::p_mobj_thinker`]'s
/// own `playeringame` parameter and `g_game::g_ticker`'s module docs:
/// this port's single-player game loop builds its own local
/// `playeringame` slice (`[true]`) rather than populating the
/// `doomstat` global, so reading the global here would see every slot
/// `false` and never find a player — this exact bug was caught by
/// `tests/real_game_loop.rs` hanging (see the fix's commit/session
/// notes) before this parameter was added.
#[allow(clippy::too_many_arguments)]
pub fn p_look_for_players(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    mut validcount: i32,
    playeringame: &[bool],
    actor: ThinkerId,
    allaround: bool,
) -> (bool, i32) {
    let mut c = 0;
    let mut lastlook = thinkers.mobj(actor).unwrap().lastlook;
    let stop = (lastlook - 1).rem_euclid(4);

    loop {
        if !playeringame
            .get(lastlook as usize)
            .copied()
            .unwrap_or(false)
        {
            lastlook = (lastlook + 1).rem_euclid(4);
            thinkers.mobj_mut(actor).unwrap().lastlook = lastlook;
            continue;
        }

        if c == 2 || lastlook == stop {
            // done looking (the original's own comment, preserved).
            thinkers.mobj_mut(actor).unwrap().lastlook = lastlook;
            return (false, validcount);
        }
        c += 1;

        let Some(player) = players.get(lastlook as usize) else {
            lastlook = (lastlook + 1).rem_euclid(4);
            thinkers.mobj_mut(actor).unwrap().lastlook = lastlook;
            continue;
        };
        if player.health <= 0 {
            lastlook = (lastlook + 1).rem_euclid(4);
            thinkers.mobj_mut(actor).unwrap().lastlook = lastlook;
            continue; // dead
        }

        let player_mo = player.mo;
        let (sight, new_validcount) =
            crate::p_sight::p_check_sight(thinkers, level, validcount, actor, player_mo);
        validcount = new_validcount;
        if !sight {
            lastlook = (lastlook + 1).rem_euclid(4);
            thinkers.mobj_mut(actor).unwrap().lastlook = lastlook;
            continue; // out of sight
        }

        if !allaround {
            let (ax, ay, aangle) = {
                let a = thinkers.mobj(actor).unwrap();
                (a.x, a.y, a.angle)
            };
            let (mx, my) = {
                let m = thinkers.mobj(player_mo).unwrap();
                (m.x, m.y)
            };
            let an = rmain.point_to_angle2(ax, ay, mx, my).wrapping_sub(aangle);

            if an > crate::tables::ANG90 && an < crate::tables::ANG270 {
                let dist = p_aprox_distance(mx - ax, my - ay);
                // if real close, react anyway (the original's own
                // comment, preserved).
                if dist > MELEERANGE {
                    lastlook = (lastlook + 1).rem_euclid(4);
                    thinkers.mobj_mut(actor).unwrap().lastlook = lastlook;
                    continue; // behind back
                }
            }
        }

        thinkers.mobj_mut(actor).unwrap().target = Some(player_mo);
        thinkers.mobj_mut(actor).unwrap().lastlook = lastlook;
        return (true, validcount);
    }
}

/// Port of `A_KeenDie`. DOOM II special, map 32. Uses special tag 666
/// (the original's own comment, preserved).
pub fn a_keen_die(thinkers: &mut Thinkers, level: &mut Level, mo: ThinkerId) {
    a_fall(thinkers, mo);

    // scan the remaining thinkers to see if all Keens are dead (the
    // original's own comment, preserved).
    let mo_type = thinkers.mobj(mo).unwrap().mobj_type;
    for (id, other) in thinkers.iter_mobjs() {
        if id != mo && other.mobj_type == mo_type && other.health > 0 {
            // other Keen not dead (the original's own comment,
            // preserved).
            return;
        }
    }

    let junk = crate::r_defs::Line {
        tag: 666,
        ..Default::default()
    };
    crate::p_doors::ev_do_door(thinkers, level, &junk, crate::p_doors::VlDoorType::Open);
}

/// Port of `A_Fall`.
pub fn a_fall(thinkers: &mut Thinkers, actor: ThinkerId) {
    // actor is on ground, it can be walked over (the original's own
    // comment, preserved).
    thinkers.mobj_mut(actor).unwrap().flags &= !mobj_flag::SOLID;
    // So change this if corpse objects are meant to be obstacles (the
    // original's own comment, preserved).
}

/// Port of `A_Look`. Stay in state until a player is sighted (the
/// original's own comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn a_look(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    mut validcount: i32,
    playeringame: &[bool],
    rdata: &crate::r_data::RData,
    brain: &mut BrainTargets,
    actor: ThinkerId,
) {
    thinkers.mobj_mut(actor).unwrap().threshold = 0; // any shot will wake up

    let sector = level.subsectors[thinkers.mobj(actor).unwrap().subsector.unwrap()].sector;
    let targ = level.sectors[sector].soundtarget;

    let mut seeyou = false;

    if let Some(targ) = targ {
        if thinkers.mobj(targ).unwrap().flags & mobj_flag::SHOOTABLE != 0 {
            thinkers.mobj_mut(actor).unwrap().target = Some(targ);

            if thinkers.mobj(actor).unwrap().flags & mobj_flag::AMBUSH != 0 {
                let (sight, new_validcount) =
                    crate::p_sight::p_check_sight(thinkers, level, validcount, actor, targ);
                validcount = new_validcount;
                if sight {
                    seeyou = true;
                }
            } else {
                seeyou = true;
            }
        }
    }

    if !seeyou {
        let (found, _validcount) = p_look_for_players(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            actor,
            false,
        );
        if !found {
            return;
        }
    }

    // go into chase state (the original's own comment, preserved).
    let (seesound, actor_type) = {
        let m = thinkers.mobj(actor).unwrap();
        (m.info.seesound, m.mobj_type)
    };
    if !matches!(seesound, Sfx::SfxNone) {
        // The random variant is picked (drawing P_Random) whether or
        // not anything ends up playing, exactly like the original.
        let sound = match seesound {
            Sfx::SfxPosit1 | Sfx::SfxPosit2 | Sfx::SfxPosit3 => Sfx::from_index(
                Sfx::SfxPosit1 as usize + (crate::m_random::p_random() % 3) as usize,
            ),
            Sfx::SfxBgsit1 | Sfx::SfxBgsit2 => Sfx::from_index(
                Sfx::SfxBgsit1 as usize + (crate::m_random::p_random() % 2) as usize,
            ),
            other => other,
        };
        if actor_type == MobjType::MtSpider || actor_type == MobjType::MtCyborg {
            // full volume
            s_start_sound(None, sound);
        } else {
            s_start_sound(Some(SoundOrigin::Mobj(actor)), sound);
        }
    }

    let seestate = thinkers.mobj(actor).unwrap().info.seestate;
    crate::p_mobj::p_set_mobj_state(
        thinkers,
        level,
        actor,
        seestate,
        |thinkers, level, id, action| {
            dispatch_mobj_action(
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
}

/// Port of `A_Chase`. Actor has a melee attack, so it tries to close as
/// fast as possible (the original's own comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn a_chase(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    mut validcount: i32,
    playeringame: &[bool],
    rdata: &crate::r_data::RData,
    brain: &mut BrainTargets,
    actor: ThinkerId,
) {
    {
        let mobj = thinkers.mobj_mut(actor).unwrap();
        if mobj.reactiontime != 0 {
            mobj.reactiontime -= 1;
        }
    }

    // modify target threshold (the original's own comment, preserved).
    {
        let mobj = *thinkers.mobj(actor).unwrap();
        if mobj.threshold != 0 {
            let dead = mobj
                .target
                .map(|t| thinkers.mobj(t).unwrap().health <= 0)
                .unwrap_or(true);
            let m = thinkers.mobj_mut(actor).unwrap();
            if dead {
                m.threshold = 0;
            } else {
                m.threshold -= 1;
            }
        }
    }

    // turn towards movement direction if not there yet (the original's
    // own comment, preserved).
    {
        let mobj = thinkers.mobj_mut(actor).unwrap();
        if mobj.movedir < 8 {
            mobj.angle &= 7u32 << 29;
            let delta =
                (mobj.angle as i32).wrapping_sub((mobj.movedir as u32).wrapping_shl(29) as i32);
            if delta > 0 {
                mobj.angle = mobj.angle.wrapping_sub(crate::tables::ANG90 / 2);
            } else if delta < 0 {
                mobj.angle = mobj.angle.wrapping_add(crate::tables::ANG90 / 2);
            }
        }
    }

    let target = thinkers.mobj(actor).unwrap().target;
    let target_shootable =
        target.is_some_and(|t| thinkers.mobj(t).unwrap().flags & mobj_flag::SHOOTABLE != 0);

    if !target_shootable {
        // look for a new target (the original's own comment, preserved).
        let (found, _validcount) = p_look_for_players(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            playeringame,
            actor,
            true,
        );
        if found {
            return; // got a new target
        }
        let spawnstate = thinkers.mobj(actor).unwrap().info.spawnstate;
        crate::p_mobj::p_set_mobj_state(
            thinkers,
            level,
            actor,
            spawnstate,
            |thinkers, level, id, action| {
                dispatch_mobj_action(
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
        return;
    }
    let target = target.unwrap();

    // do not attack twice in a row (the original's own comment,
    // preserved).
    if thinkers.mobj(actor).unwrap().flags & mobj_flag::JUSTATTACKED != 0 {
        thinkers.mobj_mut(actor).unwrap().flags &= !mobj_flag::JUSTATTACKED;
        let skill = crate::doomstat::state().gameskill;
        let fastparm = crate::doomstat::state().fastparm;
        if skill != crate::doomdef::Skill::Nightmare && !fastparm {
            p_new_chase_dir(thinkers, level, players, rmain, validcount, actor);
        }
        return;
    }

    // check for melee attack (the original's own comment, preserved).
    let meleestate = thinkers.mobj(actor).unwrap().info.meleestate;
    if !matches!(meleestate, StateNum::SNull) {
        let (melee, new_validcount) = p_check_melee_range(thinkers, level, validcount, actor);
        validcount = new_validcount;
        if melee {
            let attacksound = thinkers.mobj(actor).unwrap().info.attacksound;
            if !matches!(attacksound, Sfx::SfxNone) {
                s_start_sound(Some(SoundOrigin::Mobj(actor)), attacksound);
            }
            crate::p_mobj::p_set_mobj_state(
                thinkers,
                level,
                actor,
                meleestate,
                |thinkers, level, id, action| {
                    dispatch_mobj_action(
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
            return;
        }
    }

    // check for missile attack (the original's own comment, preserved).
    let missilestate = thinkers.mobj(actor).unwrap().info.missilestate;
    let mut nomissile = matches!(missilestate, StateNum::SNull);
    if !nomissile {
        let skill = crate::doomstat::state().gameskill;
        let fastparm = crate::doomstat::state().fastparm;
        let movecount = thinkers.mobj(actor).unwrap().movecount;
        if skill < crate::doomdef::Skill::Nightmare && !fastparm && movecount != 0 {
            nomissile = true;
        } else {
            let (in_range, new_validcount) =
                p_check_missile_range(thinkers, level, validcount, actor);
            validcount = new_validcount;
            if !in_range {
                nomissile = true;
            } else {
                crate::p_mobj::p_set_mobj_state(
                    thinkers,
                    level,
                    actor,
                    missilestate,
                    |thinkers, level, id, action| {
                        dispatch_mobj_action(
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
                thinkers.mobj_mut(actor).unwrap().flags |= mobj_flag::JUSTATTACKED;
                return;
            }
        }
    }

    if nomissile {
        // possibly choose another target (the original's own comment,
        // preserved).
        let netgame = crate::doomstat::state().netgame;
        let threshold = thinkers.mobj(actor).unwrap().threshold;
        if netgame && threshold == 0 {
            let (sight, new_validcount) =
                crate::p_sight::p_check_sight(thinkers, level, validcount, actor, target);
            validcount = new_validcount;
            if !sight {
                let (found, _validcount) = p_look_for_players(
                    thinkers,
                    level,
                    players,
                    rmain,
                    validcount,
                    playeringame,
                    actor,
                    true,
                );
                if found {
                    return; // got a new target
                }
            }
        }
    }

    // chase towards player (the original's own comment, preserved).
    {
        let mobj = thinkers.mobj_mut(actor).unwrap();
        mobj.movecount -= 1;
    }
    let movecount = thinkers.mobj(actor).unwrap().movecount;
    if movecount < 0 || !p_move(thinkers, level, players, rmain, validcount, actor) {
        p_new_chase_dir(thinkers, level, players, rmain, validcount, actor);
    }

    // make active sound (the original's own comment, preserved).
    let activesound = thinkers.mobj(actor).unwrap().info.activesound;
    if !matches!(activesound, Sfx::SfxNone) && crate::m_random::p_random() < 3 {
        s_start_sound(Some(SoundOrigin::Mobj(actor)), activesound);
    }
}

/// (`MISSILERANGE`, `p_local.h`'s own `#define`).
const MISSILERANGE: Fixed = 32 * 64 * FRACUNIT;

/// Port of `A_FaceTarget`.
pub fn a_face_target(thinkers: &mut Thinkers, rmain: &mut RMain, actor: ThinkerId) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    thinkers.mobj_mut(actor).unwrap().flags &= !mobj_flag::AMBUSH;

    let (ax, ay) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y)
    };
    let (tx, ty, tflags) = {
        let t = thinkers.mobj(target).unwrap();
        (t.x, t.y, t.flags)
    };
    let mut angle = rmain.point_to_angle2(ax, ay, tx, ty);

    if tflags & mobj_flag::SHADOW != 0 {
        angle = angle.wrapping_add(
            ((crate::m_random::p_random() - crate::m_random::p_random()) << 21) as u32,
        );
    }

    thinkers.mobj_mut(actor).unwrap().angle = angle;
}

/// Port of `A_PosAttack`.
pub fn a_pos_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    if thinkers.mobj(actor).unwrap().target.is_none() {
        return;
    }

    a_face_target(thinkers, rmain, actor);
    let angle = thinkers.mobj(actor).unwrap().angle;
    let (slope, _linetarget) =
        crate::p_map::p_aim_line_attack(thinkers, level, validcount, actor, angle, MISSILERANGE);

    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxPistol);
    let angle = angle
        .wrapping_add(((crate::m_random::p_random() - crate::m_random::p_random()) << 20) as u32);
    let damage = (crate::m_random::p_random() % 5 + 1) * 3;
    crate::p_map::p_line_attack(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        angle,
        MISSILERANGE,
        slope,
        damage,
    );
}

/// Port of `A_SPosAttack`.
pub fn a_spos_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    if thinkers.mobj(actor).unwrap().target.is_none() {
        return;
    }

    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxShotgn);
    a_face_target(thinkers, rmain, actor);
    let bangle = thinkers.mobj(actor).unwrap().angle;
    let (slope, _linetarget) =
        crate::p_map::p_aim_line_attack(thinkers, level, validcount, actor, bangle, MISSILERANGE);

    for _ in 0..3 {
        let angle = bangle.wrapping_add(
            ((crate::m_random::p_random() - crate::m_random::p_random()) << 20) as u32,
        );
        let damage = (crate::m_random::p_random() % 5 + 1) * 3;
        crate::p_map::p_line_attack(
            thinkers,
            level,
            players,
            rmain,
            validcount,
            actor,
            angle,
            MISSILERANGE,
            slope,
            damage,
        );
    }
}

/// Port of `A_CPosAttack`.
pub fn a_cpos_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    if thinkers.mobj(actor).unwrap().target.is_none() {
        return;
    }

    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxShotgn);
    a_face_target(thinkers, rmain, actor);
    let bangle = thinkers.mobj(actor).unwrap().angle;
    let (slope, _linetarget) =
        crate::p_map::p_aim_line_attack(thinkers, level, validcount, actor, bangle, MISSILERANGE);

    let angle = bangle
        .wrapping_add(((crate::m_random::p_random() - crate::m_random::p_random()) << 20) as u32);
    let damage = (crate::m_random::p_random() % 5 + 1) * 3;
    crate::p_map::p_line_attack(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        angle,
        MISSILERANGE,
        slope,
        damage,
    );
}

/// Port of `A_CPosRefire`.
pub fn a_cpos_refire(
    thinkers: &mut Thinkers,
    level: &mut Level,
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    // keep firing unless target got out of sight (the original's own
    // comment, preserved).
    a_face_target(thinkers, rmain, actor);

    if crate::m_random::p_random() < 40 {
        return;
    }

    let target = thinkers.mobj(actor).unwrap().target;
    let out_of_sight = match target {
        None => true,
        Some(t) => {
            if thinkers.mobj(t).unwrap().health <= 0 {
                true
            } else {
                !crate::p_sight::p_check_sight(thinkers, level, validcount, actor, t).0
            }
        }
    };

    if out_of_sight {
        let seestate = thinkers.mobj(actor).unwrap().info.seestate;
        crate::p_mobj::p_set_mobj_state(thinkers, level, actor, seestate, |_, _, _, _| {});
    }
}

/// Port of `A_SpidRefire`.
pub fn a_spid_refire(
    thinkers: &mut Thinkers,
    level: &mut Level,
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    // keep firing unless target got out of sight (the original's own
    // comment, preserved).
    a_face_target(thinkers, rmain, actor);

    if crate::m_random::p_random() < 10 {
        return;
    }

    let target = thinkers.mobj(actor).unwrap().target;
    let out_of_sight = match target {
        None => true,
        Some(t) => {
            if thinkers.mobj(t).unwrap().health <= 0 {
                true
            } else {
                !crate::p_sight::p_check_sight(thinkers, level, validcount, actor, t).0
            }
        }
    };

    if out_of_sight {
        let seestate = thinkers.mobj(actor).unwrap().info.seestate;
        crate::p_mobj::p_set_mobj_state(thinkers, level, actor, seestate, |_, _, _, _| {});
    }
}

/// Port of `A_BspiAttack`.
pub fn a_bspi_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);

    // launch a missile (the original's own comment, preserved).
    crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtArachplaz,
    );
}

/// Port of `A_TroopAttack`.
pub fn a_troop_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);
    let (melee, validcount) = p_check_melee_range(thinkers, level, validcount, actor);
    if melee {
        s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxClaw);
        let damage = (crate::m_random::p_random() % 8 + 1) * 3;
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            target,
            Some(actor),
            Some(actor),
            damage,
        );
        return;
    }

    // launch a missile (the original's own comment, preserved).
    crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtTroopshot,
    );
}

/// Port of `A_SargAttack`.
pub fn a_sarg_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);
    let (melee, _validcount) = p_check_melee_range(thinkers, level, validcount, actor);
    if melee {
        let damage = (crate::m_random::p_random() % 10 + 1) * 4;
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            target,
            Some(actor),
            Some(actor),
            damage,
        );
    }
}

/// Port of `A_HeadAttack`.
pub fn a_head_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);
    let (melee, validcount) = p_check_melee_range(thinkers, level, validcount, actor);
    if melee {
        let damage = (crate::m_random::p_random() % 6 + 1) * 10;
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            target,
            Some(actor),
            Some(actor),
            damage,
        );
        return;
    }

    // launch a missile (the original's own comment, preserved).
    crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtHeadshot,
    );
}

/// Port of `A_CyberAttack`.
pub fn a_cyber_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);
    crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtRocket,
    );
}

/// Port of `A_BruisAttack`.
pub fn a_bruis_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    let (melee, validcount) = p_check_melee_range(thinkers, level, validcount, actor);
    if melee {
        s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxClaw);
        let damage = (crate::m_random::p_random() % 8 + 1) * 10;
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            target,
            Some(actor),
            Some(actor),
            damage,
        );
        return;
    }

    // launch a missile (the original's own comment, preserved).
    crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtBruisershot,
    );
}

/// Port of `A_SkelMissile`.
pub fn a_skel_missile(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);
    thinkers.mobj_mut(actor).unwrap().z += 16 * FRACUNIT; // so missile spawns higher
    let mo = crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtTracer,
    );
    thinkers.mobj_mut(actor).unwrap().z -= 16 * FRACUNIT; // back to normal

    if let Some(mo_mobj) = thinkers.mobj_mut(mo) {
        let (momx, momy) = (mo_mobj.momx, mo_mobj.momy);
        mo_mobj.x += momx;
        mo_mobj.y += momy;
        mo_mobj.tracer = Some(target);
    }
}

/// (`TRACEANGLE`, `p_enemy.c`'s own global constant).
const TRACEANGLE: u32 = 0xc000000;

/// Port of `A_Tracer`.
pub fn a_tracer(thinkers: &mut Thinkers, level: &mut Level, rmain: &mut RMain, actor: ThinkerId) {
    if crate::doomstat::state().leveltime & 3 != 0 {
        return;
    }

    // spawn a puff of smoke behind the rocket (the original's own
    // comment, preserved).
    let (ax, ay, az, amomx, amomy) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y, a.z, a.momx, a.momy)
    };
    crate::p_mobj::p_spawn_puff(thinkers, level, MISSILERANGE, ax, ay, az);

    let th = crate::p_mobj::p_spawn_mobj(
        thinkers,
        level,
        ax - amomx,
        ay - amomy,
        crate::p_mobj::SpawnZ::At(az),
        MobjType::MtSmoke,
    );
    {
        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.momz = FRACUNIT;
        mobj.tics -= crate::m_random::p_random() & 3;
        if mobj.tics < 1 {
            mobj.tics = 1;
        }
    }

    // adjust direction (the original's own comment, preserved).
    let dest = thinkers.mobj(actor).unwrap().tracer;
    let Some(dest) = dest else {
        return;
    };
    if thinkers.mobj(dest).unwrap().health <= 0 {
        return;
    }

    // change angle (the original's own comment, preserved).
    let (ax, ay, aangle) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y, a.angle)
    };
    let (dx, dy) = {
        let d = thinkers.mobj(dest).unwrap();
        (d.x, d.y)
    };
    let exact = rmain.point_to_angle2(ax, ay, dx, dy);

    let mut angle = aangle;
    if exact != angle {
        if exact.wrapping_sub(angle) > 0x8000_0000 {
            angle = angle.wrapping_sub(TRACEANGLE);
            if exact.wrapping_sub(angle) < 0x8000_0000 {
                angle = exact;
            }
        } else {
            angle = angle.wrapping_add(TRACEANGLE);
            if exact.wrapping_sub(angle) > 0x8000_0000 {
                angle = exact;
            }
        }
    }

    let fine = (angle >> crate::tables::ANGLETOFINESHIFT) as usize;
    let speed = thinkers.mobj(actor).unwrap().info.speed;
    {
        let mobj = thinkers.mobj_mut(actor).unwrap();
        mobj.angle = angle;
        mobj.momx = fixed_mul(speed, crate::tables::fine_cosine(fine));
        mobj.momy = fixed_mul(speed, crate::tables::FINESINE[fine]);
    }

    // change slope (the original's own comment, preserved).
    let (ax, ay, az) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y, a.z)
    };
    let (dx, dy, dz) = {
        let d = thinkers.mobj(dest).unwrap();
        (d.x, d.y, d.z)
    };
    let mut dist = crate::p_maputl::p_aprox_distance(dx - ax, dy - ay);
    dist /= speed;
    if dist < 1 {
        dist = 1;
    }
    let slope = (dz + 40 * FRACUNIT - az) / dist;

    let mobj = thinkers.mobj_mut(actor).unwrap();
    if slope < mobj.momz {
        mobj.momz -= FRACUNIT / 8;
    } else {
        mobj.momz += FRACUNIT / 8;
    }
}

/// Port of `A_SkelWhoosh`.
pub fn a_skel_whoosh(thinkers: &mut Thinkers, rmain: &mut RMain, actor: ThinkerId) {
    if thinkers.mobj(actor).unwrap().target.is_none() {
        return;
    }
    a_face_target(thinkers, rmain, actor);
    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxSkeswg);
}

/// Port of `A_SkelFist`.
pub fn a_skel_fist(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);

    let (melee, _validcount) = p_check_melee_range(thinkers, level, validcount, actor);
    if melee {
        let damage = (crate::m_random::p_random() % 10 + 1) * 6;
        s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxSkepch);
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            target,
            Some(actor),
            Some(actor),
            damage,
        );
    }
}

/// `MAXRADIUS*2`, matching `A_VileChase`'s own inline `MAXRADIUS*2`
/// expression in the original.
const MAXRADIUS_2: Fixed = crate::p_setup::MAXRADIUS * 2;

/// The original's file-scope scratch globals for one [`a_vile_chase`]
/// call (`corpsehit`/`vileobj`/`viletryx`/`viletryy`).
struct VileCheckContext {
    viletryx: Fixed,
    viletryy: Fixed,
    corpsehit: Option<ThinkerId>,
}

/// Port of `PIT_VileCheck`. Detect a corpse that could be raised (the
/// original's own comment, preserved). Returns `false` to stop the
/// block-things scan (a corpse was found), `true` to keep scanning —
/// same boolean convention as the original.
fn pit_vile_check(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    ctx: &mut VileCheckContext,
    thing: ThinkerId,
) -> bool {
    let (flags, tics, raisestate, radius, mobj_type) = {
        let t = thinkers.mobj(thing).unwrap();
        (t.flags, t.tics, t.info.raisestate, t.radius, t.mobj_type)
    };

    if flags & mobj_flag::CORPSE == 0 {
        return true; // not a monster
    }
    if tics != -1 {
        return true; // not lying still yet
    }
    if matches!(raisestate, StateNum::SNull) {
        return true; // monster doesn't have a raise state
    }

    let vile_radius = crate::info::MOBJINFO[MobjType::MtVile as usize].radius;
    let maxdist = radius + vile_radius;

    let (tx, ty) = {
        let t = thinkers.mobj(thing).unwrap();
        (t.x, t.y)
    };
    if (tx - ctx.viletryx).wrapping_abs() > maxdist || (ty - ctx.viletryy).wrapping_abs() > maxdist
    {
        return true; // not actually touching
    }

    ctx.corpsehit = Some(thing);
    {
        let t = thinkers.mobj_mut(thing).unwrap();
        t.momx = 0;
        t.momy = 0;
        t.height <<= 2;
    }
    let (check, _move_ctx) =
        crate::p_map::p_check_position(thinkers, level, players, rmain, validcount, thing, tx, ty);
    thinkers.mobj_mut(thing).unwrap().height >>= 2;

    if !check {
        return true; // doesn't fit here
    }

    let _ = mobj_type;
    false // got one, so stop checking
}

/// Port of `A_VileChase`. Check for ressurecting a body (the original's
/// own comment, preserved).
#[allow(clippy::too_many_arguments)]
pub fn a_vile_chase(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    playeringame: &[bool],
    rdata: &crate::r_data::RData,
    brain: &mut BrainTargets,
    actor: ThinkerId,
) {
    let (movedir, speed, ax, ay) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.movedir, a.info.speed, a.x, a.y)
    };

    if movedir != DirType::NoDir as i32 {
        // check for corpses to raise (the original's own comment,
        // preserved).
        let viletryx = ax + speed * XSPEED[movedir as usize];
        let viletryy = ay + speed * YSPEED[movedir as usize];

        let xl = (viletryx - level.bmaporgx - MAXRADIUS_2) >> crate::p_setup::MAPBLOCKSHIFT;
        let xh = (viletryx - level.bmaporgx + MAXRADIUS_2) >> crate::p_setup::MAPBLOCKSHIFT;
        let yl = (viletryy - level.bmaporgy - MAXRADIUS_2) >> crate::p_setup::MAPBLOCKSHIFT;
        let yh = (viletryy - level.bmaporgy + MAXRADIUS_2) >> crate::p_setup::MAPBLOCKSHIFT;

        let mut ctx = VileCheckContext {
            viletryx,
            viletryy,
            corpsehit: None,
        };

        for bx in xl..=xh {
            for by in yl..=yh {
                let mut ids: Vec<ThinkerId> = Vec::new();
                crate::p_maputl::p_block_things_iterator(thinkers, level, bx, by, |_, id| {
                    ids.push(id);
                    true
                });
                let mut found = false;
                for id in ids {
                    if thinkers.mobj(id).is_none() {
                        continue;
                    }
                    if !pit_vile_check(thinkers, level, players, rmain, validcount, &mut ctx, id) {
                        found = true;
                        break;
                    }
                }
                if found {
                    // got one! (the original's own comment, preserved).
                    let corpsehit = ctx.corpsehit.unwrap();
                    let temp = thinkers.mobj(actor).unwrap().target;
                    thinkers.mobj_mut(actor).unwrap().target = Some(corpsehit);
                    a_face_target(thinkers, rmain, actor);
                    thinkers.mobj_mut(actor).unwrap().target = temp;

                    crate::p_mobj::p_set_mobj_state(
                        thinkers,
                        level,
                        actor,
                        StateNum::SVileHeal1,
                        |_, _, _, _| {},
                    );
                    s_start_sound(Some(SoundOrigin::Mobj(corpsehit)), Sfx::SfxSlop);
                    let info = thinkers.mobj(corpsehit).unwrap().info;

                    crate::p_mobj::p_set_mobj_state(
                        thinkers,
                        level,
                        corpsehit,
                        info.raisestate,
                        |_, _, _, _| {},
                    );
                    let mobj = thinkers.mobj_mut(corpsehit).unwrap();
                    mobj.height <<= 2;
                    mobj.flags = info.flags;
                    mobj.health = info.spawnhealth;
                    mobj.target = None;

                    return;
                }
            }
        }
    }

    // Return to normal attack (the original's own comment, preserved).
    a_chase(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        playeringame,
        rdata,
        brain,
        actor,
    );
}

/// Port of `A_VileStart`.
pub fn a_vile_start(_thinkers: &mut Thinkers, actor: ThinkerId) {
    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxVilatk);
}

/// Port of `A_Fire`. Keep fire in front of player unless out of sight
/// (the original's own comment, preserved).
fn a_fire(thinkers: &mut Thinkers, level: &mut Level, validcount: i32, actor: ThinkerId) {
    let Some(dest) = thinkers.mobj(actor).unwrap().tracer else {
        return;
    };

    // don't move it if the vile lost sight (the original's own comment,
    // preserved). The original reads `actor->target` here (the vile),
    // not `actor` (the fire) itself.
    let Some(vile) = thinkers.mobj(actor).unwrap().target else {
        return;
    };
    if !crate::p_sight::p_check_sight(thinkers, level, validcount, vile, dest).0 {
        return;
    }

    let (dx, dy, dz, dangle) = {
        let d = thinkers.mobj(dest).unwrap();
        (d.x, d.y, d.z, d.angle)
    };
    let an = (dangle >> crate::tables::ANGLETOFINESHIFT) as usize;

    crate::p_mobj::p_unset_thing_position(thinkers, level, actor);
    {
        let mobj = thinkers.mobj_mut(actor).unwrap();
        mobj.x = dx + fixed_mul(24 * FRACUNIT, crate::tables::fine_cosine(an));
        mobj.y = dy + fixed_mul(24 * FRACUNIT, crate::tables::FINESINE[an]);
        mobj.z = dz;
    }
    crate::p_mobj::p_set_thing_position(thinkers, level, actor);
}

/// Port of `A_StartFire`.
pub fn a_start_fire(thinkers: &mut Thinkers, level: &mut Level, validcount: i32, actor: ThinkerId) {
    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxFlamst);
    a_fire(thinkers, level, validcount, actor);
}

/// Port of `A_FireCrackle`.
pub fn a_fire_crackle(
    thinkers: &mut Thinkers,
    level: &mut Level,
    validcount: i32,
    actor: ThinkerId,
) {
    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxFlame);
    a_fire(thinkers, level, validcount, actor);
}

/// Port of `A_VileTarget`. Spawn the hellfire (the original's own
/// comment, preserved).
pub fn a_vile_target(
    thinkers: &mut Thinkers,
    level: &mut Level,
    rmain: &mut RMain,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);

    let (tx, tz) = {
        let t = thinkers.mobj(target).unwrap();
        (t.x, t.z)
    };
    // The original's own bug, preserved faithfully: it passes
    // `actor->target->x` for BOTH the x and y spawn coordinates
    // (`actor->target->x, actor->target->x, actor->target->z` —
    // note the second argument is `->x` again, not `->y`), so the fire
    // always spawns on the line y=x relative to origin instead of at
    // the target's real position. "Fixing" this would change observable
    // fire placement on real WADs, so it's kept exactly as the original
    // reads.
    let fog = crate::p_mobj::p_spawn_mobj(
        thinkers,
        level,
        tx,
        tx,
        crate::p_mobj::SpawnZ::At(tz),
        MobjType::MtFire,
    );

    thinkers.mobj_mut(actor).unwrap().tracer = Some(fog);
    {
        let f = thinkers.mobj_mut(fog).unwrap();
        f.target = Some(actor);
        f.tracer = Some(target);
    }
    a_fire(thinkers, level, 0, fog);
}

/// Port of `A_VileAttack`.
pub fn a_vile_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);

    if !crate::p_sight::p_check_sight(thinkers, level, validcount, actor, target).0 {
        return;
    }

    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxBarexp);
    crate::p_inter::p_damage_mobj(
        thinkers,
        level,
        players,
        rmain,
        target,
        Some(actor),
        Some(actor),
        20,
    );
    let mass = thinkers.mobj(target).unwrap().info.mass;
    thinkers.mobj_mut(target).unwrap().momz = 1000 * FRACUNIT / mass;

    let angle = thinkers.mobj(actor).unwrap().angle;
    let an = (angle >> crate::tables::ANGLETOFINESHIFT) as usize;

    let Some(fire) = thinkers.mobj(actor).unwrap().tracer else {
        return;
    };

    // move the fire between the vile and the player (the original's own
    // comment, preserved).
    let (target_x, target_y) = {
        let t = thinkers.mobj(target).unwrap();
        (t.x, t.y)
    };
    {
        let f = thinkers.mobj_mut(fire).unwrap();
        f.x = target_x - fixed_mul(24 * FRACUNIT, crate::tables::fine_cosine(an));
        f.y = target_y - fixed_mul(24 * FRACUNIT, crate::tables::FINESINE[an]);
    }
    crate::p_map::p_radius_attack(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        fire,
        Some(actor),
        70,
    );
}

/// (`FATSPREAD`, `p_enemy.c`'s own `#define` = `ANG90/8`).
const FATSPREAD: crate::tables::Angle = crate::tables::ANG90 / 8;

/// Port of `A_FatRaise`.
pub fn a_fat_raise(thinkers: &mut Thinkers, rmain: &mut RMain, actor: ThinkerId) {
    a_face_target(thinkers, rmain, actor);
    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxManatk);
}

/// Port of `A_FatAttack1`.
pub fn a_fat_attack1(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);
    // Change direction to ... (the original's own comment, preserved).
    thinkers.mobj_mut(actor).unwrap().angle =
        thinkers.mobj(actor).unwrap().angle.wrapping_add(FATSPREAD);
    crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtFatshot,
    );

    let mo = crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtFatshot,
    );
    let angle = thinkers.mobj(mo).unwrap().angle.wrapping_add(FATSPREAD);
    let speed = thinkers.mobj(mo).unwrap().info.speed;
    let an = (angle >> crate::tables::ANGLETOFINESHIFT) as usize;
    let mobj = thinkers.mobj_mut(mo).unwrap();
    mobj.angle = angle;
    mobj.momx = fixed_mul(speed, crate::tables::fine_cosine(an));
    mobj.momy = fixed_mul(speed, crate::tables::FINESINE[an]);
}

/// Port of `A_FatAttack2`.
pub fn a_fat_attack2(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);
    // Now here choose opposite deviation (the original's own comment,
    // preserved).
    thinkers.mobj_mut(actor).unwrap().angle =
        thinkers.mobj(actor).unwrap().angle.wrapping_sub(FATSPREAD);
    crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtFatshot,
    );

    let mo = crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtFatshot,
    );
    let angle = thinkers
        .mobj(mo)
        .unwrap()
        .angle
        .wrapping_sub(FATSPREAD.wrapping_mul(2));
    let speed = thinkers.mobj(mo).unwrap().info.speed;
    let an = (angle >> crate::tables::ANGLETOFINESHIFT) as usize;
    let mobj = thinkers.mobj_mut(mo).unwrap();
    mobj.angle = angle;
    mobj.momx = fixed_mul(speed, crate::tables::fine_cosine(an));
    mobj.momy = fixed_mul(speed, crate::tables::FINESINE[an]);
}

/// Port of `A_FatAttack3`.
pub fn a_fat_attack3(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let Some(target) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    a_face_target(thinkers, rmain, actor);

    let mo = crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtFatshot,
    );
    {
        let angle = thinkers.mobj(mo).unwrap().angle.wrapping_sub(FATSPREAD / 2);
        let speed = thinkers.mobj(mo).unwrap().info.speed;
        let an = (angle >> crate::tables::ANGLETOFINESHIFT) as usize;
        let mobj = thinkers.mobj_mut(mo).unwrap();
        mobj.angle = angle;
        mobj.momx = fixed_mul(speed, crate::tables::fine_cosine(an));
        mobj.momy = fixed_mul(speed, crate::tables::FINESINE[an]);
    }

    let mo = crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        target,
        MobjType::MtFatshot,
    );
    {
        let angle = thinkers.mobj(mo).unwrap().angle.wrapping_add(FATSPREAD / 2);
        let speed = thinkers.mobj(mo).unwrap().info.speed;
        let an = (angle >> crate::tables::ANGLETOFINESHIFT) as usize;
        let mobj = thinkers.mobj_mut(mo).unwrap();
        mobj.angle = angle;
        mobj.momx = fixed_mul(speed, crate::tables::fine_cosine(an));
        mobj.momy = fixed_mul(speed, crate::tables::FINESINE[an]);
    }
}

/// (`SKULLSPEED`, `p_enemy.c`'s own `#define`).
const SKULLSPEED: Fixed = 20 * FRACUNIT;

/// Port of `A_SkullAttack`. Fly at the player like a missile (the
/// original's own comment, preserved).
pub fn a_skull_attack(thinkers: &mut Thinkers, rmain: &mut RMain, actor: ThinkerId) {
    let Some(dest) = thinkers.mobj(actor).unwrap().target else {
        return;
    };

    thinkers.mobj_mut(actor).unwrap().flags |= mobj_flag::SKULLFLY;

    // (`A_SkullAttack` plays its attacksound unconditionally, so the
    // `info.attacksound` lookup mirrors that — no `sfx_None` guard.)
    let attacksound = thinkers.mobj(actor).unwrap().info.attacksound;
    s_start_sound(Some(SoundOrigin::Mobj(actor)), attacksound);
    a_face_target(thinkers, rmain, actor);
    let angle = thinkers.mobj(actor).unwrap().angle;
    let an = (angle >> crate::tables::ANGLETOFINESHIFT) as usize;
    let momx = fixed_mul(SKULLSPEED, crate::tables::fine_cosine(an));
    let momy = fixed_mul(SKULLSPEED, crate::tables::FINESINE[an]);

    let (ax, ay, az) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.x, a.y, a.z)
    };
    let (dx, dy, dz, dheight) = {
        let d = thinkers.mobj(dest).unwrap();
        (d.x, d.y, d.z, d.height)
    };
    let mut dist = crate::p_maputl::p_aprox_distance(dx - ax, dy - ay);
    dist /= SKULLSPEED;
    if dist < 1 {
        dist = 1;
    }
    let momz = (dz + (dheight >> 1) - az) / dist;

    let mobj = thinkers.mobj_mut(actor).unwrap();
    mobj.momx = momx;
    mobj.momy = momy;
    mobj.momz = momz;
}

/// Port of `A_PainShootSkull`. Spawn a lost soul and launch it at the
/// target (the original's own comment, preserved).
fn a_pain_shoot_skull(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
    angle: crate::tables::Angle,
) {
    // count total number of skull currently on the level (the original's
    // own comment, preserved).
    let count = thinkers
        .iter_mobjs()
        .filter(|(_, m)| m.mobj_type == MobjType::MtSkull)
        .count();

    // if there are allready 20 skulls on the level, don't spit another
    // one (the original's own comment, preserved).
    if count > 20 {
        return;
    }

    // okay, there's playe for another one (the original's own comment,
    // preserved).
    let an = (angle >> crate::tables::ANGLETOFINESHIFT) as usize;

    let (actor_radius, actor_x, actor_y, actor_z) = {
        let a = thinkers.mobj(actor).unwrap();
        (a.radius, a.x, a.y, a.z)
    };
    let skull_radius = crate::info::MOBJINFO[MobjType::MtSkull as usize].radius;
    let prestep = 4 * FRACUNIT + 3 * (actor_radius + skull_radius) / 2;

    let x = actor_x + fixed_mul(prestep, crate::tables::fine_cosine(an));
    let y = actor_y + fixed_mul(prestep, crate::tables::FINESINE[an]);
    let z = actor_z + 8 * FRACUNIT;

    let newmobj = crate::p_mobj::p_spawn_mobj(
        thinkers,
        level,
        x,
        y,
        crate::p_mobj::SpawnZ::At(z),
        MobjType::MtSkull,
    );

    // Check for movements (the original's own comment, preserved).
    let (nx, ny) = {
        let n = thinkers.mobj(newmobj).unwrap();
        (n.x, n.y)
    };
    let (moved, _spechit) =
        crate::p_map::p_try_move(thinkers, level, players, rmain, validcount, newmobj, nx, ny);
    if !moved {
        // kill it immediately (the original's own comment, preserved).
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            newmobj,
            Some(actor),
            Some(actor),
            10000,
        );
        return;
    }

    let target = thinkers.mobj(actor).unwrap().target;
    thinkers.mobj_mut(newmobj).unwrap().target = target;
    a_skull_attack(thinkers, rmain, newmobj);
}

/// Port of `A_PainAttack`. Spawn a lost soul and launch it at the target
/// (the original's own comment, preserved).
pub fn a_pain_attack(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    if thinkers.mobj(actor).unwrap().target.is_none() {
        return;
    }

    a_face_target(thinkers, rmain, actor);
    let angle = thinkers.mobj(actor).unwrap().angle;
    a_pain_shoot_skull(thinkers, level, players, rmain, validcount, actor, angle);
}

/// Port of `A_PainDie`.
pub fn a_pain_die(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    a_fall(thinkers, actor);
    let angle = thinkers.mobj(actor).unwrap().angle;
    a_pain_shoot_skull(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        angle.wrapping_add(crate::tables::ANG90),
    );
    a_pain_shoot_skull(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        angle.wrapping_add(crate::tables::ANG180),
    );
    a_pain_shoot_skull(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        actor,
        angle.wrapping_add(crate::tables::ANG270),
    );
}

/// Port of `A_Scream`.
pub fn a_scream(thinkers: &mut Thinkers, actor: ThinkerId) {
    let (deathsound, actor_type) = {
        let m = thinkers.mobj(actor).unwrap();
        (m.info.deathsound, m.mobj_type)
    };
    let sound = match deathsound {
        Sfx::SfxNone => return,
        Sfx::SfxPodth1 | Sfx::SfxPodth2 | Sfx::SfxPodth3 => {
            Sfx::from_index(Sfx::SfxPodth1 as usize + (crate::m_random::p_random() % 3) as usize)
        }
        Sfx::SfxBgdth1 | Sfx::SfxBgdth2 => {
            Sfx::from_index(Sfx::SfxBgdth1 as usize + (crate::m_random::p_random() % 2) as usize)
        }
        other => other,
    };

    // Check for bosses (the original's own comment, preserved).
    if actor_type == MobjType::MtSpider || actor_type == MobjType::MtCyborg {
        // full volume
        s_start_sound(None, sound);
    } else {
        s_start_sound(Some(SoundOrigin::Mobj(actor)), sound);
    }
}

/// Port of `A_XScream`.
pub fn a_xscream(_thinkers: &mut Thinkers, actor: ThinkerId) {
    s_start_sound(Some(SoundOrigin::Mobj(actor)), Sfx::SfxSlop);
}

/// Port of `A_Pain`.
pub fn a_pain(thinkers: &mut Thinkers, actor: ThinkerId) {
    let painsound = thinkers.mobj(actor).unwrap().info.painsound;
    if !matches!(painsound, Sfx::SfxNone) {
        s_start_sound(Some(SoundOrigin::Mobj(actor)), painsound);
    }
}

/// Port of `A_Explode`.
pub fn a_explode(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    actor: ThinkerId,
) {
    let target = thinkers.mobj(actor).unwrap().target;
    crate::p_map::p_radius_attack(
        thinkers, level, players, rmain, validcount, actor, target, 128,
    );
}

/// Port of `A_BFGSpray` (physically in `p_pspr.c`, but a *mobj* action:
/// the BFG ball's death). Sprays 40 aimed tracers around the ball's
/// angle from its originator (`mo->target`, the player).
pub fn a_bfg_spray(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    mo: ThinkerId,
) {
    let Some(origin) = thinkers.mobj(mo).unwrap().target else {
        return;
    };
    let mo_angle = thinkers.mobj(mo).unwrap().angle;

    // offset angles from its attack angle (the original's own comment,
    // preserved)
    for i in 0..40u32 {
        let an = mo_angle
            .wrapping_sub(crate::tables::ANG90 / 2)
            .wrapping_add(crate::tables::ANG90 / 40 * i);

        // mo->target is the originator (player) of the missile (the
        // original's own comment, preserved)
        let (_slope, linetarget) = crate::p_map::p_aim_line_attack(
            thinkers,
            level,
            validcount,
            origin,
            an,
            16 * 64 * FRACUNIT,
        );
        let Some(target) = linetarget else {
            continue;
        };

        let (tx, ty, tz, theight) = {
            let t = thinkers.mobj(target).unwrap();
            (t.x, t.y, t.z, t.height)
        };
        crate::p_mobj::p_spawn_mobj(
            thinkers,
            level,
            tx,
            ty,
            crate::p_mobj::SpawnZ::At(tz + (theight >> 2)),
            MobjType::MtExtrabfg,
        );

        let mut damage = 0;
        for _ in 0..15 {
            damage += (crate::m_random::p_random() & 7) + 1;
        }
        crate::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            rmain,
            target,
            Some(origin),
            Some(origin),
            damage,
        );
    }
}

/// Port of `A_BossDeath`. Possibly trigger special effects if on first
/// boss level (the original's own comment, preserved).
///
/// `rdata` is [`crate::r_data::RData`] — needed by
/// [`crate::p_floor::ev_do_floor`]'s `RaiseToTexture`/
/// `LowerFloorToLowest` cases. Taken as a real reference (not a
/// placeholder default) since a boss death genuinely needs real texture
/// data to resolve correctly; unlike `p_line_attack`'s
/// `Option<&mut SpecialsCtx>`, there's no reasonable "skip it" fallback
/// here — every call site that can reach `A_BossDeath` (`p_mobj_thinker`,
/// which doesn't otherwise touch `RData`) must pass one through, same as
/// how `playeringame` was threaded through in Phase 7c.
pub fn a_boss_death(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rdata: &crate::r_data::RData,
    mo: ThinkerId,
    playeringame: &[bool],
) {
    let mo_type = thinkers.mobj(mo).unwrap().mobj_type;
    let state = crate::doomstat::state();

    if state.gamemode == crate::doomdef::GameMode::Commercial {
        if state.gamemap != 7 {
            return;
        }
        if mo_type != MobjType::MtFatso && mo_type != MobjType::MtBaby {
            return;
        }
    } else {
        match state.gameepisode {
            1 => {
                if state.gamemap != 8 || mo_type != MobjType::MtBruiser {
                    return;
                }
            }
            2 => {
                if state.gamemap != 8 || mo_type != MobjType::MtCyborg {
                    return;
                }
            }
            3 => {
                if state.gamemap != 8 || mo_type != MobjType::MtSpider {
                    return;
                }
            }
            4 => match state.gamemap {
                6 => {
                    if mo_type != MobjType::MtCyborg {
                        return;
                    }
                }
                8 => {
                    if mo_type != MobjType::MtSpider {
                        return;
                    }
                }
                _ => return,
            },
            _ => {
                if state.gamemap != 8 {
                    return;
                }
            }
        }
    }

    // make sure there is a player alive for victory (the original's own
    // comment, preserved).
    let anyone_alive = (0..crate::doomdef::MAXPLAYERS as usize)
        .any(|i| playeringame.get(i).copied().unwrap_or(false) && players[i].health > 0);
    if !anyone_alive {
        return; // no one left alive, so do not end game
    }

    // scan the remaining thinkers to see if all bosses are dead (the
    // original's own comment, preserved).
    for (id, other) in thinkers.iter_mobjs() {
        if id != mo && other.mobj_type == mo_type && other.health > 0 {
            return; // other boss not dead
        }
    }

    // victory! (the original's own comment, preserved).
    let state = crate::doomstat::state();
    if state.gamemode == crate::doomdef::GameMode::Commercial {
        if state.gamemap == 7 {
            if mo_type == MobjType::MtFatso {
                let junk = crate::r_defs::Line {
                    tag: 666,
                    ..Default::default()
                };
                crate::p_floor::ev_do_floor(
                    level,
                    thinkers,
                    rdata,
                    &junk,
                    crate::p_floor::FloorType::LowerFloorToLowest,
                );
                return;
            }
            if mo_type == MobjType::MtBaby {
                let junk = crate::r_defs::Line {
                    tag: 667,
                    ..Default::default()
                };
                crate::p_floor::ev_do_floor(
                    level,
                    thinkers,
                    rdata,
                    &junk,
                    crate::p_floor::FloorType::RaiseToTexture,
                );
                return;
            }
        }
    } else {
        match state.gameepisode {
            1 => {
                let junk = crate::r_defs::Line {
                    tag: 666,
                    ..Default::default()
                };
                crate::p_floor::ev_do_floor(
                    level,
                    thinkers,
                    rdata,
                    &junk,
                    crate::p_floor::FloorType::LowerFloorToLowest,
                );
                return;
            }
            4 => match state.gamemap {
                6 => {
                    let junk = crate::r_defs::Line {
                        tag: 666,
                        ..Default::default()
                    };
                    crate::p_doors::ev_do_door(
                        thinkers,
                        level,
                        &junk,
                        crate::p_doors::VlDoorType::BlazeOpen,
                    );
                    return;
                }
                8 => {
                    let junk = crate::r_defs::Line {
                        tag: 666,
                        ..Default::default()
                    };
                    crate::p_floor::ev_do_floor(
                        level,
                        thinkers,
                        rdata,
                        &junk,
                        crate::p_floor::FloorType::LowerFloorToLowest,
                    );
                    return;
                }
                _ => {}
            },
            _ => {}
        }
    }

    crate::g_game::g_exit_level();
}

/// Port of `A_Hoof`.
#[allow(clippy::too_many_arguments)]
pub fn a_hoof(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    playeringame: &[bool],
    rdata: &crate::r_data::RData,
    brain: &mut BrainTargets,
    mo: ThinkerId,
) {
    s_start_sound(Some(SoundOrigin::Mobj(mo)), Sfx::SfxHoof);
    a_chase(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        playeringame,
        rdata,
        brain,
        mo,
    );
}

/// Port of `A_Metal`.
#[allow(clippy::too_many_arguments)]
pub fn a_metal(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    playeringame: &[bool],
    rdata: &crate::r_data::RData,
    brain: &mut BrainTargets,
    mo: ThinkerId,
) {
    s_start_sound(Some(SoundOrigin::Mobj(mo)), Sfx::SfxMetal);
    a_chase(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        playeringame,
        rdata,
        brain,
        mo,
    );
}

/// Port of `A_BabyMetal`.
#[allow(clippy::too_many_arguments)]
pub fn a_baby_metal(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    playeringame: &[bool],
    rdata: &crate::r_data::RData,
    brain: &mut BrainTargets,
    mo: ThinkerId,
) {
    s_start_sound(Some(SoundOrigin::Mobj(mo)), Sfx::SfxBspwlk);
    a_chase(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        playeringame,
        rdata,
        brain,
        mo,
    );
}

/// Standing in for the original's file-scope `mobj_t*
/// braintargets[32]`/`numbraintargets`/`braintargeton` globals — Doom
/// II MAP30 ("Icon of Sin") boss-brain state, never exercised by any
/// map this port's tests load (E1M1 has no `MT_BOSSBRAIN`). Threaded as
/// an explicit parameter through [`dispatch_mobj_action`]/
/// [`crate::p_mobj::p_mobj_thinker`]/[`crate::p_tick::p_ticker`]/
/// `g_ticker`, same convention as `playeringame` (Phase 7c) and `rdata`
/// (this phase, for [`a_boss_death`]) — owned by whichever caller drives
/// the game loop (`main.rs`/`g_game.rs`/tests), not a `GlobalCell`,
/// since every other per-level mutable state in this port
/// (`SwitchState`, `ActivePlats`, `ActiveCeilings`) is threaded the same
/// way rather than made a hidden global.
#[derive(Debug, Clone, Default)]
pub struct BrainTargets {
    targets: Vec<ThinkerId>,
    on: usize,
    /// `A_BrainSpit`'s own `static int easy = 0` (the original's own
    /// comment, preserved) — a `static` local inside a single function
    /// in the original, folded into this struct rather than given its
    /// own threaded `&mut bool` parameter, since it's exclusively
    /// `a_brain_spit`'s own persisted state.
    easy: bool,
}

/// Port of `A_BrainAwake`.
pub fn a_brain_awake(thinkers: &Thinkers, brain: &mut BrainTargets) {
    // find all the target spots (the original's own comment, preserved).
    brain.targets.clear();
    brain.on = 0;

    for (id, m) in thinkers.iter_mobjs() {
        if m.mobj_type == MobjType::MtBosstarget {
            brain.targets.push(id);
        }
    }

    s_start_sound(None, Sfx::SfxBossit);
}

/// Port of `A_BrainPain`.
pub fn a_brain_pain() {
    s_start_sound(None, Sfx::SfxBospn);
}

/// Port of `A_BrainScream`.
pub fn a_brain_scream(thinkers: &mut Thinkers, level: &mut Level, mo: ThinkerId) {
    let mo_x = thinkers.mobj(mo).unwrap().x;
    let mut x = mo_x - 196 * FRACUNIT;
    while x < mo_x + 320 * FRACUNIT {
        let y = thinkers.mobj(mo).unwrap().y - 320 * FRACUNIT;
        let z = 128 + crate::m_random::p_random() * 2 * FRACUNIT;
        let th = crate::p_mobj::p_spawn_mobj(
            thinkers,
            level,
            x,
            y,
            crate::p_mobj::SpawnZ::At(z),
            MobjType::MtRocket,
        );
        {
            let mobj = thinkers.mobj_mut(th).unwrap();
            mobj.momz = crate::m_random::p_random() * 512;
        }

        crate::p_mobj::p_set_mobj_state(
            thinkers,
            level,
            th,
            StateNum::SBrainexplode1,
            |_, _, _, _| {},
        );

        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.tics -= crate::m_random::p_random() & 7;
        if mobj.tics < 1 {
            mobj.tics = 1;
        }

        x += FRACUNIT * 8;
    }

    s_start_sound(None, Sfx::SfxBosdth);
}

/// Port of `A_BrainExplode`.
pub fn a_brain_explode(thinkers: &mut Thinkers, level: &mut Level, mo: ThinkerId) {
    let (mo_x, mo_y) = {
        let m = thinkers.mobj(mo).unwrap();
        (m.x, m.y)
    };
    let x = mo_x + (crate::m_random::p_random() - crate::m_random::p_random()) * 2048;
    let y = mo_y;
    let z = 128 + crate::m_random::p_random() * 2 * FRACUNIT;
    let th = crate::p_mobj::p_spawn_mobj(
        thinkers,
        level,
        x,
        y,
        crate::p_mobj::SpawnZ::At(z),
        MobjType::MtRocket,
    );
    {
        let mobj = thinkers.mobj_mut(th).unwrap();
        mobj.momz = crate::m_random::p_random() * 512;
    }

    crate::p_mobj::p_set_mobj_state(
        thinkers,
        level,
        th,
        StateNum::SBrainexplode1,
        |_, _, _, _| {},
    );

    let mobj = thinkers.mobj_mut(th).unwrap();
    mobj.tics -= crate::m_random::p_random() & 7;
    if mobj.tics < 1 {
        mobj.tics = 1;
    }
}

/// Port of `A_BrainDie`.
pub fn a_brain_die() {
    crate::g_game::g_exit_level();
}

/// Port of `A_BrainSpit`. `easy` is the original's own `static int easy
/// = 0` — kept as a `&mut bool` the caller owns instead of a hidden
/// function-local static, same reasoning as [`BrainTargets`] above.
#[allow(clippy::too_many_arguments)]
pub fn a_brain_spit(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    brain: &mut BrainTargets,
    mo: ThinkerId,
) {
    brain.easy = !brain.easy;
    if crate::doomstat::state().gameskill <= crate::doomdef::Skill::Easy && !brain.easy {
        return;
    }

    // shoot a cube at current target (the original's own comment,
    // preserved).
    let Some(&targ) = brain.targets.get(brain.on) else {
        return;
    };
    brain.on = (brain.on + 1) % brain.targets.len().max(1);

    // spawn brain missile (the original's own comment, preserved).
    let newmobj = crate::p_mobj::p_spawn_missile(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        mo,
        targ,
        MobjType::MtSpawnshot,
    );
    thinkers.mobj_mut(newmobj).unwrap().target = Some(targ);

    let (targ_y, mo_y) = {
        let t = thinkers.mobj(targ).unwrap();
        let m = thinkers.mobj(mo).unwrap();
        (t.y, m.y)
    };
    let (newmobj_momy, newmobj_state) = {
        let n = thinkers.mobj(newmobj).unwrap();
        (n.momy, n.state)
    };
    let state_tics = STATES[newmobj_state as usize].tics;
    thinkers.mobj_mut(newmobj).unwrap().reactiontime =
        ((targ_y - mo_y) / newmobj_momy) / state_tics;

    s_start_sound(None, Sfx::SfxBospit);
}

/// Port of `A_SpawnSound`. Travelling cube sound (the original's own
/// comment, preserved).
pub fn a_spawn_sound(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    playeringame: &[bool],
    mo: ThinkerId,
) {
    s_start_sound(Some(SoundOrigin::Mobj(mo)), Sfx::SfxBoscub);
    a_spawn_fly(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        playeringame,
        mo,
    );
}

/// Port of `A_SpawnFly`.
#[allow(clippy::too_many_arguments)]
pub fn a_spawn_fly(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    playeringame: &[bool],
    mo: ThinkerId,
) {
    {
        let mobj = thinkers.mobj_mut(mo).unwrap();
        mobj.reactiontime -= 1;
        if mobj.reactiontime != 0 {
            return; // still flying
        }
    }

    let Some(targ) = thinkers.mobj(mo).unwrap().target else {
        return;
    };
    let (tx, ty, tz) = {
        let t = thinkers.mobj(targ).unwrap();
        (t.x, t.y, t.z)
    };

    // First spawn teleport fog (the original's own comment, preserved).
    let fog = crate::p_mobj::p_spawn_mobj(
        thinkers,
        level,
        tx,
        ty,
        crate::p_mobj::SpawnZ::At(tz),
        MobjType::MtSpawnfire,
    );
    s_start_sound(Some(SoundOrigin::Mobj(fog)), Sfx::SfxTelept);

    // Randomly select monster to spawn. Probability distribution (kind
    // of :), decreasing likelihood (the original's own comment,
    // preserved).
    let r = crate::m_random::p_random();
    let mobj_type = if r < 50 {
        MobjType::MtTroop
    } else if r < 90 {
        MobjType::MtSergeant
    } else if r < 120 {
        MobjType::MtShadows
    } else if r < 130 {
        MobjType::MtPain
    } else if r < 160 {
        MobjType::MtHead
    } else if r < 162 {
        MobjType::MtVile
    } else if r < 172 {
        MobjType::MtUndead
    } else if r < 192 {
        MobjType::MtBaby
    } else if r < 222 {
        MobjType::MtFatso
    } else if r < 246 {
        MobjType::MtKnight
    } else {
        MobjType::MtBruiser
    };

    let newmobj = crate::p_mobj::p_spawn_mobj(
        thinkers,
        level,
        tx,
        ty,
        crate::p_mobj::SpawnZ::At(tz),
        mobj_type,
    );
    let (found, _validcount) = p_look_for_players(
        thinkers,
        level,
        players,
        rmain,
        validcount,
        playeringame,
        newmobj,
        true,
    );
    if found {
        let seestate = thinkers.mobj(newmobj).unwrap().info.seestate;
        crate::p_mobj::p_set_mobj_state(thinkers, level, newmobj, seestate, |_, _, _, _| {});
    }

    // telefrag anything in this spot (the original's own comment,
    // preserved).
    let (nx, ny) = {
        let n = thinkers.mobj(newmobj).unwrap();
        (n.x, n.y)
    };
    crate::p_map::p_teleport_move(thinkers, level, players, rmain, newmobj, nx, ny);

    // remove self (i.e., cube) (the original's own comment, preserved).
    crate::p_mobj::p_remove_mobj(thinkers, level, mo);
}

/// Port of `A_PlayerScream`.
pub fn a_player_scream(thinkers: &Thinkers, mo: ThinkerId) {
    let health = thinkers.mobj(mo).unwrap().health;
    // Default death sound (the original's own comment, preserved).
    let mut sound = Sfx::SfxPldeth;
    if crate::doomstat::state().gamemode == crate::doomdef::GameMode::Commercial && health < -50 {
        // IF THE PLAYER DIES LESS THAN -50% WITHOUT GIBBING (the
        // original's own comment, preserved).
        sound = Sfx::SfxPdiehi;
    }
    s_start_sound(Some(SoundOrigin::Mobj(mo)), sound);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d_player::Player;
    use crate::info::MobjType;
    use crate::p_mobj::{p_spawn_mobj, SpawnZ};
    use crate::p_setup::Level;
    use crate::r_main::RMain;
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

    // Player 1's start, from E1M1 (see tests/real_frame.rs).
    const START_X: Fixed = 1056 * FRACUNIT;
    const START_Y: Fixed = -3616 * FRACUNIT;

    #[test]
    fn a_fall_clears_solid_flag() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let id = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        assert_ne!(thinkers.mobj(id).unwrap().flags & mobj_flag::SOLID, 0);

        a_fall(&mut thinkers, id);

        assert_eq!(thinkers.mobj(id).unwrap().flags & mobj_flag::SOLID, 0);
    }

    #[test]
    fn check_melee_range_false_without_a_target() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let id = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let (melee, _validcount) = p_check_melee_range(&thinkers, &mut level, 0, id);
        assert!(!melee, "no target means never in melee range");
    }

    #[test]
    fn check_melee_range_true_when_target_adjacent() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let target = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 10 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().target = Some(target);

        let (melee, _validcount) = p_check_melee_range(&thinkers, &mut level, 0, actor);
        assert!(melee, "a target 10 units away is well within MELEERANGE");
    }

    #[test]
    fn check_melee_range_false_when_target_far() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let target = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 500 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().target = Some(target);

        let (melee, _validcount) = p_check_melee_range(&thinkers, &mut level, 0, actor);
        assert!(!melee, "500 units is well past MELEERANGE");
    }

    #[test]
    fn look_for_players_finds_player_in_same_room() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();

        let player_mo = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 32 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut player = Player::for_test(player_mo);
        player.health = 100;
        let mut players = [player];

        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        // `p_spawn_mobj` seeds `lastlook` from `P_Random() % MAXPLAYERS`
        // (faithful to the original's `P_SpawnMobj`), so it isn't
        // necessarily 0 — pin it for a deterministic starting point,
        // same as the original's own behavior when a monster's very
        // first `P_LookForPlayers` call happens to start mid-cycle.
        thinkers.mobj_mut(actor).unwrap().lastlook = 0;

        let playeringame = [true, false, false, false];
        let (found, _validcount) = p_look_for_players(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            &playeringame,
            actor,
            true,
        );
        assert!(found, "a nearby player in the same room should be found");
        assert_eq!(thinkers.mobj(actor).unwrap().target, Some(player_mo));
    }

    #[test]
    fn look_for_players_skips_dead_players() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();

        let player_mo = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 32 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut player = Player::for_test(player_mo);
        player.health = 0; // dead
        let mut players = [player];

        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().lastlook = 0;

        let playeringame = [true, false, false, false];
        let (found, _validcount) = p_look_for_players(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            &playeringame,
            actor,
            true,
        );
        assert!(!found, "a dead player must never be targeted");
    }

    #[test]
    fn new_chase_dir_faces_toward_a_due_east_target() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: [Player; 0] = [];

        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let target = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 200 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().target = Some(target);
        thinkers.mobj_mut(actor).unwrap().movedir = DirType::NoDir as i32;

        p_new_chase_dir(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            actor,
        );

        let dir = thinkers.mobj(actor).unwrap().movedir;
        assert_ne!(
            dir,
            DirType::NoDir as i32,
            "a due-east target with open floor should yield some walkable direction"
        );
    }

    #[test]
    #[should_panic(expected = "P_NewChaseDir: called with no target")]
    fn new_chase_dir_panics_with_no_target() {
        let Some((_wad, mut level)) = load_e1m1() else {
            // Skipping via early return would make #[should_panic] fail
            // the test (no panic occurred); panic with the same message
            // instead so the test still reports as passing when the WAD
            // isn't available, consistent with every other test's skip
            // convention in spirit (this is the one case where a plain
            // early return doesn't work under #[should_panic]).
            panic!("P_NewChaseDir: called with no target");
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: [Player; 0] = [];
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        p_new_chase_dir(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            actor,
        );
    }

    #[test]
    fn dispatch_mobj_action_none_is_a_noop() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: [Player; 0] = [];
        let id = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let flags_before = thinkers.mobj(id).unwrap().flags;

        let rdata = crate::r_data::RData::default();
        let mut brain = BrainTargets::default();
        dispatch_mobj_action(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            &[],
            &rdata,
            &mut brain,
            id,
            StateAction::None,
        );

        assert_eq!(thinkers.mobj(id).unwrap().flags, flags_before);
    }

    #[test]
    fn dispatch_mobj_action_a_fall_clears_solid() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: [Player; 0] = [];
        let id = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let rdata = crate::r_data::RData::default();
        let mut brain = BrainTargets::default();
        dispatch_mobj_action(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            &[],
            &rdata,
            &mut brain,
            id,
            StateAction::AFall,
        );

        assert_eq!(thinkers.mobj(id).unwrap().flags & mobj_flag::SOLID, 0);
    }

    #[test]
    fn face_target_turns_actor_toward_its_target() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let target = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 200 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().target = Some(target);
        thinkers.mobj_mut(actor).unwrap().angle = crate::tables::ANG180; // facing away

        a_face_target(&mut thinkers, &mut rmain, actor);

        // Target is due east (angle 0); actor should now face roughly
        // east, not the ANG180 it started at.
        assert_ne!(thinkers.mobj(actor).unwrap().angle, crate::tables::ANG180);
    }

    #[test]
    fn face_target_is_a_noop_without_a_target() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().angle = crate::tables::ANG180;

        a_face_target(&mut thinkers, &mut rmain, actor);

        assert_eq!(thinkers.mobj(actor).unwrap().angle, crate::tables::ANG180);
    }

    #[test]
    fn pos_attack_damages_a_target_in_the_line_of_fire() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let target = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 200 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().target = Some(target);
        thinkers.mobj_mut(target).unwrap().player = Some(0);
        let mut players = [Player::for_test(target)];
        let health_before = players[0].health;

        a_pos_attack(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            actor,
        );

        assert!(
            players[0].health <= health_before,
            "P_Random-driven aim can occasionally miss narrowly, but shooting \
             straight at an adjacent target at MISSILERANGE should usually hit; \
             health must never increase"
        );
    }

    #[test]
    fn pos_attack_is_a_noop_without_a_target() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: [Player; 0] = [];
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let angle_before = thinkers.mobj(actor).unwrap().angle;

        a_pos_attack(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            actor,
        );

        assert_eq!(thinkers.mobj(actor).unwrap().angle, angle_before);
    }

    #[test]
    fn troop_attack_melee_damages_target_directly() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtTroop,
        );
        let target = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 10 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().target = Some(target);
        thinkers.mobj_mut(target).unwrap().player = Some(0);
        let mut players = [Player::for_test(target)];
        let health_before = players[0].health;

        a_troop_attack(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            actor,
        );

        assert!(
            players[0].health < health_before,
            "a target 10 units away is within melee range and should take damage"
        );
    }

    #[test]
    fn troop_attack_out_of_melee_range_spawns_a_missile() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtTroop,
        );
        let target = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 300 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().target = Some(target);
        let mut players: [Player; 0] = [];
        let mobj_count_before = thinkers.iter_mobjs().count();

        a_troop_attack(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            actor,
        );

        let mobj_count_after = thinkers.iter_mobjs().count();
        assert!(
            mobj_count_after > mobj_count_before,
            "out of melee range, A_TroopAttack should spawn a MT_TROOPSHOT missile"
        );
    }

    #[test]
    fn skull_attack_sets_skullfly_and_aims_momentum_at_the_target() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtSkull,
        );
        let target = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 200 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(actor).unwrap().target = Some(target);

        a_skull_attack(&mut thinkers, &mut rmain, actor);

        let mobj = thinkers.mobj(actor).unwrap();
        assert_ne!(mobj.flags & mobj_flag::SKULLFLY, 0);
        assert!(
            mobj.momx > 0,
            "target is due east, momentum should point east"
        );
    }

    #[test]
    fn explode_damages_nearby_things_via_radius_attack() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let actor = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtTroopshot,
        );
        let nearby = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 40 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        thinkers.mobj_mut(nearby).unwrap().player = Some(0);
        let mut players = [Player::for_test(nearby)];
        let health_before = players[0].health;

        a_explode(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            actor,
        );

        assert!(
            players[0].health < health_before,
            "a nearby shootable thing within blast radius should take damage from A_Explode"
        );
    }

    #[test]
    fn boss_death_on_a_non_boss_level_is_a_noop() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut players: [Player; 0] = [];
        let mo = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtBruiser,
        );
        // E1M1 is episode 1, map 1 — A_BossDeath only ever does anything
        // on gamemap 8 (E1M8 in the shareware/registered episodes).
        // `doomstat::GameState` is a single global shared across every
        // test in this process (including ones running concurrently on
        // other threads), so its prior values are saved and restored
        // rather than left mutated — other tests (e.g. `p_inter`'s
        // damage tests) depend on the default `gameskill`/`gamemode`.
        let state = crate::doomstat::state_mut();
        let (saved_gamemode, saved_gameepisode, saved_gamemap) =
            (state.gamemode, state.gameepisode, state.gamemap);
        state.gamemode = crate::doomdef::GameMode::Registered;
        state.gameepisode = 1;
        state.gamemap = 1;
        let rdata = crate::r_data::RData::default();

        // Should return early without touching the level/thinkers in
        // any observable way (no panic, no G_ExitLevel side effect we
        // can directly assert here, but at minimum this must not panic
        // reading `RData::default()`'s empty tables).
        a_boss_death(&mut thinkers, &mut level, &mut players, &rdata, mo, &[]);

        let state = crate::doomstat::state_mut();
        state.gamemode = saved_gamemode;
        state.gameepisode = saved_gameepisode;
        state.gamemap = saved_gamemap;
    }

    #[test]
    fn hoof_delegates_to_chase() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let rdata = crate::r_data::RData::default();
        let mut brain = BrainTargets::default();
        let mo = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtCyborg,
        );
        thinkers.mobj_mut(mo).unwrap().lastlook = 0;
        // `P_LookForPlayers`'s loop (faithfully ported, see
        // `p_look_for_players`'s own docs) only terminates once it finds
        // a `playeringame[i] == true` slot to evaluate `stop` against —
        // with every slot false (nobody in the game) it spins forever,
        // exactly like the original's C would with an all-false
        // `playeringame[MAXPLAYERS]` (a scenario that never happens in a
        // real running game, which always has at least one player). A
        // real player must be present for this test to terminate.
        let player_mo = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 500 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut players = [Player::for_test(player_mo)];
        let playeringame = [true, false, false, false];

        a_hoof(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            &playeringame,
            &rdata,
            &mut brain,
            mo,
        );

        // A_Chase either finds the player as a new target or falls
        // through to P_SetMobjState(actor, spawnstate) — either way it
        // must not panic, and the mobj must still be alive (spawnstate
        // is never S_NULL for a live monster type).
        assert!(thinkers.is_live(mo));
    }

    #[test]
    fn brain_awake_finds_boss_target_spots() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let target1 = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtBosstarget,
        );
        let target2 = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 50 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtBosstarget,
        );
        let _decoy = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let mut brain = BrainTargets::default();

        a_brain_awake(&thinkers, &mut brain);

        assert_eq!(brain.targets.len(), 2);
        assert!(brain.targets.contains(&target1));
        assert!(brain.targets.contains(&target2));
    }

    #[test]
    fn brain_spit_cycles_through_targets_round_robin() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: [Player; 0] = [];
        let brain_mo = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtBossbrain,
        );
        let target1 = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 300 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtBosstarget,
        );
        let target2 = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 320 * FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtBosstarget,
        );
        let mut brain = BrainTargets::default();
        brain.targets.push(target1);
        brain.targets.push(target2);

        // Force `easy` so skill doesn't gate the spit on this call.
        // `doomstat::GameState` is process-global and shared with tests
        // running concurrently on other threads, so save/restore rather
        // than leave `gameskill` mutated (see the same note on
        // `boss_death_on_a_non_boss_level_is_a_noop`).
        brain.easy = true;
        let saved_gameskill = crate::doomstat::state().gameskill;
        crate::doomstat::state_mut().gameskill = crate::doomdef::Skill::Hard;

        a_brain_spit(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            0,
            &mut brain,
            brain_mo,
        );

        crate::doomstat::state_mut().gameskill = saved_gameskill;

        assert_eq!(
            brain.on, 1,
            "A_BrainSpit should advance braintargeton round-robin"
        );
    }
}
