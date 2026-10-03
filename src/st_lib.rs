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
//	The status bar widget code.
//
//-----------------------------------------------------------------------------

//! Rust port of `st_lib.h` / `st_lib.c`. The status bar widget code: a
//! number ([`StNumber`]), a number with a `%` ([`StPercent`]), an icon
//! chosen from a list ([`StMultIcon`]) and an on/off icon
//! ([`StBinIcon`]).
//!
//! # Divergences
//!
//! The original widgets hold pointers into live game state
//! (`int* num`, `boolean* on`, `boolean* val`, `int* inum`) that
//! `st_stuff` points at the player's fields and its own flags once.
//! Those pointers aren't stored here; every `update`/`draw` takes the
//! current value(s) as arguments, which is exactly what dereferencing
//! them at call time yields. The `patch_t** pl` lists are
//! `Rc<[PatchData]>`. `STlib_init`'s global `sttminus` lives in
//! [`StLib`], which the number-drawing functions are methods of.

use std::rc::Rc;

use crate::doomdef::{SCREENHEIGHT, SCREENWIDTH};
use crate::v_video::{
    patch_height, patch_leftoffset, patch_topoffset, patch_width, PatchData, VVideo,
};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// Background screen (`BG`): the pristine status bar copy.
pub const BG: usize = 4;
/// Foreground screen (`FG`).
pub const FG: usize = 0;

/// (`ST_HEIGHT`, `32*SCREEN_MUL` with `SCREEN_MUL` = 1)
pub const ST_HEIGHT: i32 = 32;
/// (`ST_WIDTH`)
pub const ST_WIDTH: i32 = SCREENWIDTH;
/// (`ST_Y`)
pub const ST_Y: i32 = SCREENHEIGHT - ST_HEIGHT;

/// A list of patches (`patch_t**`).
pub type PatchList = Rc<[PatchData]>;

/// (`st_number_t`) Number widget.
#[derive(Clone)]
pub struct StNumber {
    /// Upper right-hand corner of the number (right-justified).
    pub x: i32,
    pub y: i32,
    /// Max # of digits in number.
    pub width: i32,
    /// Last number value.
    pub oldnum: i32,
    /// List of patches for 0-9.
    pub p: PatchList,
}

/// (`st_percent_t`) Percentage widget.
#[derive(Clone)]
pub struct StPercent {
    /// Number information.
    pub n: StNumber,
    /// Percent sign graphic.
    pub p: PatchData,
}

/// (`st_multicon_t`) Multiple icon widget.
#[derive(Clone)]
pub struct StMultIcon {
    /// Center-justified location of icons.
    pub x: i32,
    pub y: i32,
    /// Last icon number.
    pub oldinum: i32,
    /// List of icons.
    pub p: PatchList,
}

/// (`st_binicon_t`) Binary icon widget.
#[derive(Clone)]
pub struct StBinIcon {
    /// Center-justified location of icon.
    pub x: i32,
    pub y: i32,
    /// Last icon value.
    pub oldval: bool,
    /// Icon.
    pub p: PatchData,
}

/// The library's own data: the minus sign graphic (`sttminus`).
pub struct StLib {
    pub sttminus: PatchData,
}

impl StNumber {
    /// Port of `STlib_initNum`.
    pub fn new(x: i32, y: i32, pl: PatchList, width: i32) -> Self {
        StNumber {
            x,
            y,
            oldnum: 0,
            width,
            p: pl,
        }
    }
}

impl StPercent {
    /// Port of `STlib_initPercent`.
    pub fn new(x: i32, y: i32, pl: PatchList, percent: PatchData) -> Self {
        StPercent {
            n: StNumber::new(x, y, pl, 3),
            p: percent,
        }
    }
}

impl StMultIcon {
    /// Port of `STlib_initMultIcon`.
    pub fn new(x: i32, y: i32, il: PatchList) -> Self {
        StMultIcon {
            x,
            y,
            oldinum: -1,
            p: il,
        }
    }
}

impl StBinIcon {
    /// Port of `STlib_initBinIcon`.
    pub fn new(x: i32, y: i32, i: PatchData) -> Self {
        StBinIcon {
            x,
            y,
            oldval: false,
            p: i,
        }
    }
}

impl StLib {
    /// Port of `STlib_init`.
    pub fn init(wad: &mut WadFiles) -> Self {
        StLib {
            sttminus: Rc::from(wad.cache_lump_name("STTMINUS", PurgeTag::Static)),
        }
    }

