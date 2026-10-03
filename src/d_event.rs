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

//! Rust port of `d_event.h`.
//!
//! Event handling: input event types, the event structure, game actions,
//! and button/action code definitions.

/// Input event types (`evtype_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvType {
    KeyDown,
    KeyUp,
    Mouse,
    Joystick,
}

/// Event structure (`event_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub event_type: EvType,
    /// keys / mouse/joystick buttons
    pub data1: i32,
    /// mouse/joystick x move
    pub data2: i32,
    /// mouse/joystick y move
    pub data3: i32,
}

/// (`gameaction_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameAction {
    Nothing,
    LoadLevel,
    NewGame,
    LoadGame,
    SaveGame,
    PlayDemo,
    Completed,
    Victory,
    WorldDone,
    Screenshot,
}

// Button/action code definitions (`buttoncode_t`). The original packs
// these into one C enum despite the values overlapping across
// unrelated bit-groups (BT_ATTACK=1 and BTS_PAUSE=1 share a numeric
// value, for instance — they're never compared against each other,
// only masked into different byte positions of `ticcmd_t::buttons`).
// Kept as plain constants rather than a single enum for exactly that
// reason: a Rust enum requires distinct discriminants, and forcing one
// here would misrepresent the original's actual bit-layout intent.

/// Press "Fire" (`BT_ATTACK`).
pub const BT_ATTACK: i32 = 1;
/// Use button, to open doors, activate switches (`BT_USE`).
pub const BT_USE: i32 = 2;

/// Flag: game events, not really buttons (`BT_SPECIAL`).
pub const BT_SPECIAL: i32 = 128;
pub const BT_SPECIALMASK: i32 = 3;

/// Flag, weapon change pending. If true, the next 3 bits hold weapon
/// num (`BT_CHANGE`).
pub const BT_CHANGE: i32 = 4;
/// The 3-bit weapon mask and shift, convenience.
pub const BT_WEAPONMASK: i32 = 8 + 16 + 32;
pub const BT_WEAPONSHIFT: i32 = 3;

/// Pause the game (`BTS_PAUSE`).
pub const BTS_PAUSE: i32 = 1;
/// Save the game at each console (`BTS_SAVEGAME`).
pub const BTS_SAVEGAME: i32 = 2;

/// Savegame slot numbers occupy the second byte of buttons.
pub const BTS_SAVEMASK: i32 = 4 + 8 + 16;
pub const BTS_SAVESHIFT: i32 = 2;

/// (`MAXEVENTS`)
pub const MAXEVENTS: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_constants_match_original_defines() {
        assert_eq!(BT_ATTACK, 1);
        assert_eq!(BT_USE, 2);
        assert_eq!(BT_SPECIAL, 128);
        assert_eq!(BT_SPECIALMASK, 3);
        assert_eq!(BT_CHANGE, 4);
        assert_eq!(BT_WEAPONMASK, 56);
        assert_eq!(BT_WEAPONSHIFT, 3);
        assert_eq!(BTS_PAUSE, 1);
        assert_eq!(BTS_SAVEGAME, 2);
        assert_eq!(BTS_SAVEMASK, 28);
        assert_eq!(BTS_SAVESHIFT, 2);
    }

    #[test]
    fn maxevents_matches_original() {
        assert_eq!(MAXEVENTS, 64);
    }
}
