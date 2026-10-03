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
//	System specific interface stuff.
//
//-----------------------------------------------------------------------------

//! Rust port of `d_ticcmd.h`.
//!
//! System specific interface stuff — the data sampled per tick (single
//! player) and transmitted to other peers (multiplayer). Mainly
//! movements/button commands per game tick, plus a checksum for
//! internal state consistency.

use crate::doomtype::Byte;

/// (`ticcmd_t`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TicCmd {
    /// `*2048` for move.
    pub forwardmove: i8,
    /// `*2048` for move.
    pub sidemove: i8,
    /// `<<16` for angle delta.
    pub angleturn: i16,
    /// Checks for net game.
    pub consistancy: i16,
    pub chatchar: Byte,
    pub buttons: Byte,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ticcmd_is_all_zero() {
        let cmd = TicCmd::default();
        assert_eq!(cmd.forwardmove, 0);
        assert_eq!(cmd.sidemove, 0);
        assert_eq!(cmd.angleturn, 0);
        assert_eq!(cmd.consistancy, 0);
        assert_eq!(cmd.chatchar, 0);
        assert_eq!(cmd.buttons, 0);
    }
}
