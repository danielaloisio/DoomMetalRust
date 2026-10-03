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
//	Handling interactions (i.e., collisions).
//
//-----------------------------------------------------------------------------

//! Rust port of `p_inter.h` / `p_inter.c`. Handling interactions
//! (i.e., collisions) (the original's own description, preserved).
//!
//! # Scope
//!
//! Ported: [`p_give_ammo`]/[`p_give_weapon`]/[`p_give_body`]/
//! [`p_give_armor`]/[`p_give_card`]/[`p_give_power`] (`P_GiveAmmo`/
//! `P_GiveWeapon`/`P_GiveBody`/`P_GiveArmor`/`P_GiveCard`/`P_GivePower`),
//! [`p_touch_special_thing`] (`P_TouchSpecialThing`), [`p_kill_mobj`]
//! (`P_KillMobj`), [`p_damage_mobj`] (`P_DamageMobj`) — everything in
//! this file. This closes the loop the Phase 6d weapon state machine
//! left open (`TODO(Phase 7)` at every attack-action call site): once
//! Phase 7d ports the attack functions, they'll call
//! [`p_damage_mobj`]/[`p_touch_special_thing`] for real.
//!
//! Not ported (deferred, with the original function/reason noted, same
//! convention as every other not-yet-ported call in this port):
//! - `AM_Stop` (`P_KillMobj`'s "switch off the automap before dying"
//!   call) — automap is Phase 9; nothing to switch off yet.
//! - `I_Tactile` (`P_DamageMobj`'s joystick force-feedback call) — the
//!   original's own implementation is `on = off = total = 0;`, i.e. a
//!   no-op (`i_system.c`), so there is nothing to port here either.
//! - `R_PointToAngle2`'s use in `P_DamageMobj`'s knockback-angle
//!   calculation needs [`crate::r_main::RMain`] (for its `viewx`/
//!   `viewy` side effect, see `p_map.rs`'s docs on the same pattern) —
//!   [`p_damage_mobj`] takes `&mut RMain` and saves/restores it, same
//!   as `p_map::p_hit_slide_line`.
//!
//! # `frags[]` / `source->player - players`
//!
//! `P_KillMobj` indexes `frags[]` by `target->player - players`
//! (pointer arithmetic recovering a player index from a `player_t*`).
//! This port's [`crate::r_defs::Mobj::player`] is already that index
//! (`Option<usize>`, see `r_defs`'s docs) — no arithmetic needed, just
//! read the field. `frags[]` itself is write-only in this phase (no
//! deathmatch scoreboard exists yet, Phase 9/11), but is still written
//! faithfully.

use crate::d_englsh as msg;
use crate::d_player::Player;
use crate::doomdef::{AmmoType, Card, GameMode, PowerType, WeaponType, INVISTICS, INVULNTICS};
use crate::doomstat::{self};
use crate::info::MobjType;
use crate::m_fixed::{fixed_mul, FRACUNIT};
use crate::m_random::p_random;
use crate::p_mobj::{p_set_mobj_state, p_spawn_mobj, SpawnZ};
use crate::p_setup::Level;
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_defs::mobj_flag;
use crate::r_main::RMain;
use crate::tables::{ANG180, ANGLETOFINESHIFT, FINESINE};

/// (`MAXHEALTH`, `p_local.h`).
pub const MAXHEALTH: i32 = 100;
/// (`BASETHRESHOLD`, `p_local.h`).
pub const BASETHRESHOLD: i32 = 100;
/// A weapon is found with two clip loads, a big item has five clip
/// loads (the original's own comment, preserved). (`maxammo[NUMAMMO]`
/// — same name/values as `doomstat`'s `GameState::maxammo`, this is the
/// original's *other*, `p_inter.c`-local `maxammo[]` used only to seed
/// `P_GiveAmmo`'s cap semantics conceptually; kept here since nothing
/// else reads it and duplicating a 4-entry array is cheaper than
/// threading it through from elsewhere.)
const MAXAMMO: [i32; 4] = [200, 50, 300, 50];
/// (`clipammo[NUMAMMO]`).
const CLIPAMMO: [i32; 4] = [10, 4, 20, 1];
/// (`BONUSADD`, `p_inter.c`'s own `#define`).
const BONUSADD: i32 = 6;

