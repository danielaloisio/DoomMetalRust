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
//	Mission begin melt/wipe screen special effect.
//
//-----------------------------------------------------------------------------

//! Rust port of `f_wipe.h` / `f_wipe.c`. Mission start screen wipe/melt,
//! special effects (the original's own description): the transition
//! between two full-screen pictures, either a colour cross-fade
//! ([`WIPE_COLOR_XFORM`]) or the "melt" ([`WIPE_MELT`]) `D_Display`
//! uses.
//!
//! The screens are the original's: the picture being wiped lives in
//! `screens[0]`, the *start* screen is copied into `screens[2]` by
//! [`wipe_start_screen`] and the *end* screen into `screens[3]` by
//! [`wipe_end_screen`].
//!
//! # State
//!
//! `go`, the melt's per-column `y[]` and the column-major copies of the
//! start/end screens are one [`WipeState`] in a `thread_local!`. The
//! original transposes `screens[2]`/`screens[3]` *in place* for the
//! melt (`wipe_shittyColMajorXform`) and works on `short`s — two
//! pixels at a time; the port keeps transposed copies of 2-pixel
//! pairs instead of aliasing the screens.

use std::cell::RefCell;

use crate::m_random::m_random;
use crate::v_video::VVideo;

/// (`wipe_ColorXForm`)
pub const WIPE_COLOR_XFORM: i32 = 0;
/// (`wipe_Melt`)
pub const WIPE_MELT: i32 = 1;

#[derive(Default)]
struct WipeState {
    go: bool,
    /// Melt: the per-column drop offset (`y[]`), `width` entries.
    y: Vec<i32>,
    /// Melt: start/end screens, column-major, as 2-pixel pairs
    /// (`short`s): `[x/2 * height + y]`.
    start_cols: Vec<[u8; 2]>,
    end_cols: Vec<[u8; 2]>,
}

thread_local! {
    static WIPE: RefCell<WipeState> = RefCell::new(WipeState::default());
}

/// Port of `wipe_shittyColMajorXform` on a copy: `array` is
/// `width*height` `short`s in row-major order (`width` counts shorts);
/// returns them column-major.
fn col_major(array: &[u8], width: usize, height: usize) -> Vec<[u8; 2]> {
    let mut dest = vec![[0u8; 2]; width * height];
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 2;
            dest[x * height + y] = [array[i], array[i + 1]];
        }
    }
    dest
}

/// Port of `wipe_initColorXForm`.
fn init_color_xform(v: &mut VVideo, width: i32, height: i32) {
    let n = (width * height) as usize;
    let (s0, rest) = v.screens.split_at_mut(1);
    s0[0][..n].copy_from_slice(&rest[1][..n]); // screens[2] = start
}

/// Port of `wipe_doColorXForm`. Returns true when nothing changed.
fn do_color_xform(v: &mut VVideo, width: i32, height: i32, ticks: i32) -> bool {
    let n = (width * height) as usize;
    let mut changed = false;
    let (s0, rest) = v.screens.split_at_mut(1);
    let end = &rest[2]; // screens[3]
    for (w, &e) in s0[0][..n].iter_mut().zip(&end[..n]) {
        if *w != e {
            if *w > e {
                let newval = *w as i32 - ticks;
                *w = if newval < e as i32 { e } else { newval as u8 };
                changed = true;
            } else {
                let newval = *w as i32 + ticks;
                *w = if newval > e as i32 { e } else { newval as u8 };
                changed = true;
            }
        }
    }
    !changed
}

/// Port of `wipe_initMelt`.
fn init_melt(s: &mut WipeState, v: &mut VVideo, width: i32, height: i32) {
    // copy start screen to main screen
    let n = (width * height) as usize;
    {
        let (s0, rest) = v.screens.split_at_mut(1);
        s0[0][..n].copy_from_slice(&rest[1][..n]);
    }

    // makes this wipe faster (in theory) to have stuff in column-major
    // format
    let half = (width / 2) as usize;
    s.start_cols = col_major(&v.screens[2][..n], half, height as usize);
    s.end_cols = col_major(&v.screens[3][..n], half, height as usize);

    // setup initial column positions (y<0 => not ready to scroll yet)
    s.y = vec![0; width as usize];
    s.y[0] = -(m_random() % 16);
    for i in 1..width as usize {
        let r = (m_random() % 3) - 1;
        s.y[i] = s.y[i - 1] + r;
        if s.y[i] > 0 {
            s.y[i] = 0;
        } else if s.y[i] == -16 {
            s.y[i] = -15;
        }
    }
}

