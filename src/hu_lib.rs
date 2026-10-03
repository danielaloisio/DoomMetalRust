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
// DESCRIPTION:  heads-up text and input code
//
//-----------------------------------------------------------------------------

//! Rust port of `hu_lib.h` / `hu_lib.c`. Heads-up text widgets: a line of
//! text ([`HuTextLine`]), a scrolling list of lines ([`HuSText`], the
//! message display) and an input line ([`HuIText`], chat).
//!
//! # Divergences
//!
//! * `patch_t** f` (the font) is a [`Font`]: a shared slice of
//!   [`PatchData`], indexed by `c - startchar` like the original.
//! * `boolean* on` — a pointer to a flag that `hu_stuff` owns and flips
//!   — is not stored. The functions that read it (`draw`/`erase` of
//!   [`HuSText`]/[`HuIText`]) take the current value as an argument,
//!   which is what the original's dereference at call time amounts to.
//! * `HUlib_init` is empty in the original and isn't ported.
//! * The original's `static boolean lastautomapactive` in
//!   `HUlib_eraseTextLine` is written and never read; not ported.
//! * The view window (`viewwindowx`/`viewwindowy`/`viewwidth`/
//!   `viewheight`, globals in the original) is passed as a
//!   [`ViewWin`], and `R_VideoErase` goes through the caller's
//!   [`RDraw`].

use std::rc::Rc;

use crate::doomdef::{KEY_BACKSPACE, KEY_ENTER, SCREENWIDTH};
use crate::doomstat;
use crate::r_draw::RDraw;
use crate::v_video::{patch_height, patch_width, PatchData, VVideo};

/// (`FG`) the visible screen.
pub const FG: usize = 0;
/// (`BG`) the backup screen `hu_lib` erases from.
pub const BG: usize = 1;
/// (`HU_CHARERASE`)
pub const HU_CHARERASE: i32 = KEY_BACKSPACE;
/// (`HU_MAXLINES`)
pub const HU_MAXLINES: usize = 4;
/// (`HU_MAXLINELENGTH`)
pub const HU_MAXLINELENGTH: usize = 80;

/// A font: one patch per character from the widget's start character.
pub type Font = Rc<[PatchData]>;

/// The 3D view's window, as `HUlib_eraseTextLine` needs it.
#[derive(Debug, Clone, Copy, Default)]
pub struct ViewWin {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// (`hu_textline_t`) A single line of text.
#[derive(Clone)]
pub struct HuTextLine {
    /// Left-justified position of scrolling text window.
    pub x: i32,
    pub y: i32,
    /// Font.
    pub f: Font,
    /// Start character.
    pub sc: i32,
    /// Line of text (`l[]`/`len`: its length is the current line
    /// length).
    pub l: Vec<u8>,
    /// Whether this line needs to be erased/redrawn.
    pub needsupdate: i32,
}

impl HuTextLine {
    /// Port of `HUlib_initTextLine`.
    pub fn new(x: i32, y: i32, f: Font, sc: i32) -> Self {
        HuTextLine {
            x,
            y,
            f,
            sc,
            l: Vec::new(),
            needsupdate: 0,
        }
    }

    /// Port of `HUlib_clearTextLine`.
    pub fn clear(&mut self) {
        self.l.clear();
        self.needsupdate = 1;
    }

    /// Port of `HUlib_addCharToTextLine`. Returns false when full.
    pub fn add_char(&mut self, ch: u8) -> bool {
        if self.l.len() == HU_MAXLINELENGTH {
            false
        } else {
            self.l.push(ch);
            self.needsupdate = 4;
            true
        }
    }

    /// Port of `HUlib_delCharFromTextLine`. Returns false when empty.
    pub fn del_char(&mut self) -> bool {
        if self.l.pop().is_none() {
            false
        } else {
            self.needsupdate = 4;
            true
        }
    }

    /// Port of `HUlib_drawTextLine`. Draws the line (and, optionally, a
    /// cursor) onto the visible screen.
    pub fn draw(&self, drawcursor: bool, v: &mut VVideo) {
        let mut x = self.x;

        // draw the new stuff
        for &ch in &self.l {
            let c = ch.to_ascii_uppercase();
            if c != b' ' && c as i32 >= self.sc && c <= b'_' {
                let patch = &self.f[(c as i32 - self.sc) as usize];
                let w = patch_width(patch);
                if x + w > SCREENWIDTH {
                    break;
                }
                v.v_draw_patch_direct(x, self.y, FG, patch);
                x += w;
            } else {
                x += 4;
                if x >= SCREENWIDTH {
                    break;
                }
            }
        }

        // draw the cursor if requested
        if drawcursor {
            let cursor = &self.f[(b'_' as i32 - self.sc) as usize];
            if x + patch_width(cursor) <= SCREENWIDTH {
                v.v_draw_patch_direct(x, self.y, FG, cursor);
            }
        }
    }

