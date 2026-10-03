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
//	Gamma correction LUT stuff.
//	Functions to draw patches (by post) directly to screen.
//	Functions to blit a block to the screen.
//
//-----------------------------------------------------------------------------

//! Rust port of `v_video.h` / `v_video.c`.
//!
//! Gamma correction LUT. Functions to draw patches (by post) directly to
//! screen. Functions to blit a block to the screen.
//!
//! # Patches
//!
//! `V_DrawPatch`/`V_DrawPatchFlipped`/`V_DrawPatchDirect` take a
//! `patch_t*`, which in the original is just a typed pointer into the
//! lump's raw bytes (header, `columnofs[]`, then column posts). Here a
//! patch is the lump's `&[u8]` and the header/columns are read from it
//! directly — same layout, no separate struct to keep in sync.
//! `V_DrawPatchDirect` is `V_DrawPatch` in the original too (its
//! VGA-planar body is commented out).
//!
//! `screens`/`dirtybox`/`gammatable`/`usegamma` are exposed as a single
//! [`VVideo`] struct (following the plan's globals pattern) rather than
//! free-standing `static`s, since `i_video.rs` needs to own/borrow one
//! instance of this alongside its own SDL state.

use crate::doomdef::{SCREENHEIGHT, SCREENWIDTH};
use crate::i_system::i_alloc_low;
use crate::m_bbox::{m_add_to_box, BBox};

/// A patch: the raw bytes of a `patch_t` lump (see the module docs),
/// shared cheaply between the widgets that draw it.
pub type PatchData = std::rc::Rc<[u8]>;

/// `patch->width`.
pub fn patch_width(patch: &[u8]) -> i32 {
    i16::from_le_bytes([patch[0], patch[1]]) as i32
}

/// `patch->height`.
pub fn patch_height(patch: &[u8]) -> i32 {
    i16::from_le_bytes([patch[2], patch[3]]) as i32
}

/// `patch->leftoffset`.
pub fn patch_leftoffset(patch: &[u8]) -> i32 {
    i16::from_le_bytes([patch[4], patch[5]]) as i32
}

/// `patch->topoffset`.
pub fn patch_topoffset(patch: &[u8]) -> i32 {
    i16::from_le_bytes([patch[6], patch[7]]) as i32
}

/// (`CENTERY`) = `SCREENHEIGHT/2`
pub const CENTERY: i32 = SCREENHEIGHT / 2;

/// Number of screen buffers (`screens[5]`). Screen 0 is the screen
/// updated by `I_UpdateScreen`(equivalent). Screen 1+ are extra buffers
/// (used by the status bar/wipe/etc. once those are ported).
pub const NUM_SCREENS: usize = 5;

/// Port of the `v_video` module state: `screens[5]`, `dirtybox[4]`,
/// `gammatable[5][256]`, `usegamma`.
pub struct VVideo {
    /// (`screens[5]`) — each screen is `[SCREENWIDTH*SCREENHEIGHT]`
    /// palette-indexed bytes. `Vec<u8>` in place of the original's
    /// `byte*` into one big `I_AllocLow`'d block — see
    /// [`VVideo::new`]/[`v_init`] for why a single shared allocation
    /// isn't reproduced.
    pub screens: [Vec<u8>; NUM_SCREENS],
    /// (`dirtybox[4]`) — `BOXTOP`/`BOXBOTTOM`/`BOXLEFT`/`BOXRIGHT`
    /// indices, see `m_bbox`.
    pub dirtybox: BBox,
    /// (`gammatable[5][256]`) — gamma correction LUT. "Now where did
    /// these came from?" (the original's own comment, preserved).
    pub gammatable: [[u8; 256]; 5],
    pub usegamma: i32,
}

impl Default for VVideo {
    fn default() -> Self {
        VVideo {
            screens: Default::default(),
            dirtybox: [0; 4],
            gammatable: GAMMATABLE,
            usegamma: 0,
        }
    }
}

