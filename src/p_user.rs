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
//	Player related stuff.
//	Bobbing POV/weapon, movement.
//	Pending weapon.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_user.c` (partial, see note below). Player
//! related stuff. Bobbing POV/weapon, movement. Pending weapon (the
//! original's own description, preserved).
//!
//! # Scope
//!
//! Ported: [`p_thrust`] (`P_Thrust`), [`p_calc_height`]
//! (`P_CalcHeight`), [`p_move_player`] (`P_MovePlayer`),
//! [`p_death_think`] (`P_DeathThink`), [`p_player_think`]
//! (`P_PlayerThink`) — everything the "walk, look around, take stock of
//! powerups ticking down" milestone needs. Phase 7b4 closed the two
//! gaps this used to stop short of: [`p_player_think`] now calls
//! [`crate::p_spec::p_player_in_special_sector`] (`P_PlayerInSpecialSector`
//! — damaging floors) and the new (private) `p_use_lines`
//! (`P_UseLines`, `p_map.c`'s line-activation half — `PTR_UseTraverse`'s
//! read-only decision of which single line to use, then
//! [`crate::p_switch::p_use_special_line`] outside the traversal
//! closure, same two-pass shape as every other `&Level`-only-iterator/
//! `&mut Level`-needing-callback split in this port). Both needed
//! [`p_player_think`] to take the whole [`crate::p_spec::SpecialsCtx`]
//! instead of a bare `&mut Player` — see its own doc comment.
//!
//! Sounds: `p_user.c` itself calls none directly; `PTR_UseTraverse`'s
//! `sfx_noway` (really `p_map.c`, ported inside [`p_use_lines`]) is
//! queued via [`crate::s_sound`].
//!
//! # `onground`
//!
//! The original's `onground` is a file-level global, written by both
//! `P_MovePlayer` and `P_DeathThink` and read only by `P_CalcHeight`
//! (called right after, same tic, in both cases). Reproduced as an
//! explicit `bool` parameter to [`p_calc_height`] instead of a module
//! `static`, since every use is a same-tic write-then-read with no
//! cross-tic persistence — a parameter carries the same information
//! without the global.

use crate::d_player::{Player, PlayerState};
use crate::doomstat;
use crate::info::StateNum;
use crate::m_fixed::{fixed_mul, Fixed, FRACUNIT};
use crate::p_mobj::p_set_mobj_state;
use crate::p_pspr::p_move_psprites;
use crate::p_setup::Level;
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_main::RMain;
use crate::tables::{Angle, ANG180, ANG90, ANGLETOFINESHIFT, FINEANGLES, FINEMASK, FINESINE};

/// Index of the special effects (INVUL inverse) map (`INVERSECOLORMAP`).
pub const INVERSECOLORMAP: i32 = 32;

/// 16 pixels of bob (`MAXBOB`).
pub const MAXBOB: Fixed = 0x10_0000;

/// (`VIEWHEIGHT`, `p_local.h`)
pub const VIEWHEIGHT: Fixed = 41 * FRACUNIT;

/// (`USERANGE`, `p_local.h`)
const USERANGE: Fixed = 64 * FRACUNIT;

/// `ANG90/18` (`p_user.c`'s own `#define ANG5`).
const ANG5: Angle = ANG90 / 18;

/// Port of `P_Thrust`. Moves the given origin along a given angle (the
/// original's own comment, preserved).
pub fn p_thrust(thinkers: &mut Thinkers, mo: ThinkerId, angle: Angle, move_: Fixed) {
    let fine = (angle >> ANGLETOFINESHIFT) as usize;
    let mobj = thinkers.mobj_mut(mo).unwrap();
    mobj.momx = mobj
        .momx
        .wrapping_add(fixed_mul(move_, crate::tables::fine_cosine(fine)));
    mobj.momy = mobj.momy.wrapping_add(fixed_mul(move_, FINESINE[fine]));
}

