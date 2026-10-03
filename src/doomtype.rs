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
//	Simple basic typedefs, isolated here to make it easier
//	 separating modules.
//
//-----------------------------------------------------------------------------

//! Rust port of `doomtype.h`.
//!
//! Simple basic typedefs, isolated here to make it easier separating
//! modules, exactly as in the original.

/// The original `byte` typedef (`unsigned char`).
pub type Byte = u8;

/// The original `boolean` typedef. The C source uses an `enum { False,
/// True }` fallback when not compiled as C++; Rust's native `bool` is the
/// faithful equivalent, so callers should just use `bool` directly. This
/// alias exists only to keep ported call sites readable when they mirror
/// the original `boolean` spelling.
pub type Boolean = bool;

/// Max positive 8-bit signed value (`MAXCHAR`).
pub const MAXCHAR: i8 = 0x7f;
/// Max positive 16-bit signed value (`MAXSHORT`).
pub const MAXSHORT: i16 = 0x7fff;
/// Max positive 32-bit signed value (`MAXINT`).
pub const MAXINT: i32 = 0x7fffffffu32 as i32;
/// Max positive 32-bit signed value, long variant (`MAXLONG`).
pub const MAXLONG: i32 = 0x7fffffffu32 as i32;

/// Min negative 8-bit signed value (`MINCHAR`).
pub const MINCHAR: i8 = -0x80;
/// Min negative 16-bit signed value (`MINSHORT`).
pub const MINSHORT: i16 = -0x8000;
/// Min negative 32-bit signed value (`MININT`). The C source spells this
/// as the bit pattern `(int)0x80000000`, i.e. `i32::MIN`.
pub const MININT: i32 = 0x80000000u32 as i32;
/// Min negative 32-bit signed value, long variant (`MINLONG`).
pub const MINLONG: i32 = 0x80000000u32 as i32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extremes_match_c_bit_patterns() {
        assert_eq!(MAXINT, i32::MAX);
        assert_eq!(MININT, i32::MIN);
        assert_eq!(MAXSHORT, i16::MAX);
        assert_eq!(MINSHORT, i16::MIN);
        assert_eq!(MAXCHAR, i8::MAX);
        assert_eq!(MINCHAR, i8::MIN);
    }
}