impl VVideo {
    /// Port of `V_Init`. Allocates buffer screens, call before `R_Init`.
    ///
    /// The original allocates one `SCREENWIDTH*SCREENHEIGHT*4`-byte
    /// block via `I_AllocLow` and slices `screens[0..4]` as views into
    /// it (screen 4 is left unallocated — the original's loop only goes
    /// up to `i<4`, a detail preserved here: `screens[4]` stays empty
    /// after `new()`/`v_init()`, exactly like the original never points
    /// it anywhere). Four independent `Vec<u8>` buffers are used instead
    /// of one shared allocation sliced into four — the original's
    /// reason for a single block (fitting in scarce low DOS memory) no
    /// longer applies, and separate buffers are both simpler and safer
    /// in Rust (no aliasing between "different" screens).
    pub fn new() -> Self {
        let mut v = VVideo::default();
        v.v_init();
        v
    }

    pub fn v_init(&mut self) {
        let size = (SCREENWIDTH * SCREENHEIGHT) as usize;
        for screen in self.screens.iter_mut().take(4) {
            *screen = i_alloc_low(size);
        }
    }

    /// Port of `V_MarkRect`.
    pub fn v_mark_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        m_add_to_box(&mut self.dirtybox, x, y);
        m_add_to_box(&mut self.dirtybox, x + width - 1, y + height - 1);
    }

    /// Port of `V_CopyRect`.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on out-of-range coordinates,
    /// same `#ifdef RANGECHECK` guard as the original (always compiled
    /// in here, matching `doomdef.h`'s `#define RANGECHECK`).
    #[allow(clippy::too_many_arguments)] // mirrors V_CopyRect's original 8-parameter signature
    pub fn v_copy_rect(
        &mut self,
        srcx: i32,
        srcy: i32,
        srcscrn: usize,
        width: i32,
        height: i32,
        destx: i32,
        desty: i32,
        destscrn: usize,
    ) {
        if srcx < 0
            || srcx + width > SCREENWIDTH
            || srcy < 0
            || srcy + height > SCREENHEIGHT
            || destx < 0
            || destx + width > SCREENWIDTH
            || desty < 0
            || desty + height > SCREENHEIGHT
            || srcscrn > 4
            || destscrn > 4
        {
            panic!("Bad V_CopyRect");
        }

        self.v_mark_rect(destx, desty, width, height);

        let w = SCREENWIDTH as usize;
        let width = width as usize;
        let mut height = height as usize;
        let mut src_off = w * srcy as usize + srcx as usize;
        let mut dest_off = w * desty as usize + destx as usize;

        while height > 0 {
            // Copy one row at a time; done via a temp buffer when
            // src/dest are the same screen (self-copy) to sidestep the
            // borrow checker disallowing two overlapping `&mut`
            // slices — functionally identical to the original's
            // (overlap-unsafe, but never actually exercised with
            // overlapping ranges in the original's callers) `memcpy`.
            if srcscrn == destscrn {
                let mut row = vec![0u8; width];
                row.copy_from_slice(&self.screens[srcscrn][src_off..src_off + width]);
                self.screens[destscrn][dest_off..dest_off + width].copy_from_slice(&row);
            } else {
                let (src_buf, dest_buf) = borrow_two_mut(&mut self.screens, srcscrn, destscrn);
                dest_buf[dest_off..dest_off + width]
                    .copy_from_slice(&src_buf[src_off..src_off + width]);
            }
            src_off += w;
            dest_off += w;
            height -= 1;
        }
    }

    /// Draws the columns of a patch (`V_DrawPatch`'s and
    /// `V_DrawPatchFlipped`'s shared loop). `x`/`y` are already
    /// adjusted for the patch's offsets.
    fn draw_patch_columns(&mut self, x: i32, y: i32, scrn: usize, patch: &[u8], flipped: bool) {
        let w = i16::from_le_bytes([patch[0], patch[1]]) as i32;
        let sw = SCREENWIDTH as usize;
        let desttop = y as usize * sw + x as usize;

        for col in 0..w {
            let src_col = if flipped { w - 1 - col } else { col } as usize;
            let ofs_at = 8 + 4 * src_col;
            let mut column =
                i32::from_le_bytes(patch[ofs_at..ofs_at + 4].try_into().unwrap()) as usize;

            while patch[column] != 0xff {
                let topdelta = patch[column] as usize;
                let count = patch[column + 1] as usize;
                let mut dest = desttop + col as usize + topdelta * sw;
                for &b in &patch[column + 3..column + 3 + count] {
                    self.screens[scrn][dest] = b;
                    dest += sw;
                }
                column += count + 4;
            }
        }
    }

    /// `x`/`y` after applying the patch's offsets, and whether the
    /// patch then fits the screen (`RANGECHECK`).
    fn patch_placement(x: i32, y: i32, scrn: usize, patch: &[u8]) -> (i32, i32, i32, i32, bool) {
        let width = i16::from_le_bytes([patch[0], patch[1]]) as i32;
        let height = i16::from_le_bytes([patch[2], patch[3]]) as i32;
        let leftoffset = i16::from_le_bytes([patch[4], patch[5]]) as i32;
        let topoffset = i16::from_le_bytes([patch[6], patch[7]]) as i32;
        let y = y - topoffset;
        let x = x - leftoffset;
        let ok =
            !(x < 0 || x + width > SCREENWIDTH || y < 0 || y + height > SCREENHEIGHT || scrn > 4);
        (x, y, width, height, ok)
    }

    /// Port of `V_DrawPatch`. Masks a column based masked pic to the
    /// screen (the original's own comment, preserved).
    ///
    /// A patch that doesn't fit the screen is reported on stderr and
    /// ignored, as in the original (`RANGECHECK`).
    pub fn v_draw_patch(&mut self, x: i32, y: i32, scrn: usize, patch: &[u8]) {
        let (x, y, width, height, ok) = Self::patch_placement(x, y, scrn, patch);
        if !ok {
            eprintln!("Patch at {x},{y} exceeds LFB");
            eprintln!("V_DrawPatch: bad patch (ignored)");
            return;
        }
        if scrn == 0 {
            self.v_mark_rect(x, y, width, height);
        }
        self.draw_patch_columns(x, y, scrn, patch, false);
    }

    /// Port of `V_DrawPatchFlipped`. Masks a column based masked pic to
    /// the screen, mirrored horizontally (Doom II's cast/finale).
    ///
    /// # Panics
    /// If the patch doesn't fit the screen (`I_Error`).
    pub fn v_draw_patch_flipped(&mut self, x: i32, y: i32, scrn: usize, patch: &[u8]) {
        let (x, y, width, height, ok) = Self::patch_placement(x, y, scrn, patch);
        if !ok {
            eprintln!("Patch origin {x},{y} exceeds LFB");
            panic!("Bad V_DrawPatch in V_DrawPatchFlipped");
        }
        if scrn == 0 {
            self.v_mark_rect(x, y, width, height);
        }
        self.draw_patch_columns(x, y, scrn, patch, true);
    }

    /// Port of `V_DrawPatchDirect`. Draws directly to the screen on the
    /// pc (the original's own comment) — in this SDL build the original
    /// just calls [`VVideo::v_draw_patch`], and so does this.
    pub fn v_draw_patch_direct(&mut self, x: i32, y: i32, scrn: usize, patch: &[u8]) {
        self.v_draw_patch(x, y, scrn, patch);
    }

    /// Port of `V_DrawBlock`. Draw a linear block of pixels into the
    /// view buffer.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on out-of-range coordinates.
    pub fn v_draw_block(
        &mut self,
        x: i32,
        y: i32,
        scrn: usize,
        width: i32,
        height: i32,
        src: &[u8],
    ) {
        if x < 0 || x + width > SCREENWIDTH || y < 0 || y + height > SCREENHEIGHT || scrn > 4 {
            panic!("Bad V_DrawBlock");
        }

        self.v_mark_rect(x, y, width, height);

        let w = SCREENWIDTH as usize;
        let width = width as usize;
        let mut dest_off = w * y as usize + x as usize;
        let mut src_off = 0;
        let mut height = height;

        while height > 0 {
            self.screens[scrn][dest_off..dest_off + width]
                .copy_from_slice(&src[src_off..src_off + width]);
            src_off += width;
            dest_off += w;
            height -= 1;
        }
    }

    /// Port of `V_GetBlock`. Reads a linear block of pixels from the
    /// view buffer.
    ///
    /// Note: the original's error message here ("Bad V_DrawBlock") is
    /// copy-pasted from `V_DrawBlock` rather than saying `V_GetBlock` —
    /// preserved verbatim for fidelity, not "fixed".
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on out-of-range coordinates.
    pub fn v_get_block(
        &self,
        x: i32,
        y: i32,
        scrn: usize,
        width: i32,
        height: i32,
        dest: &mut [u8],
    ) {
        if x < 0 || x + width > SCREENWIDTH || y < 0 || y + height > SCREENHEIGHT || scrn > 4 {
            panic!("Bad V_DrawBlock");
        }

        let w = SCREENWIDTH as usize;
        let width = width as usize;
        let mut src_off = w * y as usize + x as usize;
        let mut dest_off = 0;
        let mut height = height;

        while height > 0 {
            dest[dest_off..dest_off + width]
                .copy_from_slice(&self.screens[scrn][src_off..src_off + width]);
            src_off += w;
            dest_off += width;
            height -= 1;
        }
    }
}

