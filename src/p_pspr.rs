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
//	Weapon sprite animation, weapon objects.
//	Action functions for weapons.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_pspr.h` / `p_pspr.c`.
//! Weapon sprite animation, weapon objects. Action functions for
//! weapons (the original's own description, preserved).
//!
//! # Scope
//!
//! Ported: [`p_set_psprite`] (`P_SetPsprite`), [`p_setup_psprites`]
//! (`P_SetupPsprites`), [`p_move_psprites`] (`P_MovePsprites`),
//! [`p_bring_up_weapon`] (`P_BringUpWeapon`), [`p_check_ammo`]
//! (`P_CheckAmmo`), [`p_drop_weapon`] (`P_DropWeapon`), and the weapon
//! state machine's own action functions: [`a_weapon_ready`]
//! (`A_WeaponReady`), [`a_refire`] (`A_ReFire`), [`a_check_reload`]
//! (`A_CheckReload`), [`a_lower`] (`A_Lower`), [`a_raise`] (`A_Raise`),
//! [`a_gun_flash`] (`A_GunFlash`), [`a_light0`]/[`a_light1`]/[`a_light2`]
//! (`A_Light0`/`A_Light1`/`A_Light2`) — none of these need combat.
//!
//! Also ported (Phase 7e): the weapon attack actions `A_Punch`,
//! `A_Saw`, `A_FireMissile`, `A_FireBFG`, `A_FirePlasma`,
//! `A_FirePistol`, `A_FireShotgun`, `A_FireShotgun2`, `A_FireCGun`, with
//! `P_BulletSlope`/`P_GunShot`, `P_FireWeapon`'s `P_NoiseAlert`, and
//! (in `p_enemy.rs`, where mobj actions dispatch) `A_BFGSpray`. The
//! three super-shotgun reload cues and `A_BFGsound` are sound-only.
//!
//! # Deferred attacks
//!
//! An attack needs the whole game (the shot can hurt *any* player,
//! trigger any special line, spawn missiles), while `P_SetPsprite` and
//! every weapon function only have `&mut Player` — and the attack's
//! `players: &mut [Player]` would alias it. So entering an attack state
//! only *queues* a [`PendingPspr`] on the player, and
//! [`p_move_psprites_full`] (the one caller with a `SpecialsCtx`) runs
//! the queue right after each psprite's `P_SetPsprite`. That is the
//! same point the original runs them: every attack state has a
//! non-zero tic count, so `P_SetPsprite`'s state chain ends right there
//! anyway and nothing between the action and that point can observe the
//! difference. The one behavioural gap is `P_NoiseAlert`, whose
//! bumped `validcount` isn't handed back (see [`run_pending`]).
//! [`p_move_psprites`] (no context) is for the dead player, whose
//! weapon is being lowered and cannot attack.
//!
//! # `P_SetPsprite`'s action dispatch
//!
//! Like [`crate::p_mobj::p_set_mobj_state`] (Phase 6b), the original's
//! `state->action.acp2(player, psp)` call is a `match` over
//! [`crate::info::StateAction`] here. The weapon state machine's own
//! actions (listed above) are dispatched for real; the attack actions
//! are queued and run by [`p_move_psprites_full`] (see "Deferred
//! attacks").

use crate::d_event::BT_ATTACK;
use crate::d_items::WEAPONINFO;
use crate::d_player::Player;
use crate::doomdef::GameMode;
use crate::doomdef::WeaponType;
use crate::doomstat;
use crate::info::{SpriteNum, StateAction, StateNum, STATES};
use crate::m_fixed::{fixed_mul, Fixed, FRACBITS, FRACUNIT};
use crate::m_random::p_random;
use crate::p_map::MELEERANGE;
use crate::p_mobj::p_set_mobj_state;
use crate::p_setup::Level;
use crate::p_tick::{ThinkerId, Thinkers};
use crate::s_sound::{s_start_sound, SoundOrigin};
use crate::sounds::Sfx;
use crate::tables::{fine_cosine, FINEANGLES, FINEMASK, FINESINE};

/// Weapon bob/lower/raise speed and the psprite's resting Y (`p_pspr.c`
/// `#define`s).
pub const LOWERSPEED: Fixed = FRACUNIT * 6;
pub const RAISESPEED: Fixed = FRACUNIT * 6;
pub const WEAPONBOTTOM: Fixed = 128 * FRACUNIT;
pub const WEAPONTOP: Fixed = 32 * FRACUNIT;

/// Plasma cells for a bfg attack (`BFGCELLS`).
pub const BFGCELLS: i32 = 40;

/// Overlay psprites are scaled shapes drawn directly on the view
/// screen, coordinates are given for a 320*200 view screen (the
/// original's own comment, preserved). (`psprnum_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsprNum {
    Weapon,
    Flash,
}

/// (`NUMPSPRITES`)
pub const NUMPSPRITES: usize = 2;

/// (`pspdef_t`). `state: NULL` (not active) is `state: None` here —
/// deliberately a field, not the whole struct wrapped in `Option`
/// (unlike most "absent" values elsewhere in this port): the original's
/// `pspdef_t` is a plain struct whose `sx`/`sy` stay addressable and
/// meaningful even while `state == NULL` (`P_BringUpWeapon` writes
/// `player->psprites[ps_weapon].sy = WEAPONBOTTOM` *before* calling
/// `P_SetPsprite`, i.e. while the slot may still be inactive from a
/// previous `P_SetPsprite(..., S_NULL)` or the initial
/// `P_SetupPsprites` reset) — wrapping the whole struct in `Option`
/// would silently drop that write instead of carrying it forward like
/// the original does. Every other `Option` in this port stands for "no
/// meaningful state to read", which isn't true of `sx`/`sy` here.
#[derive(Debug, Clone, Copy, Default)]
pub struct PSpr {
    /// A null state means not active (the original's own comment,
    /// preserved, on `pspdef_t.state`).
    pub state: Option<StateNum>,
    pub tics: i32,
    pub sx: Fixed,
    pub sy: Fixed,
}