    /// Port of `STlib_drawNum`. A fairly efficient way to draw a number
    /// based on differences from the old number (the original's own
    /// comment, preserved). `num` is `*n->num`.
    ///
    /// # Panics
    /// If `n.y < ST_Y` (`I_Error`).
    pub fn draw_num(&self, n: &mut StNumber, _refresh: bool, num: i32, v: &mut VVideo) {
        let mut numdigits = n.width;
        let mut num = num;
        let w = patch_width(&n.p[0]);
        let h = patch_height(&n.p[0]);

        n.oldnum = num;

        let neg = num < 0;
        if neg {
            if numdigits == 2 && num < -9 {
                num = -9;
            } else if numdigits == 3 && num < -99 {
                num = -99;
            }
            num = -num;
        }

        // clear the area
        let mut x = n.x - numdigits * w;

        if n.y - ST_Y < 0 {
            panic!("drawNum: n->y - ST_Y < 0");
        }

        v.v_copy_rect(x, n.y - ST_Y, BG, w * numdigits, h, x, n.y, FG);

        // if non-number, do not draw it
        if num == 1994 {
            return;
        }

        x = n.x;

        // in the special case of 0, you draw 0
        if num == 0 {
            v.v_draw_patch(x - w, n.y, FG, &n.p[0]);
        }

        // draw the new number
        while num != 0 && numdigits != 0 {
            numdigits -= 1;
            x -= w;
            v.v_draw_patch(x, n.y, FG, &n.p[(num % 10) as usize]);
            num /= 10;
        }

        // draw a minus sign if necessary
        if neg {
            v.v_draw_patch(x - 8, n.y, FG, &self.sttminus);
        }
    }

    /// Port of `STlib_updateNum`. `on` is `*n->on`.
    pub fn update_num(&self, n: &mut StNumber, refresh: bool, num: i32, on: bool, v: &mut VVideo) {
        if on {
            self.draw_num(n, refresh, num, v);
        }
    }

    /// Port of `STlib_updatePercent`. `num`/`on` are `*per->n.num`/
    /// `*per->n.on`.
    pub fn update_percent(
        &self,
        per: &mut StPercent,
        refresh: bool,
        num: i32,
        on: bool,
        v: &mut VVideo,
    ) {
        if refresh && on {
            v.v_draw_patch(per.n.x, per.n.y, FG, &per.p);
        }
        self.update_num(&mut per.n, refresh, num, on, v);
    }
}

/// Port of `STlib_updateMultIcon`. `inum`/`on` are `*mi->inum`/
/// `*mi->on`.
///
/// # Panics
/// If the icon would start above the status bar (`I_Error`).
pub fn st_updatemulticon(mi: &mut StMultIcon, refresh: bool, inum: i32, on: bool, v: &mut VVideo) {
    if on && (mi.oldinum != inum || refresh) && inum != -1 {
        if mi.oldinum != -1 {
            let old = &mi.p[mi.oldinum as usize];
            let x = mi.x - patch_leftoffset(old);
            let y = mi.y - patch_topoffset(old);
            let w = patch_width(old);
            let h = patch_height(old);

            if y - ST_Y < 0 {
                panic!("updateMultIcon: y - ST_Y < 0");
            }

            v.v_copy_rect(x, y - ST_Y, BG, w, h, x, y, FG);
        }
        v.v_draw_patch(mi.x, mi.y, FG, &mi.p[inum as usize]);
        mi.oldinum = inum;
    }
}