/// Port of `P_CalcHeight`. Calculate the walking/running height
/// adjustment (the original's own comment, preserved). See module docs
/// on `onground`.
pub fn p_calc_height(thinkers: &Thinkers, player: &mut Player, onground: bool) {
    let mobj = thinkers.mobj(player.mo).unwrap();

    // Regular movement bobbing (needs to be calculated for gun swing
    // even if not on ground) OPTIMIZE: tablify angle. Note: a LUT
    // allows for effects like a ramp with low health (the original's
    // own comment, preserved).
    player.bob =
        (fixed_mul(mobj.momx, mobj.momx).wrapping_add(fixed_mul(mobj.momy, mobj.momy))) >> 2;
    if player.bob > MAXBOB {
        player.bob = MAXBOB;
    }

    if player.cheats & crate::d_player::Cheat::NoMomentum as i32 != 0 || !onground {
        player.viewz = mobj.z + player.viewheight;
        if player.viewz > mobj.ceilingz - 4 * FRACUNIT {
            player.viewz = mobj.ceilingz - 4 * FRACUNIT;
        }
        player.viewz = mobj.z + player.viewheight;
        return;
    }

    let leveltime = doomstat::state().leveltime;
    let angle = ((FINEANGLES / 20 * leveltime) & FINEMASK) as usize;
    let bob = fixed_mul(player.bob / 2, FINESINE[angle]);

    // move viewheight (the original's own comment, preserved).
    if player.playerstate == PlayerState::Live {
        player.viewheight += player.deltaviewheight;

        if player.viewheight > VIEWHEIGHT {
            player.viewheight = VIEWHEIGHT;
            player.deltaviewheight = 0;
        }
        if player.viewheight < VIEWHEIGHT / 2 {
            player.viewheight = VIEWHEIGHT / 2;
            if player.deltaviewheight <= 0 {
                player.deltaviewheight = 1;
            }
        }
        if player.deltaviewheight != 0 {
            player.deltaviewheight += FRACUNIT / 4;
            if player.deltaviewheight == 0 {
                player.deltaviewheight = 1;
            }
        }
    }

    player.viewz = mobj.z + player.viewheight + bob;
    if player.viewz > mobj.ceilingz - 4 * FRACUNIT {
        player.viewz = mobj.ceilingz - 4 * FRACUNIT;
    }
}

/// Port of `P_MovePlayer`.
pub fn p_move_player(thinkers: &mut Thinkers, player: &mut Player) -> bool {
    let cmd = player.cmd;

    {
        let mobj = thinkers.mobj_mut(player.mo).unwrap();
        mobj.angle = mobj.angle.wrapping_add((cmd.angleturn as i32 as u32) << 16);
    }

    // Do not let the player control movement if not onground (the
    // original's own comment, preserved).
    let (z, floorz, angle) = {
        let mobj = thinkers.mobj(player.mo).unwrap();
        (mobj.z, mobj.floorz, mobj.angle)
    };
    let onground = z <= floorz;

    if cmd.forwardmove != 0 && onground {
        p_thrust(thinkers, player.mo, angle, (cmd.forwardmove as i32) * 2048);
    }
    if cmd.sidemove != 0 && onground {
        p_thrust(
            thinkers,
            player.mo,
            angle.wrapping_sub(ANG90),
            (cmd.sidemove as i32) * 2048,
        );
    }

    // The original also does `if ((cmd->forwardmove || cmd->sidemove)
    // && player->mo->state == &states[S_PLAY]) P_SetMobjState(...,
    // S_PLAY_RUN1)` right here. `P_SetMobjState` needs `&mut Level`,
    // which this function (matching the original's own `void
    // P_MovePlayer(player_t*)` signature) isn't given — the caller
    // ([`p_player_think`]) does that check itself right after calling
    // this, with `&mut Level` in scope, instead of duplicating it here.
    onground
}