/// Work `P_SetPsprite`'s action dispatch can't do itself and defers to
/// [`p_move_psprites_full`], which owns the whole game context. See the
/// module docs ("Deferred attacks").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPspr {
    /// A weapon attack action (`A_Punch`, `A_FirePistol`, ...) reached
    /// while entering `state` on `position`.
    Attack {
        position: PsprNum,
        state: StateNum,
        action: StateAction,
    },
    /// `P_NoiseAlert(player->mo, player->mo)` at the end of
    /// `P_FireWeapon`.
    NoiseAlert,
}

/// Port of `P_SetPsprite`.
pub fn p_set_psprite(
    thinkers: &mut Thinkers,
    level: &mut Level,
    player: &mut Player,
    position: PsprNum,
    stnum: StateNum,
) {
    let mut stnum = stnum;
    loop {
        if stnum == StateNum::SNull {
            // object removed itself (the original's own comment,
            // preserved). sx/sy are left as they are — the original
            // doesn't reset them either, only `state`.
            player.psprites[position as usize].state = None;
            break;
        }

        let state = STATES[stnum as usize];
        {
            let psp = &mut player.psprites[position as usize];
            psp.state = Some(stnum);
            psp.tics = state.tics; // could be 0 (the original's own comment, preserved)

            if state.misc1 != 0 {
                // coordinate set (the original's own comment, preserved).
                psp.sx = state.misc1 << FRACBITS;
                psp.sy = state.misc2 << FRACBITS;
            }
        }

        // Call action routine. Modified handling (the original's own
        // comment, preserved). See module docs on the dispatch.
        dispatch_psprite_action(thinkers, level, player, position, state.action);

        // the action may have called p_set_psprite itself (e.g.
        // A_Lower -> P_BringUpWeapon -> P_SetPsprite), which the
        // original allows too (`if (!psp->state) break;` re-reads
        // through the pointer) — re-read after dispatch.
        let psp = player.psprites[position as usize];
        let Some(cur_state) = psp.state else {
            break; // object removed itself during the action
        };

        stnum = STATES[cur_state as usize].nextstate;
        if psp.tics != 0 {
            break;
        }
        // an initial state of 0 could cycle through (the original's own
        // comment, preserved).
    }
}

/// Dispatches a state's action for a psprite, standing in for the
/// original's `state->action.acp2(player, psp)`. See module docs.
fn dispatch_psprite_action(
    thinkers: &mut Thinkers,
    level: &mut Level,
    player: &mut Player,
    position: PsprNum,
    action: StateAction,
) {
    match action {
        StateAction::None => {}
        StateAction::AWeaponReady => a_weapon_ready(thinkers, level, player, position),
        StateAction::AReFire => a_refire(thinkers, level, player),
        StateAction::ACheckReload => a_check_reload(thinkers, level, player),
        StateAction::ALower => a_lower(thinkers, level, player),
        StateAction::ARaise => a_raise(thinkers, level, player),
        StateAction::AGunFlash => a_gun_flash(thinkers, level, player),
        // A_OpenShotgun2/A_LoadShotgun2/A_CloseShotgun2 live in
        // `p_enemy.c` in the original but are weapon-side actions
        // (`player_t*`, `pspdef_t*`); A_BFGsound is `p_pspr.c`'s. All
        // four are pure sound cues, so they need no combat context.
        StateAction::AOpenShotgun2 => {
            s_start_sound(Some(SoundOrigin::Mobj(player.mo)), Sfx::SfxDbopn)
        }
        StateAction::ALoadShotgun2 => {
            s_start_sound(Some(SoundOrigin::Mobj(player.mo)), Sfx::SfxDbload)
        }
        StateAction::ACloseShotgun2 => {
            s_start_sound(Some(SoundOrigin::Mobj(player.mo)), Sfx::SfxDbcls);
            a_refire(thinkers, level, player);
        }
        StateAction::ABFGsound => s_start_sound(Some(SoundOrigin::Mobj(player.mo)), Sfx::SfxBfg),
        StateAction::ALight0 => a_light0(player),
        StateAction::ALight1 => a_light1(player),
        StateAction::ALight2 => a_light2(player),
        // The attack actions need the whole game context (the shot can
        // hurt any player, trigger any special line), not just this
        // player — deferred, see module docs.
        StateAction::APunch
        | StateAction::ASaw
        | StateAction::AFireMissile
        | StateAction::AFireBFG
        | StateAction::AFirePlasma
        | StateAction::AFirePistol
        | StateAction::AFireShotgun
        | StateAction::AFireShotgun2
        | StateAction::AFireCGun => {
            if let Some(state) = player.psprites[position as usize].state {
                player.pending_pspr.push(PendingPspr::Attack {
                    position,
                    state,
                    action,
                });
            }
        }
        // A_BFGSpray is a *mobj* action (the BFG ball's death), handled
        // by `p_enemy::dispatch_mobj_action`.
        _ => {}
    }
}

// P_CalcSwing's scratch globals (swingx/swingy) are unused by anything
// Phase 6d ports (the HUD/weapon-sprite renderer that reads
// P_CalcSwing's output is Phase 9) — not ported yet; noted here so it
// isn't forgotten, not left silently missing.

/// Port of `P_BringUpWeapon`. Starts bringing the pending weapon up from
/// the bottom of the screen. Uses player (the original's own comment,
/// preserved).
pub fn p_bring_up_weapon(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    if player.pendingweapon == WeaponType::WpNochange {
        player.pendingweapon = player.readyweapon;
    }

    if player.pendingweapon == WeaponType::WpChainsaw {
        s_start_sound(Some(SoundOrigin::Mobj(player.mo)), Sfx::SfxSawup);
    }

    let newstate = WEAPONINFO[player.pendingweapon as usize].upstate;

    player.pendingweapon = WeaponType::WpNochange;
    set_weapon_sy(player, WEAPONBOTTOM);

    p_set_psprite(thinkers, level, player, PsprNum::Weapon, newstate);
}