/// Port of `wipe_doMelt`. Returns true when every column is done.
fn do_melt(s: &mut WipeState, v: &mut VVideo, width: i32, height: i32, mut ticks: i32) -> bool {
    let mut done = true;
    let width = (width / 2) as usize;
    let height_u = height as usize;

    while ticks > 0 {
        ticks -= 1;
        for i in 0..width {
            if s.y[i] < 0 {
                s.y[i] += 1;
                done = false;
            } else if s.y[i] < height {
                let mut dy = if s.y[i] < 16 { s.y[i] + 1 } else { 8 };
                if s.y[i] + dy >= height {
                    dy = height - s.y[i];
                }

                // the end screen's next `dy` rows of this column slide in
                let mut src = i * height_u + s.y[i] as usize;
                let mut dst = (s.y[i] as usize) * width + i;
                for _ in 0..dy {
                    let pair = s.end_cols[src];
                    src += 1;
                    v.screens[0][dst * 2..dst * 2 + 2].copy_from_slice(&pair);
                    dst += width;
                }
                s.y[i] += dy;

                // ...and the start screen's column, shifted down
                let mut src = i * height_u;
                let mut dst = (s.y[i] as usize) * width + i;
                for _ in 0..(height - s.y[i]) {
                    let pair = s.start_cols[src];
                    src += 1;
                    v.screens[0][dst * 2..dst * 2 + 2].copy_from_slice(&pair);
                    dst += width;
                }
                done = false;
            }
        }
    }

    done
}

/// Port of `wipe_StartScreen`: remembers the current picture (copies
/// `screens[0]` — `I_ReadScreen` — to `screens[2]`).
pub fn wipe_start_screen(v: &mut VVideo, _x: i32, _y: i32, _width: i32, _height: i32) {
    let n = v.screens[0].len();
    let (s0, rest) = v.screens.split_at_mut(1);
    rest[1][..n].copy_from_slice(&s0[0][..n]);
}

/// Port of `wipe_EndScreen`: remembers the picture to wipe *to*
/// (`screens[0]` -> `screens[3]`), then restores the start picture on
/// screen.
pub fn wipe_end_screen(v: &mut VVideo, x: i32, y: i32, width: i32, height: i32) {
    let n = v.screens[0].len();
    {
        let (s0, rest) = v.screens.split_at_mut(1);
        rest[2][..n].copy_from_slice(&s0[0][..n]);
    }
    let start = v.screens[2].clone();
    v.v_draw_block(x, y, 0, width, height, &start); // restore start scr.
}