/// Port of `P_GiveAmmo`. Num is the number of clip loads, not the
/// individual count (0 = 1/2 clip). Returns false if the ammo can't be
/// picked up at all (the original's own comment, preserved).
///
/// # Panics
/// Panics (standing in for `I_Error`) if `ammo` somehow indexes outside
/// `NUMAMMO` — can't happen through this enum's finite variants, kept
/// only for parity with the original's own defensive check.
pub fn p_give_ammo(player: &mut Player, ammo: AmmoType, num: i32) -> bool {
    if ammo == AmmoType::AmNoammo {
        return false;
    }

    if player.ammo[ammo as usize] == player.maxammo[ammo as usize] {
        return false;
    }

    let mut num = if num != 0 {
        num * CLIPAMMO[ammo as usize]
    } else {
        CLIPAMMO[ammo as usize] / 2
    };

    let skill = doomstat::state().gameskill;
    if skill == crate::doomdef::Skill::Baby || skill == crate::doomdef::Skill::Nightmare {
        // give double ammo in trainer mode, you'll need in nightmare
        // (the original's own comment, preserved).
        num <<= 1;
    }

    let oldammo = player.ammo[ammo as usize];
    player.ammo[ammo as usize] += num;
    if player.ammo[ammo as usize] > player.maxammo[ammo as usize] {
        player.ammo[ammo as usize] = player.maxammo[ammo as usize];
    }

    // If non zero ammo, don't change up weapons, player was lower on
    // purpose (the original's own comment, preserved).
    if oldammo != 0 {
        return true;
    }

    // We were down to zero, so select a new weapon. Preferences are not
    // user selectable (the original's own comment, preserved).
    match ammo {
        AmmoType::AmClip => {
            if player.readyweapon == WeaponType::WpFist {
                player.pendingweapon = if player.weaponowned[WeaponType::WpChaingun as usize] {
                    WeaponType::WpChaingun
                } else {
                    WeaponType::WpPistol
                };
            }
        }
        AmmoType::AmShell => {
            if (player.readyweapon == WeaponType::WpFist
                || player.readyweapon == WeaponType::WpPistol)
                && player.weaponowned[WeaponType::WpShotgun as usize]
            {
                player.pendingweapon = WeaponType::WpShotgun;
            }
        }
        AmmoType::AmCell => {
            if (player.readyweapon == WeaponType::WpFist
                || player.readyweapon == WeaponType::WpPistol)
                && player.weaponowned[WeaponType::WpPlasma as usize]
            {
                player.pendingweapon = WeaponType::WpPlasma;
            }
        }
        AmmoType::AmMisl => {
            if player.readyweapon == WeaponType::WpFist
                && player.weaponowned[WeaponType::WpMissile as usize]
            {
                player.pendingweapon = WeaponType::WpMissile;
            }
        }
        AmmoType::AmNoammo => {}
    }

    true
}

/// Port of `P_GiveWeapon`. The weapon name may have a `MF_DROPPED` flag
/// ored in (the original's own comment, preserved).
/// `is_console` is the original's `player == &players[consoleplayer]`.
pub fn p_give_weapon(
    player: &mut Player,
    is_console: bool,
    weapon: WeaponType,
    dropped: bool,
) -> bool {
    let info = crate::d_items::WEAPONINFO[weapon as usize];
    let state = doomstat::state();

    // `deathmatch != 2` in the original: weapons stay in coop and plain
    // deathmatch, but not in altdeath.
    if state.netgame && !(state.deathmatch && state.altdeath) && !dropped {
        // leave placed weapons forever on net games (the original's own
        // comment, preserved).
        if player.weaponowned[weapon as usize] {
            return false;
        }

        player.bonuscount += BONUSADD;
        player.weaponowned[weapon as usize] = true;

        if state.deathmatch {
            p_give_ammo(player, info.ammo, 5);
        } else {
            p_give_ammo(player, info.ammo, 2);
        }
        player.pendingweapon = weapon;

        if is_console {
            crate::s_sound::s_start_sound(None, crate::sounds::Sfx::SfxWpnup);
        }
        return false;
    }

    let gaveammo = if info.ammo != AmmoType::AmNoammo {
        // give one clip with a dropped weapon, two clips with a found
        // weapon (the original's own comment, preserved).
        if dropped {
            p_give_ammo(player, info.ammo, 1)
        } else {
            p_give_ammo(player, info.ammo, 2)
        }
    } else {
        false
    };

    let gaveweapon = if player.weaponowned[weapon as usize] {
        false
    } else {
        player.weaponowned[weapon as usize] = true;
        player.pendingweapon = weapon;
        true
    };

    gaveweapon || gaveammo
}

/// Port of `P_GiveBody`. Returns false if the body isn't needed at all
/// (the original's own comment, preserved).
pub fn p_give_body(thinkers: &mut Thinkers, player: &mut Player, num: i32) -> bool {
    if player.health >= MAXHEALTH {
        return false;
    }

    player.health += num;
    if player.health > MAXHEALTH {
        player.health = MAXHEALTH;
    }
    thinkers.mobj_mut(player.mo).unwrap().health = player.health;

    true
}

/// Port of `P_GiveArmor`. Returns false if the armor is worse than the
/// current armor (the original's own comment, preserved).
pub fn p_give_armor(player: &mut Player, armortype: i32) -> bool {
    let hits = armortype * 100;
    if player.armorpoints >= hits {
        return false; // don't pick up (the original's own comment, preserved)
    }

    player.armortype = armortype;
    player.armorpoints = hits;

    true
}

/// Port of `P_GiveCard`.
pub fn p_give_card(player: &mut Player, card: Card) {
    if player.cards[card as usize] {
        return;
    }

    player.bonuscount = BONUSADD;
    player.cards[card as usize] = true;
}