/// Port of `P_CheckAmmo`. Returns true if there is enough ammo to
/// shoot. If not, selects the next weapon to use (the original's own
/// comment, preserved).
pub fn p_check_ammo(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) -> bool {
    let info = WEAPONINFO[player.readyweapon as usize];
    let ammo = info.ammo;

    // Minimal amount for one shot varies (the original's own comment,
    // preserved).
    let count = if player.readyweapon == WeaponType::WpBfg {
        BFGCELLS
    } else if player.readyweapon == WeaponType::WpSupershotgun {
        2 // Double barrel.
    } else {
        1 // Regular.
    };

    // Some do not need ammunition anyway. Return if current ammunition
    // sufficient (the original's own comment, preserved).
    if ammo == crate::doomdef::AmmoType::AmNoammo || player.ammo[ammo as usize] >= count {
        return true;
    }

    // Out of ammo, pick a weapon to change to. Preferences are set here
    // (the original's own comment, preserved).
    let gamemode = doomstat::state().gamemode;
    let pending = if player.weaponowned[WeaponType::WpPlasma as usize]
        && player.ammo[crate::doomdef::AmmoType::AmCell as usize] != 0
        && gamemode != GameMode::Shareware
    {
        WeaponType::WpPlasma
    } else if player.weaponowned[WeaponType::WpSupershotgun as usize]
        && player.ammo[crate::doomdef::AmmoType::AmShell as usize] > 2
        && gamemode == GameMode::Commercial
    {
        WeaponType::WpSupershotgun
    } else if player.weaponowned[WeaponType::WpChaingun as usize]
        && player.ammo[crate::doomdef::AmmoType::AmClip as usize] != 0
    {
        WeaponType::WpChaingun
    } else if player.weaponowned[WeaponType::WpShotgun as usize]
        && player.ammo[crate::doomdef::AmmoType::AmShell as usize] != 0
    {
        WeaponType::WpShotgun
    } else if player.ammo[crate::doomdef::AmmoType::AmClip as usize] != 0 {
        WeaponType::WpPistol
    } else if player.weaponowned[WeaponType::WpChainsaw as usize] {
        WeaponType::WpChainsaw
    } else if player.weaponowned[WeaponType::WpMissile as usize]
        && player.ammo[crate::doomdef::AmmoType::AmMisl as usize] != 0
    {
        WeaponType::WpMissile
    } else if player.weaponowned[WeaponType::WpBfg as usize]
        && player.ammo[crate::doomdef::AmmoType::AmCell as usize] > 40
        && gamemode != GameMode::Shareware
    {
        WeaponType::WpBfg
    } else {
        // If everything fails (the original's own comment, preserved).
        WeaponType::WpFist
    };
    player.pendingweapon = pending;

    // Now set appropriate weapon overlay (the original's own comment,
    // preserved).
    let downstate = WEAPONINFO[player.readyweapon as usize].downstate;
    p_set_psprite(thinkers, level, player, PsprNum::Weapon, downstate);

    false
}

/// Port of `P_FireWeapon`.
pub fn p_fire_weapon(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    if !p_check_ammo(thinkers, level, player) {
        return;
    }

    p_set_mobj_state(
        thinkers,
        level,
        player.mo,
        StateNum::SPlayAtk1,
        |_, _, _, _| {},
    );
    let newstate = WEAPONINFO[player.readyweapon as usize].atkstate;
    p_set_psprite(thinkers, level, player, PsprNum::Weapon, newstate);
    // P_NoiseAlert(player->mo, player->mo): deferred, see module docs.
    player.pending_pspr.push(PendingPspr::NoiseAlert);
}

/// Port of `P_DropWeapon`. Player died, so put the weapon away (the
/// original's own comment, preserved).
pub fn p_drop_weapon(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    let downstate = WEAPONINFO[player.readyweapon as usize].downstate;
    p_set_psprite(thinkers, level, player, PsprNum::Weapon, downstate);
}

/// Port of `A_WeaponReady`. The player can fire the weapon or change to
/// another weapon at this time. Follows after getting weapon up, or
/// after previous attack/fire sequence (the original's own comment,
/// preserved).
fn a_weapon_ready(
    thinkers: &mut Thinkers,
    level: &mut Level,
    player: &mut Player,
    position: PsprNum,
) {
    // get out of attack state (the original's own comment, preserved).
    let mobj_state = thinkers.mobj(player.mo).unwrap().state;
    if mobj_state == StateNum::SPlayAtk1 || mobj_state == StateNum::SPlayAtk2 {
        p_set_mobj_state(thinkers, level, player.mo, StateNum::SPlay, |_, _, _, _| {});
    }

    if player.readyweapon == WeaponType::WpChainsaw
        && player.psprites[position as usize].state == Some(StateNum::SSaw)
    {
        s_start_sound(Some(SoundOrigin::Mobj(player.mo)), Sfx::SfxSawidl);
    }

    // check for change if player is dead, put the weapon away (the
    // original's own comment, preserved).
    if player.pendingweapon != WeaponType::WpNochange || player.health == 0 {
        // change weapon (pending weapon should already be validated)
        // (the original's own comment, preserved).
        let newstate = WEAPONINFO[player.readyweapon as usize].downstate;
        p_set_psprite(thinkers, level, player, PsprNum::Weapon, newstate);
        return;
    }

    // check for fire the missile launcher and bfg do not auto fire (the
    // original's own comment, preserved).
    if player.cmd.buttons & (BT_ATTACK as u8) != 0 {
        if !player.attackdown
            || (player.readyweapon != WeaponType::WpMissile
                && player.readyweapon != WeaponType::WpBfg)
        {
            player.attackdown = true;
            p_fire_weapon(thinkers, level, player);
            return;
        }
    } else {
        player.attackdown = false;
    }

    // bob the weapon based on movement speed (the original's own
    // comment, preserved).
    let leveltime = doomstat::state().leveltime;
    let mut angle = ((128 * leveltime) & FINEMASK) as usize;
    let sx = FRACUNIT + fixed_mul(player.bob, fine_cosine(angle));
    angle &= (FINEANGLES / 2 - 1) as usize;
    let sy = WEAPONTOP + fixed_mul(player.bob, FINESINE[angle]);
    set_weapon_sxy(player, position, sx, sy);
}

