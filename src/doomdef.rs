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
//  DoomDef - basic defines for DOOM, e.g. Version, game mode
//   and skill level, and display parameters.
//
//-----------------------------------------------------------------------------

//! Rust port of `doomdef.h` / `doomdef.c`.
//!
//! Global parameters/defines used virtually everywhere: DOOM version,
//! game mode/mission/language identification, screen dimensions, key
//! codes, and the top-level game state enum. `doomdef.c` itself has no
//! logic ("Location for any defines turned variables. None.").

/// DOOM engine version, as reported by the original source (`VERSION`).
pub const VERSION: i32 = 110;

/// Game mode handling - identify IWAD version to handle IWAD dependent
/// animations etc. (`GameMode_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    /// DOOM 1 shareware, E1, M9
    Shareware,
    /// DOOM 1 registered, E3, M27
    Registered,
    /// DOOM 2 retail, E1 M34
    Commercial,
    /// DOOM 1 retail, E4, M36
    Retail,
    /// No IWAD found.
    Indetermined,
}

/// Mission packs - might be useful for TC stuff? (`GameMission_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMission {
    /// DOOM 1
    Doom,
    /// DOOM 2
    Doom2,
    /// TNT mission pack
    PackTnt,
    /// Plutonia pack
    PackPlut,
    None,
}

/// Identify language to use, software localization (`Language_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    French,
    German,
    Unknown,
}

/// For resize of screen, at start of game. It will not work dynamically,
/// see visplanes. (`BASE_WIDTH`)
pub const BASE_WIDTH: i32 = 320;

/// It is educational but futile to change this scaling e.g. to 2. Drawing
/// of status bar, menus etc. is tied to the scale implied by the
/// graphics. (`SCREEN_MUL`)
pub const SCREEN_MUL: i32 = 1;
/// (`INV_ASPECT_RATIO`) — 0.75 would be ideal.
pub const INV_ASPECT_RATIO: f64 = 0.625;

/// (`SCREENWIDTH`) = `SCREEN_MUL*BASE_WIDTH`
pub const SCREENWIDTH: i32 = 320;
/// (`SCREENHEIGHT`) = `(int)(SCREEN_MUL*BASE_WIDTH*INV_ASPECT_RATIO)`
pub const SCREENHEIGHT: i32 = 200;

/// The maximum number of players, multiplayer/networking (`MAXPLAYERS`).
pub const MAXPLAYERS: i32 = 4;

/// State updates, number of tics / second (`TICRATE`).
pub const TICRATE: i32 = 35;

/// The current state of the game: whether we are playing, gazing at the
/// intermission screen, the game final animation, or a demo.
/// (`gamestate_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    Level,
    Intermission,
    Finale,
    DemoScreen,
}

// Difficulty/skill settings/filters.

/// Skill flag: easy (`MTF_EASY`).
pub const MTF_EASY: i32 = 1;
/// Skill flag: normal (`MTF_NORMAL`).
pub const MTF_NORMAL: i32 = 2;
/// Skill flag: hard (`MTF_HARD`).
pub const MTF_HARD: i32 = 4;
/// Deaf monsters/do not react to sound (`MTF_AMBUSH`).
pub const MTF_AMBUSH: i32 = 8;

/// (`skill_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Skill {
    Baby,
    Easy,
    Medium,
    Hard,
    Nightmare,
}

impl Skill {
    /// The safe inverse of `as usize` (used by the savegame reader).
    pub fn from_index(i: usize) -> Option<Skill> {
        [
            Skill::Baby,
            Skill::Easy,
            Skill::Medium,
            Skill::Hard,
            Skill::Nightmare,
        ]
        .get(i)
        .copied()
    }
}

/// Key cards (`card_t`).
///
/// Variants keep the original's `it_*` prefix, see [`WeaponType`]'s note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum Card {
    ItBluecard,
    ItYellowcard,
    ItRedcard,
    ItBlueskull,
    ItYellowskull,
    ItRedskull,
}

/// (`NUMCARDS`)
pub const NUMCARDS: usize = 6;

/// The defined weapons, including a marker indicating user has not
/// changed weapon (`weapontype_t`).
///
/// Variants keep the original's `wp_*` prefix (as `Wp*`) rather than
/// clippy's suggested prefix-free names, to stay grep-able against the
/// C source during the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum WeaponType {
    WpFist,
    WpPistol,
    WpShotgun,
    WpChaingun,
    WpMissile,
    WpPlasma,
    WpBfg,
    WpChainsaw,
    WpSupershotgun,
    /// No pending weapon change.
    WpNochange,
}

/// (`NUMWEAPONS`)
pub const NUMWEAPONS: usize = 9;

