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
//
//-----------------------------------------------------------------------------

//! Rust port of `d_player.h` (`player_t` itself now ported, see
//! below).
//!
//! # Scope
//!
//! [`Player`] (`player_t`) is ported as of Phase 6d, now that `Mobj`/
//! `ThinkerId` (Phase 6b) and `PSpr`/`p_pspr.rs` (also 6d) exist for it
//! to embed. `mo: mobj_t*` becomes `mo: ThinkerId` (the player's mobj is
//! always live once spawned — never a dangling reference the way
//! `target`/`tracer` can be, so this isn't `Option<ThinkerId>`);
//! `attacker: mobj_t*` becomes `Option<ThinkerId>` (`NULL` for
//! floors/ceilings, per the original's own comment). `psprites` is
//! `[PSpr; NUMPSPRITES]`, each with its own `state: Option<StateNum>`
//! for the original's `pspdef_t.state == NULL` — see [`PSpr`]'s docs on
//! why the `Option` is a field, not the whole struct.
//!
//! `message: char*` (Phase 7a) is `Option<&'static str>` — every write
//! this port's ported code makes (`P_TouchSpecialThing`, `p_doors.c`'s
//! locked-door messages) assigns a `static const char*` literal from
//! `dstrings.h` (`d_englsh.rs` here), never a heap/formatted string, so
//! `&'static str` captures that exactly. (The one exception anywhere in
//! the original is `g_game.c`'s turbo-cheat message, `sprintf`'d into a
//! local buffer — HUD-only text unrelated to anything Phase 7 writes,
//! left for whichever later phase ports it.) Only *writing* `message`
//! is in scope now — drawing it on screen is still `HU_Erase`/
//! `HU_Drawer`'s job (Phase 9), not ported yet.

use crate::d_ticcmd::TicCmd;
use crate::doomdef::{
    Card, PowerType, WeaponType, MAXPLAYERS, NUMAMMO, NUMCARDS, NUMPOWERS, NUMWEAPONS,
};
use crate::p_pspr::{PSpr, NUMPSPRITES};
use crate::p_tick::ThinkerId;

/// Player states (`playerstate_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    /// Playing or camping.
    Live,
    /// Dead on the ground, view follows killer.
    Dead,
    /// Ready to restart/respawn???
    Reborn,
}

/// Player internal flags, for cheats and debug (`cheat_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Cheat {
    /// No clipping, walk through barriers (`CF_NOCLIP` = 1).
    NoClip = 1,
    /// No damage, no health loss (`CF_GODMODE` = 2).
    GodMode = 2,
    /// Not really a cheat, just a debug aid (`CF_NOMOMENTUM` = 4).
    NoMomentum = 4,
}

/// Extended player object info (`player_t`). See module docs.
#[derive(Debug, Clone)]
pub struct Player {
    /// The player's mobj — always live once spawned (`P_SpawnPlayer`),
    /// unlike `target`/`tracer` which can go stale. See module docs.
    pub mo: ThinkerId,
    pub playerstate: PlayerState,
    pub cmd: TicCmd,

    // Determine POV, including viewpoint bobbing during movement. Focal
    // origin above r.z (the original's own comment, preserved).
    pub viewz: crate::m_fixed::Fixed,
    /// Base height above floor for viewz.
    pub viewheight: crate::m_fixed::Fixed,
    /// Bob/squat speed.
    pub deltaviewheight: crate::m_fixed::Fixed,
    /// Bounded/scaled total momentum.
    pub bob: crate::m_fixed::Fixed,

    /// This is only used between levels, `mo.health` is used during
    /// levels (the original's own comment, preserved).
    pub health: i32,
    pub armorpoints: i32,
    /// Armor type is 0-2.
    pub armortype: i32,

    /// Power ups. invinc and invis are tic counters.
    pub powers: [i32; NUMPOWERS],
    pub cards: [bool; NUMCARDS],
    pub backpack: bool,