/// Port of `A_ReFire`. The player can re-fire the weapon without
/// lowering it entirely (the original's own comment, preserved).
fn a_refire(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    // check for fire (if a weaponchange is pending, let it go through
    // instead) (the original's own comment, preserved).
    if player.cmd.buttons & (BT_ATTACK as u8) != 0
        && player.pendingweapon == WeaponType::WpNochange
        && player.health != 0
    {
        player.refire += 1;
        p_fire_weapon(thinkers, level, player);
    } else {
        player.refire = 0;
        p_check_ammo(thinkers, level, player);
    }
}

/// Port of `A_CheckReload`. The original's `#if 0`'d super-shotgun
/// reload-prompt branch isn't ported either (dead code in the original
/// too).
fn a_check_reload(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    p_check_ammo(thinkers, level, player);
}

/// Port of `A_Lower`. Lowers current weapon, and changes weapon at
/// bottom (the original's own comment, preserved).
fn a_lower(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    let sy = weapon_sy(player) + LOWERSPEED;
    set_weapon_sy(player, sy);

    // Is already down (the original's own comment, preserved).
    if weapon_sy(player) < WEAPONBOTTOM {
        return;
    }

    // Player is dead (the original's own comment, preserved).
    if player.playerstate == crate::d_player::PlayerState::Dead {
        set_weapon_sy(player, WEAPONBOTTOM);
        return; // don't bring weapon back up
    }

    // The old weapon has been lowered off the screen, so change the
    // weapon and start raising it (the original's own comment,
    // preserved).
    if player.health == 0 {
        // Player is dead, so keep the weapon off screen (the original's
        // own comment, preserved).
        p_set_psprite(thinkers, level, player, PsprNum::Weapon, StateNum::SNull);
        return;
    }

    player.readyweapon = player.pendingweapon;
    p_bring_up_weapon(thinkers, level, player);
}

/// Port of `A_Raise`.
fn a_raise(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    let sy = weapon_sy(player) - RAISESPEED;
    set_weapon_sy(player, sy);

    if weapon_sy(player) > WEAPONTOP {
        return;
    }
    set_weapon_sy(player, WEAPONTOP);

    // The weapon has been raised all the way, so change to the ready
    // state (the original's own comment, preserved).
    let newstate = WEAPONINFO[player.readyweapon as usize].readystate;
    p_set_psprite(thinkers, level, player, PsprNum::Weapon, newstate);
}

/// Port of `A_GunFlash`.
fn a_gun_flash(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    p_set_mobj_state(
        thinkers,
        level,
        player.mo,
        StateNum::SPlayAtk2,
        |_, _, _, _| {},
    );
    let flashstate = WEAPONINFO[player.readyweapon as usize].flashstate;
    p_set_psprite(thinkers, level, player, PsprNum::Flash, flashstate);
}

/// Port of `A_Light0`/`A_Light1`/`A_Light2`.
fn a_light0(player: &mut Player) {
    player.extralight = 0;
}
fn a_light1(player: &mut Player) {
    player.extralight = 1;
}
fn a_light2(player: &mut Player) {
    player.extralight = 2;
}

fn weapon_sy(player: &Player) -> Fixed {
    player.psprites[PsprNum::Weapon as usize].sy
}

fn set_weapon_sy(player: &mut Player, sy: Fixed) {
    player.psprites[PsprNum::Weapon as usize].sy = sy;
}

fn set_weapon_sxy(player: &mut Player, position: PsprNum, sx: Fixed, sy: Fixed) {
    let psp = &mut player.psprites[position as usize];
    psp.sx = sx;
    psp.sy = sy;
}

/// Port of `P_SetupPsprites`. Called at start of level for each player
/// (the original's own comment, preserved).
pub fn p_setup_psprites(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    // remove all psprites (the original's own comment, preserved).
    player.psprites = [PSpr::default(); NUMPSPRITES];

    // spawn the gun (the original's own comment, preserved).
    player.pendingweapon = player.readyweapon;
    p_bring_up_weapon(thinkers, level, player);
}

/// One iteration of `P_MovePsprites`' loop: drop the tic count and
/// possibly change state.
fn advance_psprite(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player, i: usize) {
    let psp = player.psprites[i];

    // a null state means not active (the original's own comment,
    // preserved).
    let Some(cur_state) = psp.state else {
        return;
    };

    // drop tic count and possibly change state a -1 tic count never
    // changes (the original's own comment, preserved).
    if psp.tics != -1 {
        let new_tics = psp.tics - 1;
        player.psprites[i].tics = new_tics;
        if new_tics == 0 {
            let nextstate = STATES[cur_state as usize].nextstate;
            let position = if i == PsprNum::Weapon as usize {
                PsprNum::Weapon
            } else {
                PsprNum::Flash
            };
            p_set_psprite(thinkers, level, player, position, nextstate);
        }
    }
}

/// The tail of `P_MovePsprites`: the flash follows the weapon.
fn sync_flash_to_weapon(player: &mut Player) {
    let weapon_sx = player.psprites[PsprNum::Weapon as usize].sx;
    let weapon_sy = player.psprites[PsprNum::Weapon as usize].sy;
    let flash = &mut player.psprites[PsprNum::Flash as usize];
    flash.sx = weapon_sx;
    flash.sy = weapon_sy;
}

/// Port of `P_MovePsprites` for callers that have no game context — the
/// dead player's `P_DeathThink` (the weapon is being lowered, no attack
/// can start). Any deferred [`PendingPspr`] is left queued for the next
/// [`p_move_psprites_full`].
pub fn p_move_psprites(thinkers: &mut Thinkers, level: &mut Level, player: &mut Player) {
    for i in 0..NUMPSPRITES {
        advance_psprite(thinkers, level, player, i);
    }
    sync_flash_to_weapon(player);
}

/// Port of `P_MovePsprites`. Called every tic by player thinking
/// routine (the original's own comment, preserved). Runs the deferred
/// attack actions/noise alerts each state change queued, right after
/// the `P_SetPsprite` that queued them — the same point the original
/// runs them (see module docs).
pub fn p_move_psprites_full(
    ctx: &mut crate::p_spec::SpecialsCtx,
    validcount: i32,
    player_idx: usize,
) {
    for i in 0..NUMPSPRITES {
        advance_psprite(ctx.thinkers, ctx.level, &mut ctx.players[player_idx], i);
        run_pending(ctx, validcount, player_idx);
    }
    sync_flash_to_weapon(&mut ctx.players[player_idx]);
}

