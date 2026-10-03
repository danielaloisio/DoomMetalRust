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
//	Main loop menu stuff.
//	Random number LUT.
//	Default Config File.
//	PCX Screenshots.
//
//-----------------------------------------------------------------------------

//! Rust port of `m_bbox.h` / `m_bbox.c`.
//!
//! Bounding box coordinate storage and functions.

use crate::doomtype::{MAXINT, MININT};
use crate::m_fixed::Fixed;

/// (`BOXTOP`, `BOXBOTTOM`, `BOXLEFT`, `BOXRIGHT`) — bbox coordinates.
/// Kept as plain indices (rather than an enum) since the original code
/// uses them purely as array indices into a `fixed_t[4]` box.
pub const BOXTOP: usize = 0;
pub const BOXBOTTOM: usize = 1;
pub const BOXLEFT: usize = 2;
pub const BOXRIGHT: usize = 3;

/// A bounding box, stored the same way as the original's `fixed_t[4]`.
pub type BBox = [Fixed; 4];

/// Port of `M_ClearBox`.
pub fn m_clear_box(box_: &mut BBox) {
    box_[BOXTOP] = MININT;
    box_[BOXRIGHT] = MININT;
    box_[BOXBOTTOM] = MAXINT;
    box_[BOXLEFT] = MAXINT;
}

/// Port of `M_AddToBox`.
pub fn m_add_to_box(box_: &mut BBox, x: Fixed, y: Fixed) {
    if x < box_[BOXLEFT] {
        box_[BOXLEFT] = x;
    } else if x > box_[BOXRIGHT] {
        box_[BOXRIGHT] = x;
    }
    if y < box_[BOXBOTTOM] {
        box_[BOXBOTTOM] = y;
    } else if y > box_[BOXTOP] {
        box_[BOXTOP] = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_box_sets_inverted_extremes() {
        let mut b: BBox = [0; 4];
        m_clear_box(&mut b);
        assert_eq!(b[BOXTOP], MININT);
        assert_eq!(b[BOXRIGHT], MININT);
        assert_eq!(b[BOXBOTTOM], MAXINT);
        assert_eq!(b[BOXLEFT], MAXINT);
    }

    #[test]
    fn add_to_box_grows_bounds() {
        let mut b: BBox = [0; 4];
        m_clear_box(&mut b);
        m_add_to_box(&mut b, 10, 20);
        m_add_to_box(&mut b, -5, 30);
        m_add_to_box(&mut b, 15, -25);

        assert_eq!(b[BOXLEFT], -5);
        assert_eq!(b[BOXRIGHT], 15);
        assert_eq!(b[BOXBOTTOM], -25);
        assert_eq!(b[BOXTOP], 30);
    }
}