/// Port of `P_GivePower`.
pub fn p_give_power(thinkers: &mut Thinkers, player: &mut Player, power: PowerType) -> bool {
    match power {
        PowerType::PwInvulnerability => {
            player.powers[power as usize] = INVULNTICS;
            true
        }
        PowerType::PwInvisibility => {
            player.powers[power as usize] = INVISTICS;
            thinkers.mobj_mut(player.mo).unwrap().flags |= mobj_flag::SHADOW;
            true
        }
        PowerType::PwInfrared => {
            player.powers[power as usize] = crate::doomdef::INFRATICS;
            true
        }
        PowerType::PwIronfeet => {
            player.powers[power as usize] = crate::doomdef::IRONTICS;
            true
        }
        PowerType::PwStrength => {
            p_give_body(thinkers, player, 100);
            player.powers[power as usize] = 1;
            true
        }
        PowerType::PwAllmap => {
            if player.powers[power as usize] != 0 {
                return false; // already got it (the original's own comment, preserved)
            }
            player.powers[power as usize] = 1;
            true
        }
    }
}

/// Port of `P_TouchSpecialThing`. Identifies the special by its sprite,
/// same as the original (`switch (special->sprite)`).
///
/// `players` stands in for the original's global `players[MAXPLAYERS]`
/// — `toucher`'s `mobj.player` (an index into it, see `r_defs::Mobj`'s
/// docs) is resolved against it, same convention
/// [`crate::p_tick::p_ticker`] already established.
///
/// # Panics
/// Panics (standing in for `I_Error`) if `special`'s sprite doesn't
/// match any pickup — same fatal-error semantics as the original
/// (`P_SpecialThing: Unknown gettable thing`).
pub fn p_touch_special_thing(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    special: ThinkerId,
    toucher: ThinkerId,
) {
    let (special_z, special_sprite, special_flags) = {
        let m = thinkers.mobj(special).unwrap();
        (m.z, m.sprite, m.flags)
    };
    let (toucher_z, toucher_height, toucher_health, player_idx) = {
        let m = thinkers.mobj(toucher).unwrap();
        (m.z, m.height, m.health, m.player)
    };

    let delta = special_z - toucher_z;
    if delta > toucher_height || delta < -8 * FRACUNIT {
        return; // out of reach (the original's own comment, preserved)
    }

    // Dead thing touching. Can happen with a sliding player corpse (the
    // original's own comment, preserved).
    if toucher_health <= 0 {
        return;
    }

    // Only players pick things up in the original too (every
    // `P_TouchSpecialThing` caller passes a player's mobj as `toucher`)
    // — guard kept explicit since this port's `player` field is
    // `Option`, unlike the original's implicit "toucher is always a
    // player here" assumption.
    let Some(player_idx) = player_idx else {
        return;
    };
    let console = player_idx as i32 == doomstat::state().consoleplayer;
    let player = &mut players[player_idx];

    let mut sound = crate::sounds::Sfx::SfxItemup;

    use crate::info::SpriteNum::*;
    match special_sprite {
        // armor (the original's own comment, preserved)
        SprArm1 => {
            if !p_give_armor(player, 1) {
                return;
            }
            player.message = Some(msg::GOTARMOR);
        }
        SprArm2 => {
            if !p_give_armor(player, 2) {
                return;
            }
            player.message = Some(msg::GOTMEGA);
        }

        // bonus items (the original's own comment, preserved)
        SprBon1 => {
            player.health += 1; // can go over 100% (the original's own comment, preserved)
            if player.health > 200 {
                player.health = 200;
            }
            thinkers.mobj_mut(toucher).unwrap().health = player.health;
            player.message = Some(msg::GOTHTHBONUS);
        }
        SprBon2 => {
            player.armorpoints += 1; // can go over 100%
            if player.armorpoints > 200 {
                player.armorpoints = 200;
            }
            if player.armortype == 0 {
                player.armortype = 1;
            }
            player.message = Some(msg::GOTARMBONUS);
        }
        SprSoul => {
            player.health += 100;
            if player.health > 200 {
                player.health = 200;
            }
            thinkers.mobj_mut(toucher).unwrap().health = player.health;
            player.message = Some(msg::GOTSUPER);
            sound = crate::sounds::Sfx::SfxGetpow;
        }
        SprMega => {
            if doomstat::state().gamemode != GameMode::Commercial {
                return;
            }
            player.health = 200;
            thinkers.mobj_mut(toucher).unwrap().health = player.health;
            p_give_armor(player, 2);
            player.message = Some(msg::GOTMSPHERE);
            sound = crate::sounds::Sfx::SfxGetpow;
        }

        // cards — leave cards for everyone (the original's own comment,
        // preserved)
        SprBkey => {
            if !player.cards[Card::ItBluecard as usize] {
                player.message = Some(msg::GOTBLUECARD);
            }
            p_give_card(player, Card::ItBluecard);
            if doomstat::state().netgame {
                return;
            }
        }
        SprYkey => {
            if !player.cards[Card::ItYellowcard as usize] {
                player.message = Some(msg::GOTYELWCARD);
            }
            p_give_card(player, Card::ItYellowcard);
            if doomstat::state().netgame {
                return;
            }
        }
        SprRkey => {
            if !player.cards[Card::ItRedcard as usize] {
                player.message = Some(msg::GOTREDCARD);
            }
            p_give_card(player, Card::ItRedcard);
            if doomstat::state().netgame {
                return;
            }
        }
        SprBsku => {
            if !player.cards[Card::ItBlueskull as usize] {
                player.message = Some(msg::GOTBLUESKUL);
            }
            p_give_card(player, Card::ItBlueskull);
            if doomstat::state().netgame {
                return;
            }
        }
        SprYsku => {
            if !player.cards[Card::ItYellowskull as usize] {
                player.message = Some(msg::GOTYELWSKUL);
            }
            p_give_card(player, Card::ItYellowskull);
            if doomstat::state().netgame {
                return;
            }
        }
        SprRsku => {
            if !player.cards[Card::ItRedskull as usize] {
                player.message = Some(msg::GOTREDSKULL);
            }
            p_give_card(player, Card::ItRedskull);
            if doomstat::state().netgame {
                return;
            }
        }

        // medikits, heals (the original's own comment, preserved)
        SprStim => {
            if !p_give_body(thinkers, player, 10) {
                return;
            }
            player.message = Some(msg::GOTSTIM);
        }
        SprMedi => {
            if !p_give_body(thinkers, player, 25) {
                return;
            }
            player.message = Some(if player.health < 25 {
                msg::GOTMEDINEED
            } else {
                msg::GOTMEDIKIT
            });
        }

        // power ups (the original's own comment, preserved)
        SprPinv => {
            if !p_give_power(thinkers, player, PowerType::PwInvulnerability) {
                return;
            }
            player.message = Some(msg::GOTINVUL);
            sound = crate::sounds::Sfx::SfxGetpow;
        }
        SprPstr => {
            if !p_give_power(thinkers, player, PowerType::PwStrength) {
                return;
            }
            player.message = Some(msg::GOTBERSERK);
            if player.readyweapon != WeaponType::WpFist {
                player.pendingweapon = WeaponType::WpFist;
            }
            sound = crate::sounds::Sfx::SfxGetpow;
        }
        SprPins => {
            if !p_give_power(thinkers, player, PowerType::PwInvisibility) {
                return;
            }
            player.message = Some(msg::GOTINVIS);
            sound = crate::sounds::Sfx::SfxGetpow;
        }
        SprSuit => {
            if !p_give_power(thinkers, player, PowerType::PwIronfeet) {
                return;
            }
            player.message = Some(msg::GOTSUIT);
            sound = crate::sounds::Sfx::SfxGetpow;
        }
        SprPmap => {
            if !p_give_power(thinkers, player, PowerType::PwAllmap) {
                return;
            }
            player.message = Some(msg::GOTMAP);
            sound = crate::sounds::Sfx::SfxGetpow;
        }
        SprPvis => {
            if !p_give_power(thinkers, player, PowerType::PwInfrared) {
                return;
            }
            player.message = Some(msg::GOTVISOR);
            sound = crate::sounds::Sfx::SfxGetpow;
        }

        // ammo (the original's own comment, preserved)
        SprClip => {
            let ok = if special_flags & mobj_flag::DROPPED != 0 {
                p_give_ammo(player, AmmoType::AmClip, 0)
            } else {
                p_give_ammo(player, AmmoType::AmClip, 1)
            };
            if !ok {
                return;
            }
            player.message = Some(msg::GOTCLIP);
        }
        SprAmmo => {
            if !p_give_ammo(player, AmmoType::AmClip, 5) {
                return;
            }
            player.message = Some(msg::GOTCLIPBOX);
        }
        SprRock => {
            if !p_give_ammo(player, AmmoType::AmMisl, 1) {
                return;
            }
            player.message = Some(msg::GOTROCKET);
        }
        SprBrok => {
            if !p_give_ammo(player, AmmoType::AmMisl, 5) {
                return;
            }
            player.message = Some(msg::GOTROCKBOX);
        }
        SprCell => {
            if !p_give_ammo(player, AmmoType::AmCell, 1) {
                return;
            }
            player.message = Some(msg::GOTCELL);
        }
        SprCelp => {
            if !p_give_ammo(player, AmmoType::AmCell, 5) {
                return;
            }
            player.message = Some(msg::GOTCELLBOX);
        }
        SprShel => {
            if !p_give_ammo(player, AmmoType::AmShell, 1) {
                return;
            }
            player.message = Some(msg::GOTSHELLS);
        }
        SprSbox => {
            if !p_give_ammo(player, AmmoType::AmShell, 5) {
                return;
            }
            player.message = Some(msg::GOTSHELLBOX);
        }
        SprBpak => {
            if !player.backpack {
                for i in 0..crate::doomdef::NUMAMMO {
                    player.maxammo[i] *= 2;
                }
                player.backpack = true;
            }
            for i in 0..crate::doomdef::NUMAMMO {
                p_give_ammo(player, ammo_from_index(i), 1);
            }
            player.message = Some(msg::GOTBACKPACK);
        }

        // weapons (the original's own comment, preserved)
        SprBfug => {
            if !p_give_weapon(player, console, WeaponType::WpBfg, false) {
                return;
            }
            player.message = Some(msg::GOTBFG9000);
            sound = crate::sounds::Sfx::SfxWpnup;
        }
        SprMgun => {
            if !p_give_weapon(
                player,
                console,
                WeaponType::WpChaingun,
                special_flags & mobj_flag::DROPPED != 0,
            ) {
                return;
            }
            player.message = Some(msg::GOTCHAINGUN);
            sound = crate::sounds::Sfx::SfxWpnup;
        }
        SprCsaw => {
            if !p_give_weapon(player, console, WeaponType::WpChainsaw, false) {
                return;
            }
            player.message = Some(msg::GOTCHAINSAW);
            sound = crate::sounds::Sfx::SfxWpnup;
        }
        SprLaun => {
            if !p_give_weapon(player, console, WeaponType::WpMissile, false) {
                return;
            }
            player.message = Some(msg::GOTLAUNCHER);
            sound = crate::sounds::Sfx::SfxWpnup;
        }
        SprPlas => {
            if !p_give_weapon(player, console, WeaponType::WpPlasma, false) {
                return;
            }
            player.message = Some(msg::GOTPLASMA);
            sound = crate::sounds::Sfx::SfxWpnup;
        }
        SprShot => {
            if !p_give_weapon(
                player,
                console,
                WeaponType::WpShotgun,
                special_flags & mobj_flag::DROPPED != 0,
            ) {
                return;
            }
            player.message = Some(msg::GOTSHOTGUN);
            sound = crate::sounds::Sfx::SfxWpnup;
        }
        SprSgn2 => {
            if !p_give_weapon(
                player,
                console,
                WeaponType::WpSupershotgun,
                special_flags & mobj_flag::DROPPED != 0,
            ) {
                return;
            }
            player.message = Some(msg::GOTSHOTGUN2);
            sound = crate::sounds::Sfx::SfxWpnup;
        }

        other => panic!("P_SpecialThing: Unknown gettable thing ({other:?})"),
    }

    if special_flags & mobj_flag::COUNTITEM != 0 {
        player.itemcount += 1;
    }
    crate::p_mobj::p_remove_mobj(thinkers, level, special);
    player.bonuscount += BONUSADD;
    if console {
        crate::s_sound::s_start_sound(None, sound);
    }
}