/// Runs the queued [`PendingPspr`]s in order (running one can queue
/// more, so this drains until empty).
fn run_pending(ctx: &mut crate::p_spec::SpecialsCtx, validcount: i32, player_idx: usize) {
    while !ctx.players[player_idx].pending_pspr.is_empty() {
        let pending = ctx.players[player_idx].pending_pspr.remove(0);
        match pending {
            PendingPspr::NoiseAlert => {
                let mo = ctx.players[player_idx].mo;
                // The bumped validcount isn't handed back (nothing above
                // owns one); sectors marked with it are simply treated as
                // fresh by later traversals.
                let _ =
                    crate::p_enemy::p_noise_alert(&*ctx.thinkers, ctx.level, validcount, mo, mo);
            }
            PendingPspr::Attack {
                position,
                state,
                action,
            } => run_attack(ctx, validcount, player_idx, position, state, action),
        }
    }
}

/// (`MISSILERANGE`, `p_local.h`).
const MISSILERANGE: Fixed = 32 * 64 * FRACUNIT;
/// `16*64*FRACUNIT`, the range `P_BulletSlope`/`A_BFGSpray` aim at.
const AIMRANGE: Fixed = 16 * 64 * FRACUNIT;
/// `ANG90/20` and `ANG90/21` (`A_Saw`'s turn-toward-target steps).
const ANG90_20: u32 = crate::tables::ANG90 / 20;
const ANG90_21: u32 = crate::tables::ANG90 / 21;

/// `flashstate + n` for the two multi-frame flashes (plasma and
/// chaingun, `n` is 0 or 1).
fn flash_plus(base: StateNum, n: usize) -> StateNum {
    match (base, n) {
        (b, 0) => b,
        (StateNum::SPlasmaflash1, 1) => StateNum::SPlasmaflash2,
        (StateNum::SChainflash1, 1) => StateNum::SChainflash2,
        _ => panic!("flash_plus: no state {n} after {base:?}"),
    }
}

/// Port of `P_BulletSlope`.
fn bullet_slope(ctx: &mut crate::p_spec::SpecialsCtx, validcount: i32, mo: ThinkerId) -> Fixed {
    // see which target is to be aimed at (the original's own comment,
    // preserved).
    let mut an = ctx.thinkers.mobj(mo).unwrap().angle;
    let (mut slope, mut target) =
        crate::p_map::p_aim_line_attack(ctx.thinkers, ctx.level, validcount, mo, an, AIMRANGE);
    if target.is_none() {
        an = an.wrapping_add(1 << 26);
        (slope, target) =
            crate::p_map::p_aim_line_attack(ctx.thinkers, ctx.level, validcount, mo, an, AIMRANGE);
        if target.is_none() {
            an = an.wrapping_sub(2 << 26);
            (slope, _) = crate::p_map::p_aim_line_attack(
                ctx.thinkers,
                ctx.level,
                validcount,
                mo,
                an,
                AIMRANGE,
            );
        }
    }
    slope
}

/// `P_LineAttack` followed by the `P_ShootSpecialLine`s it collected
/// (the original runs those inside the trace; see
/// [`crate::p_map::p_line_attack`]'s docs).
fn line_attack(
    ctx: &mut crate::p_spec::SpecialsCtx,
    validcount: i32,
    t1: ThinkerId,
    angle: crate::tables::Angle,
    distance: Fixed,
    slope: Fixed,
    damage: i32,
) {
    let lines = crate::p_map::p_line_attack(
        ctx.thinkers,
        ctx.level,
        ctx.players,
        ctx.rmain,
        validcount,
        t1,
        angle,
        distance,
        slope,
        damage,
    );
    for line_idx in lines {
        crate::p_spec::p_shoot_special_line(ctx, t1, line_idx);
    }
}

/// Port of `P_GunShot`.
fn gun_shot(
    ctx: &mut crate::p_spec::SpecialsCtx,
    validcount: i32,
    mo: ThinkerId,
    accurate: bool,
    bulletslope: Fixed,
) {
    let damage = 5 * (p_random() % 3 + 1);
    let mut angle = ctx.thinkers.mobj(mo).unwrap().angle;
    if !accurate {
        angle = angle.wrapping_add(((p_random() - p_random()) << 18) as u32);
    }
    line_attack(
        ctx,
        validcount,
        mo,
        angle,
        MISSILERANGE,
        bulletslope,
        damage,
    );
}

/// `P_SetMobjState(player->mo, S_PLAY_ATK2)`.
fn player_attack_pose(ctx: &mut crate::p_spec::SpecialsCtx, mo: ThinkerId) {
    p_set_mobj_state(
        ctx.thinkers,
        ctx.level,
        mo,
        StateNum::SPlayAtk2,
        |_, _, _, _| {},
    );
}

/// `player->ammo[weaponinfo[player->readyweapon].ammo] -= n`.
fn spend_ammo(player: &mut Player, n: i32) {
    let ammo = WEAPONINFO[player.readyweapon as usize].ammo;
    player.ammo[ammo as usize] -= n;
}

/// `P_SetPsprite(player, ps_flash, weaponinfo[readyweapon].flashstate + n)`.
fn set_flash(ctx: &mut crate::p_spec::SpecialsCtx, player_idx: usize, n: usize) {
    let flash = flash_plus(
        WEAPONINFO[ctx.players[player_idx].readyweapon as usize].flashstate,
        n,
    );
    p_set_psprite(
        ctx.thinkers,
        ctx.level,
        &mut ctx.players[player_idx],
        PsprNum::Flash,
        flash,
    );
}

