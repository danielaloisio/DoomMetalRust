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
//	Globally defined strings.
//
//-----------------------------------------------------------------------------

//! Rust port of `dstrings.c` (the `endmsg` quit-message array) plus the
//! `dstrings.h` constants not tied to a specific language.
//!
//! `endmsg` has a latent bug in the original: several array
//! initializer entries are missing their trailing comma (see
//! `dstrings.c`), so adjacent string literals silently concatenate
//! per C's string-literal-concatenation rule instead of becoming
//! separate array elements. The array is declared as
//! `[NUM_QUITMESSAGES+1]` (23 slots) but only 21 comma-separated
//! initializers are actually given, two of which are really two
//! messages joined into one string; C zero-initializes the
//! remaining trailing slots, i.e. 2 trailing `NULL` entries. This
//! is reproduced exactly (21 `Some` entries, none of them at the
//! 'intended' 22/23 message boundary, plus 2 trailing `None`s) —
//! "port fiel" preserves the original's actual compiled behavior,
//! not what the comments suggest it meant to do.

use crate::d_englsh::{NUM_QUITMESSAGES, QUITMSG};

/// Port of `endmsg[NUM_QUITMESSAGES+1]`. `None` entries correspond
/// to the C array's implicit zero-initialized (NULL) trailing
/// slots, never actually assigned a message in the original.
pub const ENDMSG: [Option<&str>; NUM_QUITMESSAGES + 1] = [
    Some(QUITMSG),
    Some("please don't leave, there's more\ndemons to toast!"),
    Some("let's beat it -- this is turning\ninto a bloodbath!"),
    Some("i wouldn't leave if i were you.\ndos is much worse."),
    Some("you're trying to say you like dos\nbetter than me, right?"),
    Some("don't leave yet -- there's a\ndemon around that corner!"),
    Some("ya know, next time you come in here\ni'm gonna toast ya."),
    Some("go ahead and leave. see if i care.you want to quit?\nthen, thou hast lost an eighth!"),
    Some("don't go now, there's a \ndimensional shambler waiting\nat the dos prompt!"),
    Some("get outta here and go back\nto your boring programs."),
    Some("if i were your boss, i'd \n deathmatch ya in a minute!"),
    Some("look, bud. you leave now\nand you forfeit your body count!"),
    Some("just leave. when you come\nback, i'll be waiting with a bat."),
    Some("you're lucky i don't smack\nyou for thinking about leaving.fuck you, pussy!\nget the fuck out!"),
    Some("you quit and i'll jizz\nin your cystholes!"),
    Some("if you leave, i'll make\nthe lord drink my jizz."),
    Some("hey, ron! can we say\n'fuck' in the game?"),
    Some("i'd leave: this is just\nmore monsters and levels.\nwhat a load."),
    Some("suck it down, asshole!\nyou're a fucking wimp!"),
    Some("don't quit now! we're \nstill spending your money!"),
    Some("THIS IS NO MESSAGE!\nPage intentionally left blank."),
    None,
    None,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endmsg_has_21_populated_and_2_null_entries() {
        let populated = ENDMSG.iter().filter(|e| e.is_some()).count();
        let empty = ENDMSG.iter().filter(|e| e.is_none()).count();
        assert_eq!(populated, 21);
        assert_eq!(empty, 2);
        assert_eq!(ENDMSG.len(), NUM_QUITMESSAGES + 1);
    }

    #[test]
    fn endmsg_first_entry_is_quitmsg() {
        assert_eq!(ENDMSG[0], Some(QUITMSG));
    }

    #[test]
    fn endmsg_concatenation_bug_preserved() {
        // Index 7: "go ahead and leave. see if i care." + "you want to
        // quit?\n..." glued together with no separator, exactly as the
        // original's missing comma produces.
        let msg = ENDMSG[7].unwrap();
        assert!(msg.contains("go ahead and leave. see if i care.you want to quit?"));

        // Index 13: same bug at the QuitDOOM II / FinalDOOM boundary.
        let msg2 = ENDMSG[13].unwrap();
        assert!(msg2.contains("for thinking about leaving.fuck you, pussy!"));
    }

    #[test]
    fn endmsg_trailing_entries_are_none() {
        assert_eq!(ENDMSG[21], None);
        assert_eq!(ENDMSG[22], None);
    }
}