/// (`ammotype_t` from a `NUMAMMO`-range index) — [`AmmoType`] has no
/// generated `from_index` the way [`MobjType`] does (see `info.rs`'s
/// generator), since it isn't part of the generated `info.c` tables;
/// this is the one small hand-written equivalent [`p_touch_special_thing`]
/// needs for its backpack loop (`for (i=0 ; i<NUMAMMO ; i++)
/// P_GiveAmmo(player, i, 1)`, iterating the enum by raw index in the
/// original).
fn ammo_from_index(i: usize) -> AmmoType {
    match i {
        0 => AmmoType::AmClip,
        1 => AmmoType::AmShell,
        2 => AmmoType::AmCell,
        3 => AmmoType::AmMisl,
        _ => unreachable!("NUMAMMO is 4"),
    }
}

/// Port of `P_KillMobj`. `players` resolves `target`/`source`'s
/// `mobj.player` indices (see [`p_touch_special_thing`]'s docs on the
/// same convention) — `source` and `target` are [`ThinkerId`]s, not
/// `Option<&mut Player>`, since this function may need *both* players'
/// structs live at once (`source`'s `frags[]`, `target`'s
/// `playerstate`), which two separate `&mut` borrows into `players`
/// can't express — indices are resolved and the slice indexed at each
/// use instead, exactly where the original dereferences `source->player`/
/// `target->player`.
pub fn p_kill_mobj(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    source: Option<ThinkerId>,
    target: ThinkerId,
) {
    {
        let t = thinkers.mobj_mut(target).unwrap();
        t.flags &= !(mobj_flag::SHOOTABLE | mobj_flag::FLOAT | mobj_flag::SKULLFLY);
        if t.mobj_type != MobjType::MtSkull {
            t.flags &= !mobj_flag::NOGRAVITY;
        }
        t.flags |= mobj_flag::CORPSE | mobj_flag::DROPOFF;
        t.height >>= 2;
    }

    let (target_type, target_player) = {
        let t = thinkers.mobj(target).unwrap();
        (t.mobj_type, t.player)
    };
    let source_player = source.and_then(|s| thinkers.mobj(s).unwrap().player);

    if let Some(source_player_idx) = source_player {
        // count for intermission (the original's own comment, preserved).
        if thinkers.mobj(target).unwrap().flags & mobj_flag::COUNTKILL != 0 {
            players[source_player_idx].killcount += 1;
        }
        if let Some(target_player_idx) = target_player {
            players[source_player_idx].frags[target_player_idx] += 1;
        }
    } else if !doomstat::state().netgame
        && thinkers.mobj(target).unwrap().flags & mobj_flag::COUNTKILL != 0
    {
        // count all monster deaths, even those caused by other monsters
        // (the original's own comment, preserved).
        players[0].killcount += 1;
    }

    if let Some(target_player_idx) = target_player {
        // count environment kills against you (the original's own
        // comment, preserved).
        if source.is_none() {
            players[target_player_idx].frags[target_player_idx] += 1;
        }

        thinkers.mobj_mut(target).unwrap().flags &= !mobj_flag::SOLID;
        players[target_player_idx].playerstate = crate::d_player::PlayerState::Dead;
        crate::p_pspr::p_drop_weapon(thinkers, level, &mut players[target_player_idx]);

        // TODO(Phase 9): AM_Stop() when target_player_idx ==
        // consoleplayer && automapactive — see module docs.
    }

    let (health, spawnhealth, xdeathstate, deathstate) = {
        let t = thinkers.mobj(target).unwrap();
        (
            t.health,
            t.info.spawnhealth,
            t.info.xdeathstate,
            t.info.deathstate,
        )
    };
    if health < -spawnhealth && xdeathstate != crate::info::StateNum::SNull {
        p_set_mobj_state(thinkers, level, target, xdeathstate, |_, _, _, _| {});
    } else {
        p_set_mobj_state(thinkers, level, target, deathstate, |_, _, _, _| {});
    }

    if let Some(t) = thinkers.mobj_mut(target) {
        t.tics -= p_random() & 3;
        if t.tics < 1 {
            t.tics = 1;
        }
    }
    // (mobj may have removed itself via S_NULL in the death chain,
    // matching the original — `p_set_mobj_state` already unlinked it,
    // the `if let Some` above just tolerates that, same as `P_KillMobj`
    // itself would dereference a freed pointer in the original, which
    // this port never does.)

    // Drop stuff. This determines the kind of object spawned during the
    // death frame of a thing (the original's own comment, preserved).
    let item = match target_type {
        MobjType::MtWolfss | MobjType::MtPossessed => MobjType::MtClip,
        MobjType::MtShotguy => MobjType::MtShotgun,
        MobjType::MtChainguy => MobjType::MtChaingun,
        _ => return,
    };

    let (tx, ty) = {
        let Some(t) = thinkers.mobj(target) else {
            return;
        };
        (t.x, t.y)
    };
    let mo = p_spawn_mobj(thinkers, level, tx, ty, SpawnZ::OnFloor, item);
    thinkers.mobj_mut(mo).unwrap().flags |= mobj_flag::DROPPED; // special versions of items (the original's own comment, preserved)
}