    /// Frags, kills of other players.
    pub frags: [i32; MAXPLAYERS as usize],
    pub readyweapon: WeaponType,
    /// Is `wp_nochange` if not changing.
    pub pendingweapon: WeaponType,

    pub weaponowned: [bool; NUMWEAPONS],
    pub ammo: [i32; NUMAMMO],
    pub maxammo: [i32; NUMAMMO],

    /// True if button down last tic.
    pub attackdown: bool,
    pub usedown: bool,

    /// Bit flags, for cheats and debug. See [`Cheat`].
    pub cheats: i32,

    /// Refired shots are less accurate.
    pub refire: i32,

    // For intermission stats.
    pub killcount: i32,
    pub itemcount: i32,
    pub secretcount: i32,

    /// Hint messages. See module docs on why this is `&'static str`.
    pub message: Option<&'static str>,

    /// For screen flashing (red or bright).
    pub damagecount: i32,
    pub bonuscount: i32,

    /// Who did damage (`None` for floors/ceilings).
    pub attacker: Option<ThinkerId>,

    /// So gun flashes light up areas.
    pub extralight: i32,

    /// Current PLAYPAL, can be set to REDCOLORMAP for pain, etc.
    pub fixedcolormap: i32,

    /// Player skin colorshift, 0-3 for which color to draw player.
    pub colormap: i32,

    /// Overlay view sprites (gun, etc). See [`PSpr`]'s docs on its own
    /// `state: Option<StateNum>` field.
    pub psprites: [PSpr; NUMPSPRITES],

    /// True if secret level has been done.
    pub didsecret: bool,
    /// Weapon work deferred by `p_pspr` until the game context is at hand
    /// (see `p_pspr`'s module docs). Not part of the original `player_t`.
    pub pending_pspr: Vec<crate::p_pspr::PendingPspr>,
}

/// (`card_t`/[`Card`] indexing helper) — the powers array by [`PowerType`],
/// and the ammo/weaponowned arrays by [`AmmoType`]/[`WeaponType`], all
/// need `as usize` at call sites since Rust doesn't index arrays by
/// arbitrary enums; kept as plain arrays (matching the original's plain
/// C arrays) rather than introducing a wrapper type for this phase.
impl Player {
    /// A zeroed `player_t` (`memset(p, 0, sizeof(*p))`), in the
    /// `PST_REBORN` state a player has before its first spawn. `mo` is
    /// only a placeholder until `P_SpawnPlayer` sets the real mobj.
    pub fn blank(mo: ThinkerId) -> Player {
        Player {
            mo,
            playerstate: PlayerState::Reborn,
            cmd: TicCmd::default(),
            viewz: 0,
            viewheight: 0,
            deltaviewheight: 0,
            bob: 0,
            health: 0,
            armorpoints: 0,
            armortype: 0,
            powers: [0; NUMPOWERS],
            cards: [false; NUMCARDS],
            backpack: false,
            frags: [0; MAXPLAYERS as usize],
            readyweapon: WeaponType::WpFist,
            pendingweapon: WeaponType::WpFist,
            weaponowned: [false; NUMWEAPONS],
            ammo: [0; NUMAMMO],
            maxammo: [0; NUMAMMO],
            attackdown: false,
            usedown: false,
            cheats: 0,
            refire: 0,
            killcount: 0,
            itemcount: 0,
            secretcount: 0,
            message: None,
            damagecount: 0,
            bonuscount: 0,
            attacker: None,
            extralight: 0,
            fixedcolormap: 0,
            colormap: 0,
            psprites: [PSpr::default(); NUMPSPRITES],
            didsecret: false,
            pending_pspr: Vec::new(),
        }
    }

    pub fn power(&self, p: PowerType) -> i32 {
        self.powers[p as usize]
    }

    pub fn has_card(&self, c: Card) -> bool {
        self.cards[c as usize]
    }