    /// Port of `HUlib_eraseTextLine`. Sorta called by HU_Erase and just
    /// better darn get things straight (the original's own comment):
    /// erases the line's rows from the backup screen, only outside the
    /// view when the view doesn't fill the screen.
    pub fn erase(&mut self, view: &ViewWin, rdraw: &RDraw, v: &mut VVideo) {
        // Only erases when NOT in automap and the screen is reduced,
        // and the text must either need updating or be "in" the
        // (original's own comment).
        if !doomstat::state().automapactive && rdraw.viewwindowx != 0 && self.needsupdate != 0 {
            let lh = patch_height(&self.f[0]) + 1;
            let sw = SCREENWIDTH as usize;
            for y in self.y..self.y + lh {
                let yoffset = y as usize * sw;
                if y < view.y || y >= view.y + view.height {
                    rdraw.r_video_erase(v, yoffset, sw); // erase entire line
                } else {
                    rdraw.r_video_erase(v, yoffset, view.x as usize); // erase left border
                    rdraw.r_video_erase(
                        v,
                        yoffset + (view.x + view.width) as usize,
                        view.x as usize,
                    );
                    // erase right border
                }
            }
        }

        if self.needsupdate != 0 {
            self.needsupdate -= 1;
        }
    }
}

/// (`hu_stext_t`) Scrolling text: a queue of [`HuTextLine`]s.
#[derive(Clone)]
pub struct HuSText {
    /// Text lines to draw.
    pub l: Vec<HuTextLine>,
    /// Height in lines (`l.len()`).
    pub h: usize,
    /// Current line number.
    pub cl: usize,
    /// Last value of the `on` flag.
    pub laston: bool,
}

impl HuSText {
    /// Port of `HUlib_initSText`. Lines stack upward from `y`.
    pub fn new(x: i32, y: i32, h: usize, font: Font, startchar: i32) -> Self {
        assert!(h <= HU_MAXLINES);
        let lh = patch_height(&font[0]) + 1;
        HuSText {
            l: (0..h)
                .map(|i| HuTextLine::new(x, y - i as i32 * lh, font.clone(), startchar))
                .collect(),
            h,
            cl: 0,
            laston: true,
        }
    }

    /// Port of `HUlib_addLineToSText`. Add a new line.
    pub fn add_line(&mut self) {
        // add a clear line
        self.cl += 1;
        if self.cl == self.h {
            self.cl = 0;
        }
        self.l[self.cl].clear();

        // everything needs updating
        for line in &mut self.l {
            line.needsupdate = 4;
        }
    }

    /// Port of `HUlib_addMessageToSText`.
    pub fn add_message(&mut self, prefix: Option<&str>, msg: &str) {
        self.add_line();
        let cl = self.cl;
        if let Some(prefix) = prefix {
            for b in prefix.bytes() {
                self.l[cl].add_char(b);
            }
        }
        for b in msg.bytes() {
            self.l[cl].add_char(b);
        }
    }

    /// Port of `HUlib_drawSText`. `on` is the widget's on-flag.
    pub fn draw(&self, on: bool, v: &mut VVideo) {
        if !on {
            return; // if not on, don't draw
        }

        // draw everything
        for i in 0..self.h {
            let idx = if i > self.cl {
                self.cl + self.h - i // handle queue of lines
            } else {
                self.cl - i
            };
            self.l[idx].draw(false, v); // no cursor, please
        }
    }

    /// Port of `HUlib_eraseSText`.
    pub fn erase(&mut self, on: bool, view: &ViewWin, rdraw: &RDraw, v: &mut VVideo) {
        for line in &mut self.l {
            if self.laston && !on {
                line.needsupdate = 4;
            }
            line.erase(view, rdraw, v);
        }
        self.laston = on;
    }
}

/// (`hu_itext_t`) Input text: a line with a protected prefix.
#[derive(Clone)]
pub struct HuIText {
    /// Text line to input on.
    pub l: HuTextLine,
    /// Left margin past which the player is not allowed to backspace.
    pub lm: usize,
    /// Last value of the `on` flag.
    pub laston: bool,
}

impl HuIText {
    /// Port of `HUlib_initIText`.
    pub fn new(x: i32, y: i32, font: Font, startchar: i32) -> Self {
        HuIText {
            lm: 0, // default left margin is start of text
            laston: true,
            l: HuTextLine::new(x, y, font, startchar),
        }
    }