/// Port of `STlib_updateBinIcon`. `val`/`on` are `*bi->val`/`*bi->on`.
///
/// # Panics
/// If the icon would start above the status bar (`I_Error`).
pub fn st_updatebinicon(bi: &mut StBinIcon, refresh: bool, val: bool, on: bool, v: &mut VVideo) {
    if on && (bi.oldval != val || refresh) {
        let x = bi.x - patch_leftoffset(&bi.p);
        let y = bi.y - patch_topoffset(&bi.p);
        let w = patch_width(&bi.p);
        let h = patch_height(&bi.p);

        if y - ST_Y < 0 {
            panic!("updateBinIcon: y - ST_Y < 0");
        }

        if val {
            v.v_draw_patch(bi.x, bi.y, FG, &bi.p);
        } else {
            v.v_copy_rect(x, y - ST_Y, BG, w, h, x, y, FG);
        }

        bi.oldval = val;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Assets {
        lib: StLib,
        digits: PatchList,
        keys: PatchList,
        percent: PatchData,
    }

    fn load() -> Option<Assets> {
        let path = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut wad = WadFiles::new();
        wad.init_file(path);
        let mut get =
            |name: &str| -> PatchData { Rc::from(wad.cache_lump_name(name, PurgeTag::Cache)) };
        let digits: Vec<PatchData> = (0..10).map(|i| get(&format!("STTNUM{i}"))).collect();
        let keys: Vec<PatchData> = (0..3).map(|i| get(&format!("STKEYS{i}"))).collect();
        let percent = get("STTPRCNT");
        Some(Assets {
            lib: StLib::init(&mut wad),
            digits: digits.into(),
            keys: keys.into(),
            percent,
        })
    }

    /// FG cleared, BG (the pristine status bar) a recognisable pattern.
    /// `screens[4]` is only status-bar sized: the original allocates it
    /// in `ST_Init` (`Z_Malloc(ST_WIDTH*ST_HEIGHT)`), rows relative to
    /// `ST_Y`.
    fn video() -> VVideo {
        let mut v = VVideo::new();
        v.screens[BG] = vec![0; (ST_WIDTH * ST_HEIGHT) as usize];
        v.screens[FG].fill(0);
        for (i, b) in v.screens[BG].iter_mut().enumerate() {
            *b = 100 + (i % 7) as u8;
        }
        v
    }

    /// What `STlib_drawNum` should leave for `digits` (most significant
    /// first) right-aligned at `x`, computed with `V_CopyRect`/
    /// `V_DrawPatch` directly.
    fn reference_number(a: &Assets, x: i32, y: i32, width: i32, digits: &[usize]) -> VVideo {
        let mut v = video();
        let w = patch_width(&a.digits[0]);
        let h = patch_height(&a.digits[0]);
        v.v_copy_rect(
            x - width * w,
            y - ST_Y,
            BG,
            w * width,
            h,
            x - width * w,
            y,
            FG,
        );
        let mut px = x;
        for &d in digits.iter().rev() {
            px -= w;
            v.v_draw_patch(px, y, FG, &a.digits[d]);
        }
        v
    }

    #[test]
    fn draw_num_clears_the_area_from_the_background_then_draws_digits() {
        let Some(a) = load() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut v = video();
        let mut n = StNumber::new(100, ST_Y + 3, a.digits.clone(), 3);
        a.lib.draw_num(&mut n, true, 123, &mut v);
        let want = reference_number(&a, 100, ST_Y + 3, 3, &[1, 2, 3]);
        assert_eq!(v.screens[FG], want.screens[FG]);
        assert_eq!(n.oldnum, 123);
    }

    #[test]
    fn draw_num_zero_1994_and_negative() {
        let Some(a) = load() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut n = StNumber::new(100, ST_Y + 3, a.digits.clone(), 3);

        let mut v = video();
        a.lib.draw_num(&mut n, true, 0, &mut v);
        assert_eq!(
            v.screens[FG],
            reference_number(&a, 100, ST_Y + 3, 3, &[0]).screens[FG]
        );

        // 1994: only the background is restored ("don't draw a number").
        let mut v = video();
        a.lib.draw_num(&mut n, true, 1994, &mut v);
        assert_eq!(
            v.screens[FG],
            reference_number(&a, 100, ST_Y + 3, 3, &[]).screens[FG]
        );

        // -5 in a 2-digit field: '5' then a minus 8 px left of it.
        let mut v = video();
        let mut n2 = StNumber::new(100, ST_Y + 3, a.digits.clone(), 2);
        a.lib.draw_num(&mut n2, true, -5, &mut v);
        let mut want = reference_number(&a, 100, ST_Y + 3, 2, &[5]);
        let w = patch_width(&a.digits[0]);
        want.v_draw_patch(100 - w - 8, ST_Y + 3, FG, &a.lib.sttminus);
        assert_eq!(v.screens[FG], want.screens[FG]);

        // -99 clamps in a 2-digit field: -100 shows as -9
        let mut v = video();
        a.lib.draw_num(&mut n2, true, -100, &mut v);
        let mut want = reference_number(&a, 100, ST_Y + 3, 2, &[9]);
        want.v_draw_patch(100 - w - 8, ST_Y + 3, FG, &a.lib.sttminus);
        assert_eq!(v.screens[FG], want.screens[FG]);
    }

    #[test]
    fn draw_num_above_the_status_bar_panics() {
        let Some(a) = load() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut n = StNumber::new(100, ST_Y - 1, a.digits.clone(), 3);
        let mut v = video();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            a.lib.draw_num(&mut n, true, 5, &mut v)
        }));
        assert!(r.is_err());
    }

    #[test]
    fn update_num_and_percent_respect_the_on_flag_and_refresh() {
        let Some(a) = load() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut per = StPercent::new(100, ST_Y + 3, a.digits.clone(), a.percent.clone());
        let mut v = video();
        a.lib.update_percent(&mut per, true, 50, false, &mut v);
        assert!(v.screens[FG].iter().all(|&b| b == 0), "off: nothing drawn");

        a.lib.update_percent(&mut per, true, 50, true, &mut v);
        // refresh draws the % sign first, then the number over it
        let mut ordered = video();
        ordered.v_draw_patch(100, ST_Y + 3, FG, &a.percent);
        let w = patch_width(&a.digits[0]);
        let h = patch_height(&a.digits[0]);
        ordered.v_copy_rect(
            100 - 3 * w,
            ST_Y + 3 - ST_Y,
            BG,
            3 * w,
            h,
            100 - 3 * w,
            ST_Y + 3,
            FG,
        );
        let mut px = 100;
        for &d in [5usize, 0].iter().rev() {
            px -= w;
            ordered.v_draw_patch(px, ST_Y + 3, FG, &a.digits[d]);
        }
        assert_eq!(v.screens[FG], ordered.screens[FG]);

        // no refresh: the % sign is not redrawn, the number still is
        let mut v2 = video();
        a.lib.update_percent(&mut per, false, 50, true, &mut v2);
        assert_eq!(
            v2.screens[FG],
            reference_number(&a, 100, ST_Y + 3, 3, &[5, 0]).screens[FG]
        );
    }

    #[test]
    fn mult_icon_draws_on_change_and_restores_the_old_icon_area() {
        let Some(a) = load() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut mi = StMultIcon::new(200, ST_Y + 5, a.keys.clone());
        let mut v = video();

        st_updatemulticon(&mut mi, false, 0, false, &mut v);
        assert!(v.screens[FG].iter().all(|&b| b == 0), "off");
        st_updatemulticon(&mut mi, false, -1, true, &mut v);
        assert!(v.screens[FG].iter().all(|&b| b == 0), "-1 = none");

        st_updatemulticon(&mut mi, false, 1, true, &mut v);
        assert_eq!(mi.oldinum, 1);
        let mut want = video();
        want.v_draw_patch(200, ST_Y + 5, FG, &a.keys[1]);
        assert_eq!(v.screens[FG], want.screens[FG]);

        // same value, no refresh: not redrawn (scribble survives)
        v.screens[FG][0] = 55;
        st_updatemulticon(&mut mi, false, 1, true, &mut v);
        assert_eq!(v.screens[FG][0], 55);

        // changing: the old icon's area is restored from BG, then the
        // new icon drawn
        st_updatemulticon(&mut mi, false, 2, true, &mut v);
        assert_eq!(mi.oldinum, 2);
        let old = &a.keys[1];
        let (x, y) = (200 - patch_leftoffset(old), ST_Y + 5 - patch_topoffset(old));
        let mut want = video();
        want.v_copy_rect(
            x,
            y - ST_Y,
            BG,
            patch_width(old),
            patch_height(old),
            x,
            y,
            FG,
        );
        want.v_draw_patch(200, ST_Y + 5, FG, &a.keys[2]);
        want.screens[FG][0] = 55; // untouched scribble
        assert_eq!(v.screens[FG], want.screens[FG]);
    }

    #[test]
    fn bin_icon_draws_when_true_and_restores_when_false() {
        let Some(a) = load() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut bi = StBinIcon::new(200, ST_Y + 5, a.keys[0].clone());
        let mut v = video();

        st_updatebinicon(&mut bi, false, false, true, &mut v);
        assert!(!bi.oldval);
        assert!(
            v.screens[FG].iter().all(|&b| b == 0),
            "unchanged false: nothing to do"
        );

        st_updatebinicon(&mut bi, false, true, true, &mut v);
        let mut want = video();
        want.v_draw_patch(200, ST_Y + 5, FG, &a.keys[0]);
        assert_eq!(v.screens[FG], want.screens[FG]);

        st_updatebinicon(&mut bi, false, false, true, &mut v);
        let p = &a.keys[0];
        let (x, y) = (200 - patch_leftoffset(p), ST_Y + 5 - patch_topoffset(p));
        let mut want = video();
        want.v_copy_rect(x, y - ST_Y, BG, patch_width(p), patch_height(p), x, y, FG);
        assert_eq!(v.screens[FG], want.screens[FG]);
    }
}
