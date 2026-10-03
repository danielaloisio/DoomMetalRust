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
//	Fixed point implementation.
//
//-----------------------------------------------------------------------------

//! Rust port of `m_fixed.h` / `m_fixed.c`.
//!
//! Fixed point arithmetic, 32-bit as 16.16. `fixed_t` in the original is
//! a plain `typedef int fixed_t`; ported call sites in later modules are
//! expected to keep using the raw `i32` (`fixed_t`) alias rather than a
//! wrapper type with operator overloading — see the note below.
//!
//! Design note: the porting plan called for a `Fixed(i32)` newtype with
//! operator overloading. On inspecting the original source, `fixed_t` is
//! used pervasively as a bare `int` throughout the ~44k lines of C (in
//! struct fields, array indices, arithmetic mixed with plain ints,
//! bit-shifts, etc.), and only three free functions
//! (`FixedMul`/`FixedDiv`/`FixedDiv2`) actually implement fixed-point
//! semantics — there is no operator overloading in the C original either
//! (C has none). A newtype would need constant `.0`/`From`/`Into`
//! plumbing at every one of those call sites for zero fidelity benefit,
//! which cuts against "port fiel primeiro". So `fixed_t` is kept as a
//! type alias for `i32`, and only the three original functions are
//! ported, matching the original API shape exactly. Revisit this in the
//! idiomatization pass if desired.

use crate::doomtype::{MAXINT, MININT};

/// (`FRACBITS`)
pub const FRACBITS: i32 = 16;
/// (`FRACUNIT`) = `1<<FRACBITS`
pub const FRACUNIT: i32 = 1 << FRACBITS;

/// (`fixed_t`) — 32-bit fixed point value, 16.16.
pub type Fixed = i32;

/// Port of `FixedMul`.
///
/// ```c
/// fixed_t FixedMul(fixed_t a, fixed_t b) {
///     return ((long long) a * (long long) b) >> FRACBITS;
/// }
/// ```
pub fn fixed_mul(a: Fixed, b: Fixed) -> Fixed {
    (((a as i64) * (b as i64)) >> FRACBITS) as Fixed
}

/// Port of `FixedDiv`.
///
/// ```c
/// fixed_t FixedDiv(fixed_t a, fixed_t b) {
///     if ((abs(a) >> 14) >= abs(b))
///         return (a^b) < 0 ? MININT : MAXINT;
///     return FixedDiv2(a, b);
/// }
/// ```
///
/// Note: `abs(MININT)` is itself undefined behavior in C (overflow); the
/// original never hits that path in practice since it only happens for
/// `a == MININT`, an input the engine never actually feeds this
/// function. `i32::wrapping_abs` is used here to keep the same
/// wrap-around behavior C's UB happens to produce on every real target
/// this engine has ever shipped on, without inviting an actual panic.
pub fn fixed_div(a: Fixed, b: Fixed) -> Fixed {
    if (a.wrapping_abs() >> 14) >= b.wrapping_abs() {
        if (a ^ b) < 0 {
            MININT
        } else {
            MAXINT
        }
    } else {
        fixed_div2(a, b)
    }
}

/// Port of `FixedDiv2`.
///
/// ```c
/// fixed_t FixedDiv2(fixed_t a, fixed_t b) {
///     double c;
///     c = ((double)a) / ((double)b) * FRACUNIT;
///     if (c >= 2147483648.0 || c < -2147483648.0)
///         I_Error("FixedDiv: divide by zero");
///     return (fixed_t) c;
/// }
/// ```
///
/// # Panics
/// Panics (standing in for the original's `I_Error`, a fatal abort) if
/// the result would overflow a 32-bit fixed value.
pub fn fixed_div2(a: Fixed, b: Fixed) -> Fixed {
    let c = (a as f64) / (b as f64) * (FRACUNIT as f64);

    if !(-2147483648.0..2147483648.0).contains(&c) {
        panic!("FixedDiv: divide by zero");
    }
    c as Fixed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fracunit_is_65536() {
        assert_eq!(FRACUNIT, 65536);
    }

    #[test]
    fn fixed_mul_one_times_one_is_one() {
        assert_eq!(fixed_mul(FRACUNIT, FRACUNIT), FRACUNIT);
    }

    #[test]
    fn fixed_mul_matches_known_values() {
        // 1.5 * 2.0 = 3.0 in 16.16 fixed point.
        let one_half = FRACUNIT + FRACUNIT / 2;
        let two = 2 * FRACUNIT;
        assert_eq!(fixed_mul(one_half, two), 3 * FRACUNIT);

        // Negative operand.
        assert_eq!(fixed_mul(-FRACUNIT, FRACUNIT), -FRACUNIT);
    }

    #[test]
    fn fixed_div_one_by_one_is_one() {
        assert_eq!(fixed_div(FRACUNIT, FRACUNIT), FRACUNIT);
    }

    #[test]
    fn fixed_div_matches_known_values() {
        // 3.0 / 2.0 = 1.5
        assert_eq!(
            fixed_div(3 * FRACUNIT, 2 * FRACUNIT),
            FRACUNIT + FRACUNIT / 2
        );
    }

    #[test]
    fn fixed_div_saturates_on_overflow() {
        // a huge numerator over a tiny denominator saturates to MAXINT/MININT
        // via the fast-path check, exactly like the C original.
        assert_eq!(fixed_div(MAXINT, 1), MAXINT);
        assert_eq!(fixed_div(MININT + 1, 1), MININT);
        assert_eq!(fixed_div(MAXINT, -1), MININT);
    }

    #[test]
    #[should_panic(expected = "FixedDiv: divide by zero")]
    fn fixed_div2_panics_like_i_error_on_overflow() {
        // Bypass FixedDiv's saturation fast path by calling FixedDiv2
        // directly, exactly as the original test would need to.
        fixed_div2(MAXINT, 1);
    }
}