/// Borrow two distinct elements of a `[Vec<u8>; N]` mutably at once.
/// Panics if `a == b` (callers must not call this for a same-screen
/// copy — see [`VVideo::v_copy_rect`], which branches around that case).
fn borrow_two_mut<T>(arr: &mut [T; NUM_SCREENS], a: usize, b: usize) -> (&T, &mut T) {
    assert_ne!(a, b);
    if a < b {
        let (left, right) = arr.split_at_mut(b);
        (&left[a], &mut right[0])
    } else {
        let (left, right) = arr.split_at_mut(a);
        (&right[0], &mut left[b])
    }
}

/// (`gammatable[5][256]`), transcribed verbatim from `v_video.c` (extracted
/// programmatically to avoid manual transcription errors, same approach
/// as tables.rs's trig tables in Phase 1).
pub const GAMMATABLE: [[u8; 256]; 5] = [
    [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
        49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71,
        72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94,
        95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113,
        114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 128, 129, 130,
        131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148,
        149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166,
        167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184,
        185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202,
        203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220,
        221, 222, 223, 224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238,
        239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254, 255,
    ],
    [
        2, 4, 5, 7, 8, 10, 11, 12, 14, 15, 16, 18, 19, 20, 21, 23, 24, 25, 26, 27, 29, 30, 31, 32,
        33, 34, 36, 37, 38, 39, 40, 41, 42, 44, 45, 46, 47, 48, 49, 50, 51, 52, 54, 55, 56, 57, 58,
        59, 60, 61, 62, 63, 64, 65, 66, 67, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82,
        83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103,
        104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121,
        122, 123, 124, 125, 126, 127, 128, 129, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138,
        139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 148, 149, 150, 151, 152, 153, 154, 155,
        156, 157, 158, 159, 160, 161, 162, 163, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172,
        173, 174, 175, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 186, 187, 188,
        189, 190, 191, 192, 193, 194, 195, 196, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205,
        205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 214, 215, 216, 217, 218, 219, 220, 221,
        222, 222, 223, 224, 225, 226, 227, 228, 229, 230, 230, 231, 232, 233, 234, 235, 236, 237,
        237, 238, 239, 240, 241, 242, 243, 244, 245, 245, 246, 247, 248, 249, 250, 251, 252, 252,
        253, 254, 255,
    ],
    [
        4, 7, 9, 11, 13, 15, 17, 19, 21, 22, 24, 26, 27, 29, 30, 32, 33, 35, 36, 38, 39, 40, 42,
        43, 45, 46, 47, 48, 50, 51, 52, 54, 55, 56, 57, 59, 60, 61, 62, 63, 65, 66, 67, 68, 69, 70,
        72, 73, 74, 75, 76, 77, 78, 79, 80, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95,
        96, 97, 98, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 114,
        115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132,
        133, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 144, 145, 146, 147, 148,
        149, 150, 151, 152, 153, 153, 154, 155, 156, 157, 158, 159, 160, 160, 161, 162, 163, 164,
        165, 166, 166, 167, 168, 169, 170, 171, 172, 172, 173, 174, 175, 176, 177, 178, 178, 179,
        180, 181, 182, 183, 183, 184, 185, 186, 187, 188, 188, 189, 190, 191, 192, 193, 193, 194,
        195, 196, 197, 197, 198, 199, 200, 201, 201, 202, 203, 204, 205, 206, 206, 207, 208, 209,
        210, 210, 211, 212, 213, 213, 214, 215, 216, 217, 217, 218, 219, 220, 221, 221, 222, 223,
        224, 224, 225, 226, 227, 228, 228, 229, 230, 231, 231, 232, 233, 234, 235, 235, 236, 237,
        238, 238, 239, 240, 241, 241, 242, 243, 244, 244, 245, 246, 247, 247, 248, 249, 250, 251,
        251, 252, 253, 254, 254, 255,
    ],
    [
        8, 12, 16, 19, 22, 24, 27, 29, 31, 34, 36, 38, 40, 41, 43, 45, 47, 49, 50, 52, 53, 55, 57,
        58, 60, 61, 63, 64, 65, 67, 68, 70, 71, 72, 74, 75, 76, 77, 79, 80, 81, 82, 84, 85, 86, 87,
        88, 90, 91, 92, 93, 94, 95, 96, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109,
        110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127,
        128, 129, 130, 131, 132, 133, 134, 135, 135, 136, 137, 138, 139, 140, 141, 142, 143, 143,
        144, 145, 146, 147, 148, 149, 150, 150, 151, 152, 153, 154, 155, 155, 156, 157, 158, 159,
        160, 160, 161, 162, 163, 164, 165, 165, 166, 167, 168, 169, 169, 170, 171, 172, 173, 173,
        174, 175, 176, 176, 177, 178, 179, 180, 180, 181, 182, 183, 183, 184, 185, 186, 186, 187,
        188, 189, 189, 190, 191, 192, 192, 193, 194, 195, 195, 196, 197, 197, 198, 199, 200, 200,
        201, 202, 202, 203, 204, 205, 205, 206, 207, 207, 208, 209, 210, 210, 211, 212, 212, 213,
        214, 214, 215, 216, 216, 217, 218, 219, 219, 220, 221, 221, 222, 223, 223, 224, 225, 225,
        226, 227, 227, 228, 229, 229, 230, 231, 231, 232, 233, 233, 234, 235, 235, 236, 237, 237,
        238, 238, 239, 240, 240, 241, 242, 242, 243, 244, 244, 245, 246, 246, 247, 247, 248, 249,
        249, 250, 251, 251, 252, 253, 253, 254, 254, 255,
    ],
    [
        16, 23, 28, 32, 36, 39, 42, 45, 48, 50, 53, 55, 57, 60, 62, 64, 66, 68, 69, 71, 73, 75, 76,
        78, 80, 81, 83, 84, 86, 87, 89, 90, 92, 93, 94, 96, 97, 98, 100, 101, 102, 103, 105, 106,
        107, 108, 109, 110, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125,
        126, 128, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143,
        143, 144, 145, 146, 147, 148, 149, 150, 150, 151, 152, 153, 154, 155, 155, 156, 157, 158,
        159, 159, 160, 161, 162, 163, 163, 164, 165, 166, 166, 167, 168, 169, 169, 170, 171, 172,
        172, 173, 174, 175, 175, 176, 177, 177, 178, 179, 180, 180, 181, 182, 182, 183, 184, 184,
        185, 186, 187, 187, 188, 189, 189, 190, 191, 191, 192, 193, 193, 194, 195, 195, 196, 196,
        197, 198, 198, 199, 200, 200, 201, 202, 202, 203, 203, 204, 205, 205, 206, 207, 207, 208,
        208, 209, 210, 210, 211, 211, 212, 213, 213, 214, 214, 215, 216, 216, 217, 217, 218, 219,
        219, 220, 220, 221, 221, 222, 223, 223, 224, 224, 225, 225, 226, 227, 227, 228, 228, 229,
        229, 230, 230, 231, 232, 232, 233, 233, 234, 234, 235, 235, 236, 236, 237, 237, 238, 239,
        239, 240, 240, 241, 241, 242, 242, 243, 243, 244, 244, 245, 245, 246, 246, 247, 247, 248,
        248, 249, 249, 250, 250, 251, 251, 252, 252, 253, 254, 254, 255, 255,
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centery_matches_original() {
        assert_eq!(CENTERY, 100);
    }

    #[test]
    fn v_init_allocates_first_four_screens_only() {
        let v = VVideo::new();
        let expected_len = (SCREENWIDTH * SCREENHEIGHT) as usize;
        for screen in &v.screens[0..4] {
            assert_eq!(screen.len(), expected_len);
        }
        // Original's V_Init loop only covers screens[0..4); screens[4]
        // is never touched.
        assert_eq!(v.screens[4].len(), 0);
    }

    #[test]
    fn gammatable_first_and_last_rows_match_source() {
        assert_eq!(GAMMATABLE[0][0], 1);
        assert_eq!(GAMMATABLE[0][255], 255);
        assert_eq!(GAMMATABLE[4][0], 16);
        assert_eq!(GAMMATABLE[4][255], 255);
    }

    #[test]
    fn v_draw_block_then_v_get_block_round_trips() {
        let mut v = VVideo::new();
        let src = vec![42u8; 10 * 5];
        v.v_draw_block(3, 2, 0, 10, 5, &src);

        let mut dest = vec![0u8; 10 * 5];
        v.v_get_block(3, 2, 0, 10, 5, &mut dest);
        assert_eq!(dest, src);
    }

    #[test]
    #[should_panic(expected = "Bad V_DrawBlock")]
    fn v_draw_block_panics_out_of_range() {
        let mut v = VVideo::new();
        let src = vec![0u8; 10];
        v.v_draw_block(-1, 0, 0, 10, 1, &src);
    }

    #[test]
    fn v_copy_rect_between_distinct_screens() {
        let mut v = VVideo::new();
        let src = vec![7u8; 4 * 4];
        v.v_draw_block(0, 0, 0, 4, 4, &src);

        v.v_copy_rect(0, 0, 0, 4, 4, 5, 5, 1);

        let mut dest = vec![0u8; 4 * 4];
        v.v_get_block(5, 5, 1, 4, 4, &mut dest);
        assert_eq!(dest, src);
    }

    #[test]
    fn v_copy_rect_within_same_screen() {
        let mut v = VVideo::new();
        let src = vec![9u8; 4 * 4];
        v.v_draw_block(0, 0, 0, 4, 4, &src);

        v.v_copy_rect(0, 0, 0, 4, 4, 10, 10, 0);

        let mut dest = vec![0u8; 4 * 4];
        v.v_get_block(10, 10, 0, 4, 4, &mut dest);
        assert_eq!(dest, src);
    }

    #[test]
    fn v_mark_rect_grows_dirtybox() {
        let mut v = VVideo::new();
        v.v_mark_rect(10, 20, 5, 5);
        // dirtybox starts at all zeros (Default), not M_ClearBox's
        // inverted-extremes state, matching the original's static
        // zero-init of `int dirtybox[4]` (never explicitly
        // M_ClearBox'd in v_video.c itself).
        // BOXLEFT starts at 0 and m_add_to_box only shrinks it when a
        // smaller x is added; 10 > 0, so it stays 0.
        assert_eq!(v.dirtybox[crate::m_bbox::BOXLEFT], 0);
    }

    /// Test-only: the pixels of a patch decoded independently of
    /// `V_DrawPatch` — `width x height`, `None` where transparent.
    fn decode_patch(patch: &[u8]) -> (i32, i32, Vec<Option<u8>>) {
        let (w, h) = (patch_width(patch), patch_height(patch));
        let mut px = vec![None; (w * h) as usize];
        for col in 0..w as usize {
            let mut at =
                i32::from_le_bytes(patch[8 + 4 * col..12 + 4 * col].try_into().unwrap()) as usize;
            while patch[at] != 0xff {
                let (top, len) = (patch[at] as usize, patch[at + 1] as usize);
                for k in 0..len {
                    px[(top + k) * w as usize + col] = Some(patch[at + 3 + k]);
                }
                at += len + 4;
            }
        }
        (w, h, px)
    }

    fn real_patch(name: &str) -> Option<Vec<u8>> {
        let path = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut wad = crate::w_wad::WadFiles::new();
        wad.init_file(path);
        Some(
            wad.cache_lump_name(name, crate::z_zone::PurgeTag::Cache)
                .to_vec(),
        )
    }

    #[test]
    fn draw_patch_places_every_opaque_pixel_and_skips_transparent_ones() {
        let Some(patch) = real_patch("STCFN065") else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let (w, h, px) = decode_patch(&patch);
        let mut v = VVideo::new();
        v.screens[0].fill(7);
        crate::m_bbox::m_clear_box(&mut v.dirtybox);
        let (lo, to) = (patch_leftoffset(&patch), patch_topoffset(&patch));

        v.v_draw_patch(50, 40, 0, &patch);

        let (x0, y0) = (50 - lo, 40 - to);
        for row in 0..h {
            for col in 0..w {
                let got = v.screens[0][((y0 + row) * SCREENWIDTH + x0 + col) as usize];
                let want = px[(row * w + col) as usize].unwrap_or(7);
                assert_eq!(got, want, "pixel ({col},{row})");
            }
        }
        // and it marked the dirty box
        assert_eq!(v.dirtybox[crate::m_bbox::BOXLEFT], x0);
    }

    #[test]
    fn draw_patch_flipped_mirrors_columns() {
        let Some(patch) = real_patch("STCFN065") else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let (w, h, px) = decode_patch(&patch);
        let mut v = VVideo::new();
        v.screens[0].fill(7);
        let (lo, to) = (patch_leftoffset(&patch), patch_topoffset(&patch));

        v.v_draw_patch_flipped(50, 40, 0, &patch);

        let (x0, y0) = (50 - lo, 40 - to);
        for row in 0..h {
            for col in 0..w {
                let got = v.screens[0][((y0 + row) * SCREENWIDTH + x0 + col) as usize];
                let want = px[(row * w + (w - 1 - col)) as usize].unwrap_or(7);
                assert_eq!(got, want, "pixel ({col},{row})");
            }
        }
    }

    #[test]
    fn draw_patch_off_screen_is_ignored_but_flipped_panics() {
        let Some(patch) = real_patch("STCFN065") else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut v = VVideo::new();
        v.v_draw_patch(SCREENWIDTH - 1, 0, 0, &patch);
        assert!(v.screens[0].iter().all(|&b| b == 0), "nothing drawn");

        let r = std::panic::catch_unwind(move || {
            let mut v = VVideo::new();
            v.v_draw_patch_flipped(SCREENWIDTH - 1, 0, 0, &patch);
        });
        assert!(r.is_err());
    }

    #[test]
    fn draw_patch_to_a_backup_screen_does_not_mark_dirty() {
        let Some(patch) = real_patch("STCFN065") else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut v = VVideo::new();
        let before = v.dirtybox;
        v.v_draw_patch(50, 40, 1, &patch);
        assert_eq!(v.dirtybox, before);
        assert!(v.screens[1].iter().any(|&b| b != 0));
    }
}