    /// Port of `HUlib_delCharFromIText`. Whether the char can be
    /// deleted depends on the left margin.
    pub fn del_char(&mut self) {
        if self.l.l.len() != self.lm {
            self.l.del_char();
        }
    }

    /// Port of `HUlib_eraseLineFromIText`.
    pub fn erase_line(&mut self) {
        while self.lm != self.l.l.len() {
            self.l.del_char();
        }
    }

    /// Port of `HUlib_resetIText`. Resets left margin as well.
    pub fn reset(&mut self) {
        self.lm = 0;
        self.l.clear();
    }

    /// Port of `HUlib_addPrefixToIText`. Adds `str` and moves the left
    /// margin past it.
    pub fn add_prefix(&mut self, str: &str) {
        for b in str.bytes() {
            self.l.add_char(b);
        }
        self.lm = self.l.l.len();
    }

    /// Port of `HUlib_keyInIText`. Wrapper function for handling
    /// general keyed input; returns true if it ate the key.
    pub fn key_in(&mut self, ch: u8) -> bool {
        if (b' '..=b'_').contains(&ch) {
            self.l.add_char(ch);
        } else if ch as i32 == KEY_BACKSPACE {
            self.del_char();
        } else if ch as i32 != KEY_ENTER {
            return false; // did not eat key
        }
        true // ate the key
    }

    /// Port of `HUlib_drawIText`. `on` is the widget's on-flag.
    pub fn draw(&self, on: bool, v: &mut VVideo) {
        if !on {
            return;
        }
        self.l.draw(true, v); // draw the line w/ cursor
    }