    /// A minimal but fully-initialized `Player` around an already-spawned
    /// mobj, for `p_pspr`/`p_user`/later tests that need a `Player` to
    /// call ported functions against without hand-writing every field
    /// at each call site. Not a stand-in for real player spawning (that
    /// remains `P_SpawnPlayer`, still not fully ported — see
    /// `p_setup`'s module docs); just a shared test fixture.
    #[doc(hidden)]
    pub fn for_test(mo: ThinkerId) -> Player {
        Player {
            mo,
            playerstate: PlayerState::Live,
            cmd: TicCmd::default(),
            viewz: 0,
            viewheight: 0,
            deltaviewheight: 0,
            bob: 0,
            health: 100,
            armorpoints: 0,
            armortype: 0,
            powers: [0; NUMPOWERS],
            cards: [false; NUMCARDS],
            backpack: false,
            frags: [0; MAXPLAYERS as usize],
            readyweapon: WeaponType::WpPistol,
            pendingweapon: WeaponType::WpNochange,
            weaponowned: {
                let mut w = [false; NUMWEAPONS];
                w[WeaponType::WpFist as usize] = true;
                w[WeaponType::WpPistol as usize] = true;
                w
            },
            ammo: [50, 0, 0, 0],
            maxammo: [200, 50, 300, 50],
            attackdown: false,
            usedown: false,
            cheats: 0,
            refire: 0,
            killcount: 0,
            itemcount: 0,
            secretcount: 0,
            message: None,
            damagecount: 0,
            bonuscount: 0,
            attacker: None,
            extralight: 0,
            fixedcolormap: 0,
            colormap: 0,
            psprites: [PSpr::default(); NUMPSPRITES],
            didsecret: false,
            pending_pspr: Vec::new(),
        }
    }
}

/// INTERMISSION. Structure passed e.g. to `WI_Start(wb)`
/// (`wbplayerstruct_t`).
#[derive(Debug, Clone, Copy, Default)]
pub struct WbPlayerStruct {
    /// Whether the player is in game.
    pub in_game: bool,

    // Player stats, kills, collected items etc.
    pub skills: i32,
    pub sitems: i32,
    pub ssecret: i32,
    pub stime: i32,
    pub frags: [i32; 4],
    /// Current score on entry, modified on return.
    pub score: i32,
}

/// (`wbstartstruct_t`)
#[derive(Debug, Clone, Copy)]
pub struct WbStartStruct {
    /// Episode # (0-2).
    pub epsd: i32,

    /// If true, splash the secret level.
    pub didsecret: bool,

    /// Previous and next levels, origin 0.
    pub last: i32,
    pub next: i32,

    pub maxkills: i32,
    pub maxitems: i32,
    pub maxsecret: i32,
    pub maxfrags: i32,

    /// The par time.
    pub partime: i32,

    /// Index of this player in game.
    pub pnum: i32,

    pub plyr: [WbPlayerStruct; MAXPLAYERS as usize],
}

impl Default for WbStartStruct {
    fn default() -> Self {
        WbStartStruct {
            epsd: 0,
            didsecret: false,
            last: 0,
            next: 0,
            maxkills: 0,
            maxitems: 0,
            maxsecret: 0,
            maxfrags: 0,
            partime: 0,
            pnum: 0,
            plyr: [WbPlayerStruct::default(); MAXPLAYERS as usize],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cheat_values_match_original_defines() {
        assert_eq!(Cheat::NoClip as i32, 1);
        assert_eq!(Cheat::GodMode as i32, 2);
        assert_eq!(Cheat::NoMomentum as i32, 4);
    }

    #[test]
    fn wbstartstruct_default_has_maxplayers_entries() {
        let w = WbStartStruct::default();
        assert_eq!(w.plyr.len(), 4);
    }

    #[test]
    fn wbplayerstruct_default_is_zeroed() {
        let p = WbPlayerStruct::default();
        assert!(!p.in_game);
        assert_eq!(p.frags, [0; 4]);
        assert_eq!(p.score, 0);
    }
}