/// Port of `P_DeathThink`. Fall on your face when dying. Decrease POV
/// height to floor height (the original's own comment, preserved). See
/// module docs on `R_PointToAngle2`'s viewx/viewy side effect (same
/// save/restore pattern as `p_map.rs`'s `p_hit_slide_line`).
pub fn p_death_think(
    thinkers: &mut Thinkers,
    level: &mut Level,
    rmain: &mut RMain,
    player: &mut Player,
) {
    p_move_psprites(thinkers, level, player);

    // fall to the ground (the original's own comment, preserved).
    if player.viewheight > 6 * FRACUNIT {
        player.viewheight -= FRACUNIT;
    }
    if player.viewheight < 6 * FRACUNIT {
        player.viewheight = 6 * FRACUNIT;
    }

    player.deltaviewheight = 0;
    let (z, floorz) = {
        let mobj = thinkers.mobj(player.mo).unwrap();
        (mobj.z, mobj.floorz)
    };
    let onground = z <= floorz;
    p_calc_height(thinkers, player, onground);

    if let Some(attacker) = player.attacker {
        if attacker != player.mo {
            let (px, py, ax, ay, mo_angle) = {
                let mobj = thinkers.mobj(player.mo).unwrap();
                let att = thinkers.mobj(attacker).unwrap();
                (mobj.x, mobj.y, att.x, att.y, mobj.angle)
            };

            let (saved_viewx, saved_viewy) = (rmain.viewx, rmain.viewy);
            let angle = rmain.point_to_angle2(px, py, ax, ay);
            rmain.viewx = saved_viewx;
            rmain.viewy = saved_viewy;

            let delta = angle.wrapping_sub(mo_angle);

            if delta < ANG5 || delta > (ANG5 as i32).wrapping_neg() as u32 {
                // Looking at killer, so fade damage flash down (the
                // original's own comment, preserved).
                thinkers.mobj_mut(player.mo).unwrap().angle = angle;
                if player.damagecount != 0 {
                    player.damagecount -= 1;
                }
            } else if delta < ANG180 {
                let mobj = thinkers.mobj_mut(player.mo).unwrap();
                mobj.angle = mobj.angle.wrapping_add(ANG5);
            } else {
                let mobj = thinkers.mobj_mut(player.mo).unwrap();
                mobj.angle = mobj.angle.wrapping_sub(ANG5);
            }
        }
    } else if player.damagecount != 0 {
        player.damagecount -= 1;
    }

    if player.cmd.buttons & (crate::d_event::BT_USE as u8) != 0 {
        player.playerstate = PlayerState::Reborn;
    }
}