    /// Port of `HUlib_eraseIText`.
    pub fn erase(&mut self, on: bool, view: &ViewWin, rdraw: &RDraw, v: &mut VVideo) {
        if self.laston && !on {
            self.l.needsupdate = 4;
        }
        self.l.erase(view, rdraw, v);
        self.laston = on;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v_video::patch_width;
    use crate::w_wad::WadFiles;
    use crate::z_zone::PurgeTag;

    /// The status-bar-less HUD font ('!' .. '_', `STCFN033`..`STCFN095`)
    /// from the real WAD.
    fn real_font() -> Option<Font> {
        let path = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut wad = WadFiles::new();
        wad.init_file(path);
        let font: Vec<PatchData> = (b'!'..=b'_')
            .map(|c| {
                let name = format!("STCFN{:03}", c as i32 - b'!' as i32 + 33);
                Rc::from(wad.cache_lump_name(&name, PurgeTag::Cache))
            })
            .collect();
        Some(font.into())
    }

    #[test]
    fn text_line_add_delete_and_full() {
        let Some(font) = real_font() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut t = HuTextLine::new(10, 20, font, b'!' as i32);
        assert!(!t.del_char());
        for _ in 0..HU_MAXLINELENGTH {
            assert!(t.add_char(b'A'));
        }
        assert!(!t.add_char(b'B'), "line is full at 80");
        assert!(t.del_char());
        assert_eq!(t.l.len(), 79);
        assert_eq!(t.needsupdate, 4);
        t.clear();
        assert!(t.l.is_empty());
        assert_eq!(t.needsupdate, 1);
    }

    #[test]
    fn draw_text_line_advances_by_glyph_width_and_4_for_spaces() {
        let Some(font) = real_font() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut v = VVideo::new();
        let mut t = HuTextLine::new(10, 20, font.clone(), b'!' as i32);
        for b in b"hi Z" {
            t.add_char(*b); // lowercase is upper-cased by the draw
        }
        t.draw(true, &mut v);

        // Where the cursor ('_') ended up: 10 + H + I + 4 + Z.
        let adv = |c: u8| patch_width(&font[(c - b'!') as usize]);
        let end = 10 + adv(b'H') + adv(b'I') + 4 + adv(b'Z');
        let cursor = &font[(b'_' - b'!') as usize];
        let mut probe = VVideo::new();
        probe.v_draw_patch_direct(end, 20, FG, cursor);
        let row = 20 - crate::v_video::patch_topoffset(cursor);
        let base = (row * SCREENWIDTH) as usize;
        let cursor_pixels: Vec<usize> = (0..SCREENWIDTH as usize * 12)
            .filter(|&i| probe.screens[0][base + i] != 0)
            .collect();
        assert!(!cursor_pixels.is_empty());
        for i in cursor_pixels {
            assert_eq!(v.screens[0][base + i], probe.screens[0][base + i]);
        }
    }

    #[test]
    fn draw_text_line_stops_at_the_screen_edge() {
        let Some(font) = real_font() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut v = VVideo::new();
        let mut t = HuTextLine::new(SCREENWIDTH - 3, 20, font, b'!' as i32);
        t.add_char(b'W');
        t.draw(false, &mut v); // must not panic or wrap
        assert!(
            v.screens[0].iter().all(|&b| b == 0),
            "W doesn't fit: not drawn"
        );
    }

    #[test]
    fn stext_queues_lines_and_draws_newest_first() {
        let Some(font) = real_font() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut s = HuSText::new(0, 100, 3, font.clone(), b'!' as i32);
        // lines stack upward: each one a font-height+1 above the last
        let lh = crate::v_video::patch_height(&font[0]) + 1;
        assert_eq!(s.l[0].y, 100);
        assert_eq!(s.l[1].y, 100 - lh);
        assert_eq!(s.l[2].y, 100 - 2 * lh);

        s.add_message(Some("P: "), "one");
        assert_eq!(s.cl, 1);
        assert_eq!(s.l[1].l, b"P: one");
        assert!(s.l.iter().all(|l| l.needsupdate == 4));
        s.add_message(None, "two");
        s.add_message(None, "three");
        // wrapped around to line 0
        assert_eq!(s.cl, 0);
        assert_eq!(s.l[0].l, b"three");
        assert_eq!(s.l[1].l, b"P: one");
        assert_eq!(s.l[2].l, b"two");

        // not on: nothing drawn
        let mut v = VVideo::new();
        s.draw(false, &mut v);
        assert!(v.screens[0].iter().all(|&b| b == 0));
        s.draw(true, &mut v);
        assert!(v.screens[0].iter().any(|&b| b != 0));
    }

    #[test]
    fn itext_respects_its_left_margin_and_key_rules() {
        let Some(font) = real_font() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut it = HuIText::new(0, 0, font, b'!' as i32);
        it.add_prefix("Say: ");
        assert_eq!(it.lm, 5);
        it.del_char(); // margin: nothing to delete
        assert_eq!(it.l.l.len(), 5);

        assert!(it.key_in(b'H'));
        assert!(it.key_in(b'I'));
        assert_eq!(it.l.l, b"Say: HI");
        assert!(it.key_in(KEY_BACKSPACE as u8));
        assert_eq!(it.l.l, b"Say: H");
        assert!(
            it.key_in(KEY_ENTER as u8),
            "enter is eaten but adds nothing"
        );
        assert_eq!(it.l.l, b"Say: H");
        assert!(!it.key_in(1), "control chars aren't eaten");
        assert!(!it.key_in(b'a'), "lowercase is above '_': not eaten");

        it.erase_line();
        assert_eq!(it.l.l, b"Say: ");
        it.reset();
        assert!(it.l.l.is_empty());
        assert_eq!(it.lm, 0);
    }

    #[test]
    fn erase_text_line_restores_the_border_from_the_backup_screen() {
        let Some(font) = real_font() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut v = VVideo::new();
        v.screens[0].fill(1);
        v.screens[1].fill(9);
        // A reduced view (setblocks 10 style): 256 wide at x=32.
        let rdraw = RDraw {
            viewwindowx: 32,
            ..Default::default()
        };
        let view = ViewWin {
            x: 32,
            y: 0,
            width: 256,
            height: 144,
        };
        let lh = crate::v_video::patch_height(&font[0]) + 1;

        // Inside the view's rows: only the two side borders come back.
        let mut t = HuTextLine::new(0, 10, font.clone(), b'!' as i32);
        t.needsupdate = 2;
        t.erase(&view, &rdraw, &mut v);
        let row = (10 * SCREENWIDTH) as usize;
        assert_eq!(v.screens[0][row], 9, "left border erased");
        assert_eq!(v.screens[0][row + 32], 1, "view untouched");
        assert_eq!(v.screens[0][row + 32 + 256], 9, "right border erased");
        assert_eq!(t.needsupdate, 1);
        let _ = lh;

        // Below the view: the whole row.
        v.screens[0].fill(1);
        let mut t = HuTextLine::new(0, 150, font, b'!' as i32);
        t.needsupdate = 1;
        t.erase(&view, &rdraw, &mut v);
        let row = (150 * SCREENWIDTH) as usize;
        assert!(v.screens[0][row..row + SCREENWIDTH as usize]
            .iter()
            .all(|&b| b == 9));
        assert_eq!(t.needsupdate, 0);
    }
}