/// Port of `wipe_ScreenWipe`. Runs `ticks` tics of wipe number
/// `wipeno` on `screens[0]`; returns true when the wipe is finished.
pub fn wipe_screen_wipe(
    v: &mut VVideo,
    wipeno: i32,
    _x: i32,
    _y: i32,
    width: i32,
    height: i32,
    ticks: i32,
) -> bool {
    WIPE.with(|s| {
        let mut s = s.borrow_mut();

        // initial stuff
        if !s.go {
            s.go = true;
            // wipe_scr = (byte *) Z_Malloc(width*height, PU_STATIC, 0); // DEBUG
            if wipeno == WIPE_COLOR_XFORM {
                init_color_xform(v, width, height);
            } else {
                init_melt(&mut s, v, width, height);
            }
        }

        // do a piece of wipe-in
        v.v_mark_rect(0, 0, width, height);
        let rc = if wipeno == WIPE_COLOR_XFORM {
            do_color_xform(v, width, height, ticks)
        } else {
            do_melt(&mut s, v, width, height, ticks)
        };

        // final stuff
        if rc {
            s.go = false;
            if wipeno == WIPE_MELT {
                s.y.clear(); // wipe_exitMelt: Z_Free(y)
            }
        }

        !s.go
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doomdef::{SCREENHEIGHT, SCREENWIDTH};

    fn picture(v: &mut VVideo, screen: usize, f: impl Fn(usize, usize) -> u8) {
        for y in 0..SCREENHEIGHT as usize {
            for x in 0..SCREENWIDTH as usize {
                v.screens[screen][y * SCREENWIDTH as usize + x] = f(x, y);
            }
        }
    }

    /// Screen 0 shows `a`; start is captured; screen 0 changes to `b`;
    /// end is captured (which puts `a` back on screen).
    fn prepared(a: u8, b: u8) -> VVideo {
        let mut v = VVideo::new();
        picture(&mut v, 0, |_, _| a);
        wipe_start_screen(&mut v, 0, 0, SCREENWIDTH, SCREENHEIGHT);
        picture(&mut v, 0, |_, _| b);
        wipe_end_screen(&mut v, 0, 0, SCREENWIDTH, SCREENHEIGHT);
        v
    }

    #[test]
    fn start_and_end_screens_are_captured_and_the_start_restored() {
        let v = prepared(10, 200);
        assert!(v.screens[2].iter().all(|&p| p == 10));
        assert!(v.screens[3].iter().all(|&p| p == 200));
        assert!(
            v.screens[0].iter().all(|&p| p == 10),
            "the start picture is back on screen"
        );
    }

    #[test]
    fn colour_crossfade_steps_each_pixel_toward_the_end_screen() {
        let mut v = prepared(10, 25);
        assert!(!wipe_screen_wipe(
            &mut v,
            WIPE_COLOR_XFORM,
            0,
            0,
            SCREENWIDTH,
            SCREENHEIGHT,
            4
        ));
        assert!(v.screens[0].iter().all(|&p| p == 14));
        let mut tics = 1;
        while !wipe_screen_wipe(&mut v, WIPE_COLOR_XFORM, 0, 0, SCREENWIDTH, SCREENHEIGHT, 4) {
            tics += 1;
            assert!(tics < 50);
        }
        assert!(
            v.screens[0].iter().all(|&p| p == 25),
            "lands exactly, no overshoot"
        );
        // and a downward fade
        let mut v = prepared(200, 190);
        while !wipe_screen_wipe(&mut v, WIPE_COLOR_XFORM, 0, 0, SCREENWIDTH, SCREENHEIGHT, 7) {}
        assert!(v.screens[0].iter().all(|&p| p == 190));
    }

    #[test]
    fn melt_ends_showing_exactly_the_end_screen() {
        // start = a horizontal gradient, end = a vertical one, so column
        // and row mixups would show.
        let mut v = VVideo::new();
        picture(&mut v, 0, |x, _| (x % 251) as u8);
        wipe_start_screen(&mut v, 0, 0, SCREENWIDTH, SCREENHEIGHT);
        picture(&mut v, 0, |_, y| (y % 251) as u8);
        wipe_end_screen(&mut v, 0, 0, SCREENWIDTH, SCREENHEIGHT);
        let end = v.screens[3].clone();

        let mut tics = 0;
        while !wipe_screen_wipe(&mut v, WIPE_MELT, 0, 0, SCREENWIDTH, SCREENHEIGHT, 1) {
            tics += 1;
            assert!(tics < 400, "the melt should finish in well under 400 tics");
        }
        assert_eq!(v.screens[0], end);
    }

    #[test]
    fn melt_starts_as_the_start_screen_and_drips_column_by_column() {
        let mut v = VVideo::new();
        picture(&mut v, 0, |x, _| (x % 251) as u8);
        wipe_start_screen(&mut v, 0, 0, SCREENWIDTH, SCREENHEIGHT);
        picture(&mut v, 0, |_, _| 77);
        wipe_end_screen(&mut v, 0, 0, SCREENWIDTH, SCREENHEIGHT);
        let start = v.screens[2].clone();

        // first tic: the columns are still (mostly) waiting to fall
        assert!(!wipe_screen_wipe(
            &mut v,
            WIPE_MELT,
            0,
            0,
            SCREENWIDTH,
            SCREENHEIGHT,
            1
        ));
        let same = v.screens[0]
            .iter()
            .zip(&start)
            .filter(|(a, b)| a == b)
            .count();
        assert!(
            same > start.len() * 9 / 10,
            "{same} of {} unchanged",
            start.len()
        );
        // a few tics later some columns have dripped (77s appear) but not all
        for _ in 0..40 {
            wipe_screen_wipe(&mut v, WIPE_MELT, 0, 0, SCREENWIDTH, SCREENHEIGHT, 1);
        }
        let dripped = v.screens[0].iter().filter(|&&p| p == 77).count();
        assert!(dripped > 0 && dripped < v.screens[0].len());
        // finish so the thread-local `go` is reset for other tests
        while !wipe_screen_wipe(&mut v, WIPE_MELT, 0, 0, SCREENWIDTH, SCREENHEIGHT, 1) {}
    }

    #[test]
    fn column_major_transform_is_a_transpose() {
        // 3 columns (shorts) x 2 rows
        let array = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let cols = col_major(&array, 3, 2);
        assert_eq!(cols[0], [1, 2]); // col 0 row 0
        assert_eq!(cols[1], [7, 8]); // col 0 row 1
        assert_eq!(cols[2], [3, 4]); // col 1 row 0
        assert_eq!(cols[5], [11, 12]); // col 2 row 1
    }
}