/// The weapon attack actions of `p_pspr.c`: `A_Punch`, `A_Saw`,
/// `A_FireMissile`, `A_FireBFG`, `A_FirePlasma`, `A_FirePistol`,
/// `A_FireShotgun`, `A_FireShotgun2`, `A_FireCGun`.
fn run_attack(
    ctx: &mut crate::p_spec::SpecialsCtx,
    validcount: i32,
    player_idx: usize,
    position: PsprNum,
    state: StateNum,
    action: StateAction,
) {
    let _ = position;
    let mo = ctx.players[player_idx].mo;
    let origin = Some(SoundOrigin::Mobj(mo));

    match action {
        StateAction::APunch => {
            let mut damage = (p_random() % 10 + 1) << 1;
            if ctx.players[player_idx].power(crate::doomdef::PowerType::PwStrength) != 0 {
                damage *= 10;
            }
            let mut angle = ctx.thinkers.mobj(mo).unwrap().angle;
            angle = angle.wrapping_add(((p_random() - p_random()) << 18) as u32);
            let (slope, linetarget) = crate::p_map::p_aim_line_attack(
                ctx.thinkers,
                ctx.level,
                validcount,
                mo,
                angle,
                MELEERANGE,
            );
            line_attack(ctx, validcount, mo, angle, MELEERANGE, slope, damage);

            // turn to face target (the original's own comment, preserved)
            if let Some(target) = linetarget {
                s_start_sound(origin, Sfx::SfxPunch);
                let (tx, ty) = {
                    let t = ctx.thinkers.mobj(target).unwrap();
                    (t.x, t.y)
                };
                let m = ctx.thinkers.mobj_mut(mo).unwrap();
                m.angle =
                    crate::r_main::angle_from_delta(tx.wrapping_sub(m.x), ty.wrapping_sub(m.y));
            }
        }
        StateAction::ASaw => {
            let damage = 2 * (p_random() % 10 + 1);
            let mut angle = ctx.thinkers.mobj(mo).unwrap().angle;
            angle = angle.wrapping_add(((p_random() - p_random()) << 18) as u32);

            // use meleerange + 1 se the puff doesn't skip the flash (the
            // original's own comment, preserved)
            let (slope, linetarget) = crate::p_map::p_aim_line_attack(
                ctx.thinkers,
                ctx.level,
                validcount,
                mo,
                angle,
                MELEERANGE + 1,
            );
            line_attack(ctx, validcount, mo, angle, MELEERANGE + 1, slope, damage);

            let Some(target) = linetarget else {
                s_start_sound(origin, Sfx::SfxSawful);
                return;
            };
            s_start_sound(origin, Sfx::SfxSawhit);

            // turn to face target (the original's own comment, preserved)
            let (tx, ty) = {
                let t = ctx.thinkers.mobj(target).unwrap();
                (t.x, t.y)
            };
            let m = ctx.thinkers.mobj_mut(mo).unwrap();
            let angle = crate::r_main::angle_from_delta(tx.wrapping_sub(m.x), ty.wrapping_sub(m.y));
            // `angle_t` arithmetic is unsigned; `-ANG90/20` is an `int`
            // (`ANG90` is a plain int literal) converted to unsigned for
            // the comparison, i.e. 2^32 - ANG90/20.
            let diff = angle.wrapping_sub(m.angle);
            if diff > crate::tables::ANG180 {
                if diff < ANG90_20.wrapping_neg() {
                    m.angle = angle.wrapping_add(ANG90_21);
                } else {
                    m.angle = m.angle.wrapping_sub(ANG90_20);
                }
            } else if diff > ANG90_20 {
                m.angle = angle.wrapping_sub(ANG90_21);
            } else {
                m.angle = m.angle.wrapping_add(ANG90_20);
            }
            m.flags |= crate::r_defs::mobj_flag::JUSTATTACKED;
        }
        StateAction::AFireMissile => {
            spend_ammo(&mut ctx.players[player_idx], 1);
            crate::p_mobj::p_spawn_player_missile(
                ctx.thinkers,
                ctx.level,
                ctx.players,
                ctx.rmain,
                validcount,
                mo,
                crate::info::MobjType::MtRocket,
            );
        }
        StateAction::AFireBFG => {
            spend_ammo(&mut ctx.players[player_idx], BFGCELLS);
            crate::p_mobj::p_spawn_player_missile(
                ctx.thinkers,
                ctx.level,
                ctx.players,
                ctx.rmain,
                validcount,
                mo,
                crate::info::MobjType::MtBfg,
            );
        }
        StateAction::AFirePlasma => {
            spend_ammo(&mut ctx.players[player_idx], 1);
            set_flash(ctx, player_idx, (p_random() & 1) as usize);
            crate::p_mobj::p_spawn_player_missile(
                ctx.thinkers,
                ctx.level,
                ctx.players,
                ctx.rmain,
                validcount,
                mo,
                crate::info::MobjType::MtPlasma,
            );
        }
        StateAction::AFirePistol => {
            s_start_sound(origin, Sfx::SfxPistol);
            player_attack_pose(ctx, mo);
            spend_ammo(&mut ctx.players[player_idx], 1);
            set_flash(ctx, player_idx, 0);
            let slope = bullet_slope(ctx, validcount, mo);
            let accurate = ctx.players[player_idx].refire == 0;
            gun_shot(ctx, validcount, mo, accurate, slope);
        }
        StateAction::AFireShotgun => {
            s_start_sound(origin, Sfx::SfxShotgn);
            player_attack_pose(ctx, mo);
            spend_ammo(&mut ctx.players[player_idx], 1);
            set_flash(ctx, player_idx, 0);
            let slope = bullet_slope(ctx, validcount, mo);
            for _ in 0..7 {
                gun_shot(ctx, validcount, mo, false, slope);
            }
        }
        StateAction::AFireShotgun2 => {
            s_start_sound(origin, Sfx::SfxDshtgn);
            player_attack_pose(ctx, mo);
            spend_ammo(&mut ctx.players[player_idx], 2);
            set_flash(ctx, player_idx, 0);
            let slope = bullet_slope(ctx, validcount, mo);
            for _ in 0..20 {
                let damage = 5 * (p_random() % 3 + 1);
                let mut angle = ctx.thinkers.mobj(mo).unwrap().angle;
                angle = angle.wrapping_add(((p_random() - p_random()) << 19) as u32);
                let spread = (p_random() - p_random()) << 5;
                line_attack(
                    ctx,
                    validcount,
                    mo,
                    angle,
                    MISSILERANGE,
                    slope + spread,
                    damage,
                );
            }
        }
        StateAction::AFireCGun => {
            s_start_sound(origin, Sfx::SfxPistol);
            let ammo = WEAPONINFO[ctx.players[player_idx].readyweapon as usize].ammo;
            if ctx.players[player_idx].ammo[ammo as usize] == 0 {
                return;
            }
            player_attack_pose(ctx, mo);
            spend_ammo(&mut ctx.players[player_idx], 1);
            // flashstate + psp->state - &states[S_CHAIN1]
            set_flash(ctx, player_idx, state as usize - StateNum::SChain1 as usize);
            let slope = bullet_slope(ctx, validcount, mo);
            let accurate = ctx.players[player_idx].refire == 0;
            gun_shot(ctx, validcount, mo, accurate, slope);
        }
        _ => unreachable!("not a weapon attack action: {action:?}"),
    }
}