/// Port of `P_PlayerThink`. `player_idx` is this player's index into
/// `ctx.players` (`ctx.players[player_idx]` is the `player: &mut
/// Player` every other function in this file still takes directly —
/// only this function needs the whole [`crate::p_spec::SpecialsCtx`],
/// for [`crate::p_spec::p_player_in_special_sector`]/
/// [`crate::p_switch::p_use_special_line`]).
pub fn p_player_think(ctx: &mut crate::p_spec::SpecialsCtx, validcount: i32, player_idx: usize) {
    let thinkers = &mut *ctx.thinkers;
    let level = &mut *ctx.level;
    let rmain = &mut *ctx.rmain;
    let player = &mut ctx.players[player_idx];

    // fixme: do this in the cheat code (the original's own comment,
    // preserved).
    {
        let mobj = thinkers.mobj_mut(player.mo).unwrap();
        if player.cheats & crate::d_player::Cheat::NoClip as i32 != 0 {
            mobj.flags |= crate::r_defs::mobj_flag::NOCLIP;
        } else {
            mobj.flags &= !crate::r_defs::mobj_flag::NOCLIP;
        }
    }

    // chain saw run forward (the original's own comment, preserved).
    {
        let mobj = thinkers.mobj(player.mo).unwrap();
        if mobj.flags & crate::r_defs::mobj_flag::JUSTATTACKED != 0 {
            player.cmd.angleturn = 0;
            player.cmd.forwardmove = (0xc800i32 / 512) as i8;
            player.cmd.sidemove = 0;
            thinkers.mobj_mut(player.mo).unwrap().flags &= !crate::r_defs::mobj_flag::JUSTATTACKED;
        }
    }

    if player.playerstate == PlayerState::Dead {
        p_death_think(thinkers, level, rmain, player);
        return;
    }

    // Move around. Reactiontime is used to prevent movement for a bit
    // after a teleport (the original's own comment, preserved).
    let reactiontime = thinkers.mobj(player.mo).unwrap().reactiontime;
    let onground = if reactiontime != 0 {
        thinkers.mobj_mut(player.mo).unwrap().reactiontime -= 1;
        let mobj = thinkers.mobj(player.mo).unwrap();
        mobj.z <= mobj.floorz
    } else {
        let onground = p_move_player(thinkers, player);
        if (player.cmd.forwardmove != 0 || player.cmd.sidemove != 0)
            && thinkers.mobj(player.mo).unwrap().state == StateNum::SPlay
        {
            p_set_mobj_state(
                thinkers,
                level,
                player.mo,
                StateNum::SPlayRun1,
                |_, _, _, _| {},
            );
        }
        onground
    };

    p_calc_height(thinkers, player, onground);

    let sector = level.subsectors[thinkers.mobj(player.mo).unwrap().subsector.unwrap()].sector;
    if level.sectors[sector].special != 0 {
        crate::p_spec::p_player_in_special_sector(
            ctx.thinkers,
            ctx.level,
            ctx.players,
            ctx.rmain,
            doomstat::state().leveltime,
            player_idx,
        );
    }
    let player = &mut ctx.players[player_idx];

    // Check for weapon change. A special event has no other buttons
    // (the original's own comment, preserved).
    if player.cmd.buttons & (crate::d_event::BT_SPECIAL as u8) != 0 {
        player.cmd.buttons = 0;
    }

    if player.cmd.buttons & (crate::d_event::BT_CHANGE as u8) != 0 {
        // The actual changing of the weapon is done when the weapon
        // psprite can do it (read: not in the middle of an attack) (the
        // original's own comment, preserved).
        let newweapon_idx = (player.cmd.buttons & (crate::d_event::BT_WEAPONMASK as u8))
            >> crate::d_event::BT_WEAPONSHIFT;
        let mut newweapon = weapon_from_index(newweapon_idx as i32);

        if newweapon == crate::doomdef::WeaponType::WpFist
            && player.weaponowned[crate::doomdef::WeaponType::WpChainsaw as usize]
            && !(player.readyweapon == crate::doomdef::WeaponType::WpChainsaw
                && player.power(crate::doomdef::PowerType::PwStrength) != 0)
        {
            newweapon = crate::doomdef::WeaponType::WpChainsaw;
        }

        if doomstat::state().gamemode == crate::doomdef::GameMode::Commercial
            && newweapon == crate::doomdef::WeaponType::WpShotgun
            && player.weaponowned[crate::doomdef::WeaponType::WpSupershotgun as usize]
            && player.readyweapon != crate::doomdef::WeaponType::WpSupershotgun
        {
            newweapon = crate::doomdef::WeaponType::WpSupershotgun;
        }

        if player.weaponowned[newweapon as usize] && newweapon != player.readyweapon {
            // Do not go to plasma or BFG in shareware, even if cheated
            // (the original's own comment, preserved).
            if (newweapon != crate::doomdef::WeaponType::WpPlasma
                && newweapon != crate::doomdef::WeaponType::WpBfg)
                || doomstat::state().gamemode != crate::doomdef::GameMode::Shareware
            {
                player.pendingweapon = newweapon;
            }
        }
    }

    // check for use (the original's own comment, preserved).
    if player.cmd.buttons & (crate::d_event::BT_USE as u8) != 0 {
        if !player.usedown {
            p_use_lines(ctx, validcount, player_idx);
            ctx.players[player_idx].usedown = true;
        }
    } else {
        ctx.players[player_idx].usedown = false;
    }
    // cycle psprites (the original's own comment, preserved).
    crate::p_pspr::p_move_psprites_full(ctx, validcount, player_idx);

    let thinkers = &mut *ctx.thinkers;
    let player = &mut ctx.players[player_idx];

    // Counters, time dependent power ups. Strength counts up to
    // diminish fade (the original's own comment, preserved).
    tick_powers(thinkers, player);

    if player.damagecount != 0 {
        player.damagecount -= 1;
    }
    if player.bonuscount != 0 {
        player.bonuscount -= 1;
    }

    // Handling colormaps (the original's own comment, preserved).
    update_fixedcolormap(player);
}