/// Port of `P_DamageMobj`. Damages both enemies and players.
/// "inflictor" is the thing that caused the damage — creature or
/// missile, can be `None` (slime, etc). "source" is the thing to target
/// after taking damage — creature or `None`. Source and inflictor are
/// the same for melee attacks. Source can be `None` for slime, barrel
/// explosions and other environmental stuff (the original's own
/// comment, preserved).
///
/// See module docs on `R_PointToAngle2`'s `viewx`/`viewy` side effect
/// and why this needs `&mut RMain`.
#[allow(clippy::too_many_arguments)]
pub fn p_damage_mobj(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    target: ThinkerId,
    inflictor: Option<ThinkerId>,
    source: Option<ThinkerId>,
    mut damage: i32,
) {
    let target_flags = thinkers.mobj(target).unwrap().flags;
    if target_flags & mobj_flag::SHOOTABLE == 0 {
        return; // shouldn't happen... (the original's own comment, preserved)
    }
    if thinkers.mobj(target).unwrap().health <= 0 {
        return;
    }

    if target_flags & mobj_flag::SKULLFLY != 0 {
        let t = thinkers.mobj_mut(target).unwrap();
        t.momx = 0;
        t.momy = 0;
        t.momz = 0;
    }

    let player_idx = thinkers.mobj(target).unwrap().player;
    if player_idx.is_some() && doomstat::state().gameskill == crate::doomdef::Skill::Baby {
        damage >>= 1; // take half damage in trainer mode (the original's own comment, preserved)
    }

    // Some close combat weapons should not inflict thrust and push the
    // victim out of reach, thus kick away unless using the chainsaw
    // (the original's own comment, preserved).
    let source_uses_chainsaw = source
        .and_then(|s| thinkers.mobj(s).unwrap().player)
        .is_some_and(|source_player_idx| {
            players[source_player_idx].readyweapon == WeaponType::WpChainsaw
        });

    if let Some(inflictor) = inflictor {
        if target_flags & mobj_flag::NOCLIP == 0 && !source_uses_chainsaw {
            let (inf_x, inf_y, inf_z) = {
                let m = thinkers.mobj(inflictor).unwrap();
                (m.x, m.y, m.z)
            };
            let (tx, ty, tz, tmass, thealth) = {
                let m = thinkers.mobj(target).unwrap();
                (m.x, m.y, m.z, m.info.mass, m.health)
            };

            let (saved_viewx, saved_viewy) = (rmain.viewx, rmain.viewy);
            let mut ang = rmain.point_to_angle2(inf_x, inf_y, tx, ty);
            rmain.viewx = saved_viewx;
            rmain.viewy = saved_viewy;

            let mut thrust = damage * (FRACUNIT >> 3) * 100 / tmass;

            // make fall forwards sometimes (the original's own comment,
            // preserved).
            if damage < 40
                && damage > thealth
                && tz - inf_z > 64 * FRACUNIT
                && (p_random() & 1) != 0
            {
                ang = ang.wrapping_add(ANG180);
                thrust *= 4;
            }

            let fine = (ang >> ANGLETOFINESHIFT) as usize;
            let t = thinkers.mobj_mut(target).unwrap();
            t.momx = t
                .momx
                .wrapping_add(fixed_mul(thrust, crate::tables::fine_cosine(fine)));
            t.momy = t.momy.wrapping_add(fixed_mul(thrust, FINESINE[fine]));
        }
    }

    // player specific (the original's own comment, preserved).
    if let Some(player_idx) = player_idx {
        let sector = {
            let t = thinkers.mobj(target).unwrap();
            level.subsectors[t.subsector.unwrap()].sector
        };
        // end of game hell hack (the original's own comment, preserved).
        if level.sectors[sector].special == 11 && damage >= thinkers.mobj(target).unwrap().health {
            damage = thinkers.mobj(target).unwrap().health - 1;
        }

        let player = &mut players[player_idx];

        // Below certain threshold, ignore damage in GOD mode, or with
        // INVUL power (the original's own comment, preserved).
        if damage < 1000
            && (player.cheats & crate::d_player::Cheat::GodMode as i32 != 0
                || player.power(PowerType::PwInvulnerability) != 0)
        {
            return;
        }

        if player.armortype != 0 {
            let mut saved = if player.armortype == 1 {
                damage / 3
            } else {
                damage / 2
            };

            if player.armorpoints <= saved {
                // armor is used up (the original's own comment, preserved).
                saved = player.armorpoints;
                player.armortype = 0;
            }
            player.armorpoints -= saved;
            damage -= saved;
        }

        player.health -= damage; // mirror mobj health here for Dave (the original's own comment, preserved)
        if player.health < 0 {
            player.health = 0;
        }

        player.attacker = source;
        player.damagecount += damage; // add damage after armor/invuln (the original's own comment, preserved)
        if player.damagecount > 100 {
            player.damagecount = 100; // teleport stomp does 10k points... (the original's own comment, preserved)
        }

        // TODO: I_Tactile(...) when player == &players[consoleplayer] —
        // the original's own implementation is a no-op, see module
        // docs.
    }

    // do the damage (the original's own comment, preserved).
    {
        let t = thinkers.mobj_mut(target).unwrap();
        t.health -= damage;
    }
    if thinkers.mobj(target).unwrap().health <= 0 {
        p_kill_mobj(thinkers, level, players, source, target);
        return;
    }

    let (painchance, target_flags) = {
        let t = thinkers.mobj(target).unwrap();
        (t.info.painchance, t.flags)
    };
    if p_random() < painchance && target_flags & mobj_flag::SKULLFLY == 0 {
        thinkers.mobj_mut(target).unwrap().flags |= mobj_flag::JUSTHIT; // fight back! (the original's own comment, preserved)
        let painstate = thinkers.mobj(target).unwrap().info.painstate;
        p_set_mobj_state(thinkers, level, target, painstate, |_, _, _, _| {});
    }

    thinkers.mobj_mut(target).unwrap().reactiontime = 0; // we're awake now... (the original's own comment, preserved)

    let (threshold, target_type) = {
        let t = thinkers.mobj(target).unwrap();
        (t.threshold, t.mobj_type)
    };
    let source_type = source.map(|s| thinkers.mobj(s).unwrap().mobj_type);
    if (threshold == 0 || target_type == MobjType::MtVile)
        && source.is_some_and(|s| s != target)
        && source_type != Some(MobjType::MtVile)
    {
        // if not intent on another player, chase after this one (the
        // original's own comment, preserved).
        let t = thinkers.mobj_mut(target).unwrap();
        t.target = source;
        t.threshold = BASETHRESHOLD;

        let (state, spawnstate, seestate) = {
            let t = thinkers.mobj(target).unwrap();
            (t.state, t.info.spawnstate, t.info.seestate)
        };
        if state == spawnstate && seestate != crate::info::StateNum::SNull {
            p_set_mobj_state(thinkers, level, target, seestate, |_, _, _, _| {});
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d_player::Player;
    use crate::info::{MobjType, SpriteNum};
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

    const START_X: crate::m_fixed::Fixed = 1056 * FRACUNIT;
    const START_Y: crate::m_fixed::Fixed = -3616 * FRACUNIT;

    fn spawn_player(thinkers: &mut Thinkers, level: &mut Level) -> (ThinkerId, Player) {
        let mo = p_spawn_mobj(
            thinkers,
            level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        // P_SpawnPlayer (not fully ported, see p_setup's module docs)
        // is what sets mobj->player in the original; every function
        // under test here reads it (P_TouchSpecialThing/P_DamageMobj/
        // P_KillMobj all branch on "is the toucher/target a player"),
        // so tests must set it themselves.
        thinkers.mobj_mut(mo).unwrap().player = Some(0);
        (mo, Player::for_test(mo))
    }

    #[test]
    fn give_ammo_caps_at_max_and_switches_weapon_from_empty() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let (_mo, mut player) = spawn_player(&mut thinkers, &mut level);
        // for_test seeds 50 clip ammo already; zero it to exercise the
        // "was at zero, switch weapons" branch.
        player.ammo[AmmoType::AmClip as usize] = 0;
        player.readyweapon = WeaponType::WpFist;

        let gave = p_give_ammo(&mut player, AmmoType::AmClip, 1);
        assert!(gave);
        // doomstat::GameState defaults to Skill::Baby, which doubles
        // pickup ammo (the original's own "trainer mode" rule).
        assert_eq!(
            player.ammo[AmmoType::AmClip as usize],
            CLIPAMMO[AmmoType::AmClip as usize] * 2
        );
        assert_eq!(player.pendingweapon, WeaponType::WpPistol);
    }

    #[test]
    fn give_ammo_at_max_returns_false() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let (_mo, mut player) = spawn_player(&mut thinkers, &mut level);
        player.ammo[AmmoType::AmClip as usize] = player.maxammo[AmmoType::AmClip as usize];
        assert!(!p_give_ammo(&mut player, AmmoType::AmClip, 1));
    }

    #[test]
    fn give_armor_rejects_a_downgrade() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let (_mo, mut player) = spawn_player(&mut thinkers, &mut level);
        assert!(p_give_armor(&mut player, 2));
        assert_eq!(player.armorpoints, 200);
        assert!(
            !p_give_armor(&mut player, 1),
            "1 (100 pts) is worse than the 200 already held"
        );
    }

    #[test]
    fn touch_special_thing_stimpack_heals_and_removes_the_item() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let (player_mo, mut player) = spawn_player(&mut thinkers, &mut level);
        player.health = 50;
        let mut players = vec![player];

        let stim = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtMisc10, // S_STIM (health bonus) — see info.rs; MtMisc10 carries SPR_STIM.
        );
        assert_eq!(thinkers.mobj(stim).unwrap().sprite, SpriteNum::SprStim);

        p_touch_special_thing(&mut thinkers, &mut level, &mut players, stim, player_mo);

        assert_eq!(players[0].health, 60);
        assert_eq!(players[0].message, Some(msg::GOTSTIM));
        assert!(!thinkers.is_live(stim), "the pickup should remove itself");
    }

    #[test]
    fn damage_mobj_reduces_health_and_sets_pain_state_or_kills() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let (player_mo, player) = spawn_player(&mut thinkers, &mut level);
        let mut players = vec![player];
        let mut rmain = RMain::new();

        let start_health = players[0].health;
        p_damage_mobj(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            player_mo,
            None,
            None,
            10,
        );

        // doomstat::GameState defaults to Skill::Baby, which halves
        // damage taken by a player (the original's own "trainer mode"
        // rule).
        assert_eq!(players[0].health, start_health - 5);
        assert!(thinkers.is_live(player_mo));
    }

    #[test]
    fn damage_mobj_lethal_damage_kills_the_player() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let (player_mo, mut player) = spawn_player(&mut thinkers, &mut level);
        player.health = 1;
        thinkers.mobj_mut(player_mo).unwrap().health = 1;
        let mut players = vec![player];
        let mut rmain = RMain::new();

        p_damage_mobj(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            player_mo,
            None,
            None,
            100,
        );

        assert_eq!(players[0].health, 0);
        assert_eq!(players[0].playerstate, crate::d_player::PlayerState::Dead);
    }

    #[test]
    fn kill_mobj_drops_a_clip_for_a_possessed() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        // Single-player always has players[0] live (matches the
        // original's own assumption in the "count all monster deaths"
        // non-netgame branch, which always indexes players[0]).
        let (_player_mo, player) = spawn_player(&mut thinkers, &mut level);
        let mut players = vec![player];

        let possessed = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPossessed,
        );

        let before = thinkers.iter_mobjs().count();
        p_kill_mobj(&mut thinkers, &mut level, &mut players, None, possessed);
        let after_types: Vec<MobjType> = thinkers.iter_mobjs().map(|(_, m)| m.mobj_type).collect();

        assert!(
            thinkers.iter_mobjs().count() > before,
            "a clip should have been spawned"
        );
        assert!(after_types.contains(&MobjType::MtClip));
    }
}