/// Ammunition types defined (`ammotype_t`).
///
/// Variants keep the original's `am_*` prefix, see [`WeaponType`]'s note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum AmmoType {
    /// Pistol / chaingun ammo.
    AmClip,
    /// Shotgun / double barreled shotgun.
    AmShell,
    /// Plasma rifle, BFG.
    AmCell,
    /// Missile launcher.
    AmMisl,
    /// Unlimited for chainsaw / fist.
    AmNoammo,
}

/// (`NUMAMMO`)
pub const NUMAMMO: usize = 4;

/// Power up artifacts (`powertype_t`).
///
/// Variants keep the original's `pw_*` prefix, see [`WeaponType`]'s note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum PowerType {
    PwInvulnerability,
    PwStrength,
    PwInvisibility,
    PwIronfeet,
    PwAllmap,
    PwInfrared,
}

/// (`NUMPOWERS`)
pub const NUMPOWERS: usize = 6;

// Power up durations, how many seconds till expiration, assuming TICRATE
// is 35 ticks/second (`powerduration_t`).

/// (`INVULNTICS`) = `30*TICRATE`
pub const INVULNTICS: i32 = 30 * TICRATE;
/// (`INVISTICS`) = `60*TICRATE`
pub const INVISTICS: i32 = 60 * TICRATE;
/// (`INFRATICS`) = `120*TICRATE`
pub const INFRATICS: i32 = 120 * TICRATE;
/// (`IRONTICS`) = `60*TICRATE`
pub const IRONTICS: i32 = 60 * TICRATE;

// DOOM keyboard definition. This is the stuff configured by Setup.Exe.
// Most key data are simple ascii (uppercased).

pub const KEY_RIGHTARROW: i32 = 0xae;
pub const KEY_LEFTARROW: i32 = 0xac;
pub const KEY_UPARROW: i32 = 0xad;
pub const KEY_DOWNARROW: i32 = 0xaf;
pub const KEY_STRAFELEFT: i32 = 44;
pub const KEY_STRAFERIGHT: i32 = 46;
pub const KEY_ESCAPE: i32 = 27;
pub const KEY_ENTER: i32 = 13;
pub const KEY_TAB: i32 = 9;
pub const KEY_F1: i32 = 0x80 + 0x3b;
pub const KEY_F2: i32 = 0x80 + 0x3c;
pub const KEY_F3: i32 = 0x80 + 0x3d;
pub const KEY_F4: i32 = 0x80 + 0x3e;
pub const KEY_F5: i32 = 0x80 + 0x3f;
pub const KEY_F6: i32 = 0x80 + 0x40;
pub const KEY_F7: i32 = 0x80 + 0x41;
pub const KEY_F8: i32 = 0x80 + 0x42;
pub const KEY_F9: i32 = 0x80 + 0x43;
pub const KEY_F10: i32 = 0x80 + 0x44;
pub const KEY_F11: i32 = 0x80 + 0x57;
pub const KEY_F12: i32 = 0x80 + 0x58;

pub const KEY_BACKSPACE: i32 = 127;
pub const KEY_PAUSE: i32 = 0xff;

pub const KEY_EQUALS: i32 = 0x3d;
pub const KEY_MINUS: i32 = 0x2d;

pub const KEY_RSHIFT: i32 = 0x80 + 0x36;
pub const KEY_RCTRL: i32 = 0x80 + 0x1d;
pub const KEY_RALT: i32 = 0x80 + 0x38;

pub const KEY_LALT: i32 = KEY_RALT;

impl WeaponType {
    /// Every variant in declaration order (the original's enum values), so
    /// `ALL[v as usize] == v`.
    pub const ALL: [WeaponType; 10] = [
        WeaponType::WpFist,
        WeaponType::WpPistol,
        WeaponType::WpShotgun,
        WeaponType::WpChaingun,
        WeaponType::WpMissile,
        WeaponType::WpPlasma,
        WeaponType::WpBfg,
        WeaponType::WpChainsaw,
        WeaponType::WpSupershotgun,
        WeaponType::WpNochange,
    ];

    /// The safe inverse of `as usize` (used by the savegame reader).
    pub fn from_index(i: usize) -> Option<WeaponType> {
        Self::ALL.get(i).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_dimensions_match_original() {
        assert_eq!(SCREENWIDTH, 320);
        assert_eq!(SCREENHEIGHT, 200);
    }

    #[test]
    fn power_durations_match_original() {
        assert_eq!(INVULNTICS, 1050);
        assert_eq!(INVISTICS, 2100);
        assert_eq!(INFRATICS, 4200);
        assert_eq!(IRONTICS, 2100);
    }

    #[test]
    fn function_key_codes_match_original() {
        assert_eq!(KEY_F1, 0xbb);
        assert_eq!(KEY_F12, 0xd8);
        assert_eq!(KEY_LALT, KEY_RALT);
    }
}