fn weapon_from_index(i: i32) -> crate::doomdef::WeaponType {
    use crate::doomdef::WeaponType::*;
    match i {
        0 => WpFist,
        1 => WpPistol,
        2 => WpShotgun,
        3 => WpChaingun,
        4 => WpMissile,
        5 => WpPlasma,
        6 => WpBfg,
        7 => WpChainsaw,
        8 => WpSupershotgun,
        _ => WpNochange,
    }
}

fn tick_powers(thinkers: &mut Thinkers, player: &mut Player) {
    use crate::doomdef::PowerType;

    if player.power(PowerType::PwStrength) != 0 {
        player.powers[PowerType::PwStrength as usize] += 1;
    }
    if player.power(PowerType::PwInvulnerability) != 0 {
        player.powers[PowerType::PwInvulnerability as usize] -= 1;
    }
    if player.power(PowerType::PwInvisibility) != 0 {
        player.powers[PowerType::PwInvisibility as usize] -= 1;
        if player.power(PowerType::PwInvisibility) == 0 {
            thinkers.mobj_mut(player.mo).unwrap().flags &= !crate::r_defs::mobj_flag::SHADOW;
        }
    }
    if player.power(PowerType::PwInfrared) != 0 {
        player.powers[PowerType::PwInfrared as usize] -= 1;
    }
    if player.power(PowerType::PwIronfeet) != 0 {
        player.powers[PowerType::PwIronfeet as usize] -= 1;
    }
}

fn update_fixedcolormap(player: &mut Player) {
    use crate::doomdef::PowerType;

    if player.power(PowerType::PwInvulnerability) != 0 {
        let p = player.power(PowerType::PwInvulnerability);
        if p > 4 * 32 || (p & 8) != 0 {
            player.fixedcolormap = INVERSECOLORMAP;
        } else {
            player.fixedcolormap = 0;
        }
    } else if player.power(PowerType::PwInfrared) != 0 {
        let p = player.power(PowerType::PwInfrared);
        if p > 4 * 32 || (p & 8) != 0 {
            // almost full bright (the original's own comment, preserved).
            player.fixedcolormap = 1;
        } else {
            player.fixedcolormap = 0;
        }
    } else {
        player.fixedcolormap = 0;
    }
}

