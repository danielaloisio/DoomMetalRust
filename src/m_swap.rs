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
//	Endianess handling, swapping 16bit and 32bit.
//
//-----------------------------------------------------------------------------

//! Rust port of `m_swap.h` / `m_swap.c`.
//!
//! Endianness handling, swapping 16-bit and 32-bit values. WAD files are
//! stored little endian.
//!
//! The original CMakeLists.txt passes `-U__BIG_ENDIAN__`, so on every
//! platform this project actually targets, `SHORT(x)`/`LONG(x)` compile
//! down to a plain no-op (`#define SHORT(x) (x)` / `#define LONG(x)
//! (x)`) and `SwapSHORT`/`SwapLONG` are never even compiled in. They are
//! ported here anyway for fidelity/completeness (in case a big-endian
//! target is ever revisited), but `short_le`/`long_le` are the ones
//! ported call sites should actually use, mirroring the active
//! `SHORT(x)`/`LONG(x)` macros from the original build.

/// Port of `SwapSHORT` — swaps MSB and LSB byte of a 16-bit value.
pub fn swap_short(x: u16) -> u16 {
    x.rotate_left(8)
}

/// Port of `SwapLONG` — swaps byte order of a 32-bit value.
pub fn swap_long(x: u32) -> u32 {
    (x >> 24) | ((x >> 8) & 0xff00) | ((x << 8) & 0xff0000) | (x << 24)
}

/// Port of the active `SHORT(x)` macro (`__BIG_ENDIAN__` undefined, as in
/// the original CMake build): identity on little-endian targets.
pub fn short_le(x: i16) -> i16 {
    x
}

/// Port of the active `LONG(x)` macro (`__BIG_ENDIAN__` undefined, as in
/// the original CMake build): identity on little-endian targets.
pub fn long_le(x: i32) -> i32 {
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_short_matches_known_value() {
        assert_eq!(swap_short(0x1234), 0x3412);
    }

    #[test]
    fn swap_long_matches_known_value() {
        assert_eq!(swap_long(0x12345678), 0x78563412);
    }

    #[test]
    fn swap_short_is_involution() {
        assert_eq!(swap_short(swap_short(0xabcd)), 0xabcd);
    }

    #[test]
    fn swap_long_is_involution() {
        assert_eq!(swap_long(swap_long(0xdeadbeef)), 0xdeadbeef);
    }

    #[test]
    fn le_helpers_are_identity() {
        assert_eq!(short_le(0x1234), 0x1234);
        assert_eq!(long_le(0x1234_5678), 0x1234_5678);
    }
}