/// `(sprite, frame)` for a [`PSpr`], as `r_things::PSprite`'s
/// `sprite`/`frame` fields want them (`psp->state->sprite`/`frame`,
/// which may include [`crate::r_things::FF_FULLBRIGHT`] baked into the
/// state's `frame` already — no OR-ing needed here). The 6e game loop
/// is expected to use this when it builds a frame's
/// [`crate::r_things::PlayerSprites`] from a live [`Player`].
pub fn psprite_sprite_frame(psp: &PSpr) -> Option<(SpriteNum, i32)> {
    let state = STATES[psp.state? as usize];
    Some((state.sprite, state.frame))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::MobjType;
    use crate::m_fixed::FRACUNIT;
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
    fn setup_psprites_brings_up_the_ready_weapon() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut player = blank_player(&mut thinkers, &mut level);

        p_setup_psprites(&mut thinkers, &mut level, &mut player);

        let psp = player.psprites[PsprNum::Weapon as usize];
        assert_eq!(psp.state, Some(StateNum::SPistolup));
        assert_eq!(player.readyweapon, WeaponType::WpPistol);
        assert_eq!(player.pendingweapon, WeaponType::WpNochange);
    }

    #[test]
    fn move_psprites_raises_the_weapon_until_ready() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut player = blank_player(&mut thinkers, &mut level);
        p_setup_psprites(&mut thinkers, &mut level, &mut player);

        // S_PISTOLUP's tics is 1 (a_raise moves sy by RAISESPEED each
        // tic starting at WEAPONBOTTOM=128*FRACUNIT down to
        // WEAPONTOP=32*FRACUNIT, i.e. 16 tics of RAISESPEED=6*FRACUNIT).
        for _ in 0..20 {
            p_move_psprites(&mut thinkers, &mut level, &mut player);
        }

        let psp = player.psprites[PsprNum::Weapon as usize];
        assert_eq!(
            psp.state,
            Some(StateNum::SPistol),
            "should reach the ready state"
        );
        assert_eq!(psp.sy, WEAPONTOP);
    }

    #[test]
    fn check_ammo_with_no_clip_ammo_switches_to_fist() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut player = blank_player(&mut thinkers, &mut level);
        player.ammo[crate::doomdef::AmmoType::AmClip as usize] = 0;

        let ok = p_check_ammo(&mut thinkers, &mut level, &mut player);
        assert!(!ok);
        assert_eq!(player.pendingweapon, WeaponType::WpFist);
    }

    #[test]
    fn check_ammo_with_ammo_available_returns_true() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut player = blank_player(&mut thinkers, &mut level);

        let ok = p_check_ammo(&mut thinkers, &mut level, &mut player);
        assert!(ok);
    }

    #[test]
    fn drop_weapon_starts_the_down_state() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut player = blank_player(&mut thinkers, &mut level);
        // Deliberately skip p_setup_psprites here: A_Lower's "is already
        // down" check (`sy < WEAPONBOTTOM`) is sy-position-dependent,
        // and a fresh player's psprite sy defaults to 0 — far above
        // WEAPONBOTTOM (128*FRACUNIT) even after one LOWERSPEED step,
        // so the down animation is still mid-flight (stays in
        // SPistoldown) rather than completing in this single call, same
        // as the original whenever A_Lower doesn't reach the bottom in
        // one tic.

        p_drop_weapon(&mut thinkers, &mut level, &mut player);

        let psp = player.psprites[PsprNum::Weapon as usize];
        assert_eq!(psp.state, Some(StateNum::SPistoldown));
    }

    #[test]
    fn weapon_sound_cues_are_queued_with_the_player_as_origin() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let mut player = blank_player(&mut thinkers, &mut level);

        crate::s_sound::test_init(4);
        // chainsaw up
        player.pendingweapon = WeaponType::WpChainsaw;
        p_bring_up_weapon(&mut thinkers, &mut level, &mut player);
        // super shotgun reload cues + BFG charge
        for action in [
            StateAction::AOpenShotgun2,
            StateAction::ALoadShotgun2,
            StateAction::ABFGsound,
        ] {
            dispatch_psprite_action(
                &mut thinkers,
                &mut level,
                &mut player,
                PsprNum::Weapon,
                action,
            );
        }
        crate::s_sound::test_flush(&thinkers, &level);

        // Every cue has the same origin (the player's mobj), and a
        // sound replaces its origin's previous one (S_StopSound(origin)
        // first), so only the last survives.
        assert_eq!(crate::s_sound::test_playing(), vec![Sfx::SfxBfg as usize]);
        crate::s_sound::s_shutdown();
    }

    /// Owns everything a `SpecialsCtx` borrows, for the attack tests.
    struct World {
        thinkers: Thinkers,
        level: Level,
        players: Vec<Player>,
        rmain: crate::r_main::RMain,
        rdata: crate::r_data::RData,
        wad: WadFiles,
        active_plats: crate::p_plats::ActivePlats,
        active_ceilings: crate::p_ceilng::ActiveCeilings,
        switches: crate::p_switch::SwitchState,
    }

    impl World {
        fn new() -> Option<World> {
            let (wad, mut level) = load_e1m1()?;
            let mut thinkers = Thinkers::new();
            let mut player = blank_player(&mut thinkers, &mut level);
            thinkers.mobj_mut(player.mo).unwrap().player = Some(0);
            player.readyweapon = WeaponType::WpPistol;
            player.ammo[crate::doomdef::AmmoType::AmClip as usize] = 50;
            Some(World {
                thinkers,
                level,
                players: vec![player],
                rmain: crate::r_main::RMain::new(),
                rdata: crate::r_data::RData::default(),
                wad,
                active_plats: Default::default(),
                active_ceilings: Default::default(),
                switches: Default::default(),
            })
        }

        fn ctx(&mut self) -> crate::p_spec::SpecialsCtx<'_> {
            crate::p_spec::SpecialsCtx {
                thinkers: &mut self.thinkers,
                level: &mut self.level,
                players: &mut self.players,
                rmain: &mut self.rmain,
                rdata: &self.rdata,
                wad: &self.wad,
                active_plats: &mut self.active_plats,
                active_ceilings: &mut self.active_ceilings,
                switches: &mut self.switches,
            }
        }

        /// A target `units` east of the player (who faces east, angle 0).
        fn target_ahead(&mut self, units: i32) -> ThinkerId {
            let mo = self.players[0].mo;
            let (x, y) = {
                let m = self.thinkers.mobj(mo).unwrap();
                (m.x, m.y)
            };
            p_spawn_mobj(
                &mut self.thinkers,
                &mut self.level,
                x + units * FRACUNIT,
                y,
                SpawnZ::OnFloor,
                MobjType::MtPossessed,
            )
        }

        /// Presses fire and runs psprite tics until `done` says the
        /// attack action has happened (at most 30 tics).
        fn fire_until(&mut self, done: impl Fn(&World) -> bool) {
            {
                let ctx = self.ctx();
                p_fire_weapon(ctx.thinkers, ctx.level, &mut ctx.players[0]);
            }
            for _ in 0..30 {
                if done(self) {
                    return;
                }
                let mut ctx = self.ctx();
                p_move_psprites_full(&mut ctx, 1, 0);
            }
            panic!("the attack action never ran");
        }
    }

    #[test]
    fn firing_the_pistol_spends_ammo_flashes_and_hurts_the_target() {
        let Some(mut w) = World::new() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        crate::s_sound::test_init(4);
        let target = w.target_ahead(96);
        let before = w.thinkers.mobj(target).unwrap().health;

        w.fire_until(|w| w.players[0].ammo[crate::doomdef::AmmoType::AmClip as usize] == 49);

        let p = &w.players[0];
        assert_eq!(p.ammo[crate::doomdef::AmmoType::AmClip as usize], 49);
        assert_eq!(
            p.psprites[PsprNum::Flash as usize].state,
            Some(StateNum::SPistolflash)
        );
        assert!(p.pending_pspr.is_empty());
        assert_eq!(w.thinkers.mobj(p.mo).unwrap().state, StateNum::SPlayAtk2);
        assert!(
            w.thinkers.mobj(target).unwrap().health < before,
            "the first pistol shot is accurate and must hit a target dead ahead"
        );
        crate::s_sound::test_flush(&w.thinkers, &w.level);
        assert_eq!(
            crate::s_sound::test_playing(),
            vec![Sfx::SfxPistol as usize]
        );
        crate::s_sound::s_shutdown();
    }

    #[test]
    fn punching_hurts_a_close_target_and_turns_toward_it() {
        let Some(mut w) = World::new() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        crate::s_sound::test_init(4);
        w.players[0].readyweapon = WeaponType::WpFist;
        let target = w.target_ahead(40);
        // Off to the side a little, so turning toward it is visible.
        w.thinkers.mobj_mut(target).unwrap().y += 8 * FRACUNIT;
        let before = w.thinkers.mobj(target).unwrap().health;

        w.fire_until(|w| w.thinkers.mobj(target).unwrap().health < before);

        assert!(w.thinkers.mobj(target).unwrap().health < before);
        let mo = w.thinkers.mobj(w.players[0].mo).unwrap();
        assert_ne!(mo.angle, 0, "the punch turns the player toward its target");
        crate::s_sound::test_flush(&w.thinkers, &w.level);
        assert!(crate::s_sound::test_playing().contains(&(Sfx::SfxPunch as usize)));
        crate::s_sound::s_shutdown();
    }

    #[test]
    fn chaingun_flash_frame_follows_the_firing_state_and_needs_ammo() {
        let Some(mut w) = World::new() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        w.players[0].readyweapon = WeaponType::WpChaingun;
        w.target_ahead(96);
        {
            let mut ctx = w.ctx();
            // second firing frame (S_CHAIN2) picks the second flash.
            ctx.players[0].psprites[PsprNum::Weapon as usize].state = Some(StateNum::SChain2);
            ctx.players[0].pending_pspr.push(PendingPspr::Attack {
                position: PsprNum::Weapon,
                state: StateNum::SChain2,
                action: StateAction::AFireCGun,
            });
            run_pending(&mut ctx, 1, 0);
        }
        assert_eq!(
            w.players[0].ammo[crate::doomdef::AmmoType::AmClip as usize],
            49
        );
        assert_eq!(
            w.players[0].psprites[PsprNum::Flash as usize].state,
            Some(StateNum::SChainflash2)
        );

        // No ammo: the sound plays but nothing else happens.
        w.players[0].ammo[crate::doomdef::AmmoType::AmClip as usize] = 0;
        w.players[0].psprites[PsprNum::Flash as usize].state = None;
        {
            let mut ctx = w.ctx();
            ctx.players[0].pending_pspr.push(PendingPspr::Attack {
                position: PsprNum::Weapon,
                state: StateNum::SChain1,
                action: StateAction::AFireCGun,
            });
            run_pending(&mut ctx, 2, 0);
        }
        assert_eq!(w.players[0].psprites[PsprNum::Flash as usize].state, None);
    }
}