/// Port of `P_UseLines`. Looks for special lines in front of the player
/// to activate (the original's own comment, preserved).
fn p_use_lines(ctx: &mut crate::p_spec::SpecialsCtx, validcount: i32, player_idx: usize) {
    let player = &ctx.players[player_idx];
    let usething = player.mo;
    let (px, py, pangle) = {
        let mo = ctx.thinkers.mobj(usething).unwrap();
        (mo.x, mo.y, mo.angle)
    };

    let angle = (pangle >> ANGLETOFINESHIFT) as usize;
    let x1 = px;
    let y1 = py;
    let x2 =
        x1.wrapping_add((USERANGE >> crate::m_fixed::FRACBITS) * crate::tables::fine_cosine(angle));
    let y2 = y1.wrapping_add((USERANGE >> crate::m_fixed::FRACBITS) * FINESINE[angle]);

    // Port of `PTR_UseTraverse`. `p_path_traverse`'s callback only
    // lends `&Level`/`&Thinkers` (see its docs) — `P_UseSpecialLine`
    // needs `&mut` everything, so the traverser here only decides which
    // single line (if any) should be used, matching the original's
    // "can't use more than one special line in a row" early-stop; the
    // actual use happens after, outside the closure.
    let mut line_to_use: Option<(usize, i32)> = None;
    let mut blocked_by_wall = false;
    crate::p_maputl::p_path_traverse(
        ctx.level,
        ctx.thinkers,
        validcount,
        x1,
        y1,
        x2,
        y2,
        crate::p_maputl::PT_ADDLINES,
        |level, _thinkers, intercept| {
            let crate::p_maputl::InterceptTarget::Line(line_idx) = intercept.target else {
                return true;
            };
            let line = &level.lines[line_idx];

            if line.special == 0 {
                let opening = crate::p_maputl::p_line_opening(level, line);
                if opening.openrange <= 0 {
                    crate::s_sound::s_start_sound(
                        Some(crate::s_sound::SoundOrigin::Mobj(usething)),
                        crate::sounds::Sfx::SfxNoway,
                    );
                    blocked_by_wall = true;
                    return false; // can't use through a wall
                }
                return true; // not a special line, but keep checking
            }

            let side = crate::p_maputl::p_point_on_line_side(px, py, level, line);
            line_to_use = Some((line_idx, side));
            false // can't use for more than one special line in a row
        },
    );
    let _ = blocked_by_wall;

    if let Some((line_idx, side)) = line_to_use {
        crate::p_switch::p_use_special_line(ctx, usething, line_idx, side);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d_ticcmd::TicCmd;
    use crate::info::MobjType;
    use crate::p_mobj::{p_spawn_mobj, SpawnZ};
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

    fn blank_player(thinkers: &mut Thinkers, level: &mut Level) -> Player {
        let mo = p_spawn_mobj(
            thinkers,
            level,
            1056 * FRACUNIT,
            -3616 * FRACUNIT,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        Player::for_test(mo)
    }

    #[test]
    fn thrust_adds_momentum_along_the_angle() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mo = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            1056 * FRACUNIT,
            -3616 * FRACUNIT,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        p_thrust(&mut thinkers, mo, 0, 100 * FRACUNIT);
        let mobj = thinkers.mobj(mo).unwrap();
        assert!(mobj.momx > 0, "thrust at angle 0 should push +x");
        // FINESINE[0] is 25, not exactly 0 (the original's table is a
        // real sampled LUT, not a pure sine) — a small +y nudge at
        // angle 0 is the original's actual, faithfully reproduced
        // behavior, not a bug.
        assert!(
            mobj.momy.abs() < mobj.momx / 100,
            "thrust at angle 0 should push y only negligibly (FINESINE[0] artifact)"
        );
    }

    #[test]
    fn move_player_onground_applies_forward_thrust() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut player = blank_player(&mut thinkers, &mut level);
        player.cmd = TicCmd {
            forwardmove: 10,
            ..Default::default()
        };

        let onground = p_move_player(&mut thinkers, &mut player);
        assert!(onground, "freshly spawned mobj starts on its floor");
        assert_ne!(thinkers.mobj(player.mo).unwrap().momx, 0);
    }

    #[test]
    fn calc_height_off_ground_uses_full_viewheight_capped_by_ceiling() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut player = blank_player(&mut thinkers, &mut level);
        player.viewheight = VIEWHEIGHT;

        p_calc_height(&thinkers, &mut player, false);

        let mobj = thinkers.mobj(player.mo).unwrap();
        assert_eq!(player.viewz, mobj.z + VIEWHEIGHT);
    }
}
