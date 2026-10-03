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
//	The actual span/column drawing functions.
//	Here find the main potential for optimization,
//	 e.g. inline assembly, different algorithms.
//
//-----------------------------------------------------------------------------

//! Rust port of `r_draw.h` / `r_draw.c` (partial, see note below).
//! The actual span/column drawing functions. Here find the main
//! potential for optimization, e.g. inline assembly, different
//! algorithms (the original's own comment, preserved).
//!
//! # Scope
//!
//! Ported: [`RDraw::r_draw_column`] (`R_DrawColumn`),
//! [`RDraw::r_draw_column_low`] (`R_DrawColumnLow`),
//! [`RDraw::r_draw_fuzz_column`] (`R_DrawFuzzColumn`),
//! [`RDraw::r_draw_translated_column`] (`R_DrawTranslatedColumn`),
//! [`RDraw::r_draw_span`] (`R_DrawSpan`), [`RDraw::r_draw_span_low`]
//! (`R_DrawSpanLow`), [`RDraw::r_init_buffer`] (`R_InitBuffer`),
//! [`r_init_translation_tables`] (`R_InitTranslationTables`),
//! [`RDraw::r_video_erase`] (`R_VideoErase`).
//!
//! Not ported: `R_FillBackScreen`/`R_DrawViewBorder` — both depend on
//! `V_DrawPatch` (`patch_t*`-based blitting, `v_video.c`'s own scope
//! note explains why that's deferred) and on `W_CacheLumpName`'d
//! border-patch lumps (`brdr_*`/`FLOOR7_2`/`GRNROCK`); no caller exists
//! yet either (`g_game`/`d_main`'s view-size/border logic, Phase 6),
//! so porting them now would be untestable dead code.
//!
//! # Representation choices
//!
//! `dc_colormap`/`ds_colormap` are indices into
//! [`crate::r_data::RData::colormaps`] (256-byte-aligned lookup tables)
//! rather than raw `lighttable_t*` pointers — same choice already made
//! throughout `r_main`/`r_bsp` for pointer-heavy original fields.
//! `dc_source`/`ds_source` are borrowed byte slices (a texture/flat's
//! cached column or a full 4096-byte flat) passed in per call rather
//! than a persistent global pointer, since the underlying cache
//! (`r_data::RData`'s composite-texture cache) is itself owned data now,
//! not a raw pointer the original could just stash.
//!
//! `ylookup`/`columnofs` (`R_InitBuffer`'s output) become plain `Vec<i32>`
//! fields on [`RDraw`] holding *row start offsets into `screens[0]`* and
//! *column offsets* respectively, rather than `byte* ylookup[MAXHEIGHT]`
//! (a pointer per row) — an offset is enough since callers already hold
//! `&mut VVideo`/`screens[0]` directly; this avoids needing a raw
//! pointer or a lifetime-carrying reference stored on the struct.
//!
//! `translationtables` is a plain `[[u8; 256]; 3]` instead of a
//! `Z_Malloc`'d, 255-byte-aligned raw buffer — the original's alignment
//! dance exists only to satisfy an assumed-fast aligned-access pattern
//! on period hardware and has no behavioral effect on the lookup itself
//! (see [`r_init_translation_tables`]'s docs), and the buffer is never
//! freed/reloaded through `z_zone`'s purge machinery like real cached
//! lumps are, so a handle would add ceremony with no payoff.

use crate::doomdef::{GameMode, SCREENHEIGHT, SCREENWIDTH};
use crate::m_fixed::{Fixed, FRACBITS};
use crate::v_video::{VVideo, CENTERY};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// (`SBARHEIGHT`) The status bar's height, for the border code.
const SBARHEIGHT: i32 = 32;

/// (`FUZZTABLE`) — number of entries in [`FUZZOFFSET`].
pub const FUZZTABLE: usize = 50;

/// (`fuzzoffset[FUZZTABLE]`), transcribed verbatim from `r_draw.c`.
/// Values are `+SCREENWIDTH`/`-SCREENWIDTH` (`FUZZOFF`), i.e. "one row
/// down"/"one row up" in the framebuffer's linear layout.
#[rustfmt::skip]
pub const FUZZOFFSET: [i32; FUZZTABLE] = [
    1, -1, 1, -1, 1, 1, -1,
    1, 1, -1, 1, 1, 1, -1,
    1, 1, 1, -1, -1, -1, -1,
    1, -1, -1, 1, 1, 1, 1, -1,
    1, -1, 1, 1, -1, -1, 1,
    1, -1, -1, -1, -1, 1, 1,
    1, 1, -1, 1, 1, -1, 1,
];

/// Port of the `r_draw` module's per-call drawing parameters
/// (`dc_x`/`dc_yl`/`dc_yh`/`dc_iscale`/`dc_texturemid`/`dc_colormap`/
/// `dc_source`, `ds_y`/`ds_x1`/`ds_x2`/`ds_xfrac`/`ds_yfrac`/
/// `ds_xstep`/`ds_ystep`/`ds_colormap`/`ds_source`) plus the
/// framebuffer-addressing lookup tables (`ylookup`/`columnofs`) and
/// `fuzzpos`. Grouped into one struct (following the plan's globals
/// pattern) since every field here is set immediately before a draw
/// call and consumed only by that call, mirroring the original's
/// module-global scratch variables.
#[derive(Default)]
pub struct RDraw {
    pub dc_x: i32,
    pub dc_yl: i32,
    pub dc_yh: i32,
    pub dc_iscale: Fixed,
    pub dc_texturemid: Fixed,
    /// Index into [`crate::r_data::RData::colormaps`] of the 256-byte
    /// colormap row to use (`dc_colormap`).
    pub dc_colormap: usize,
    /// `dc_translation` — index into [`RDraw::translationtables`]'s
    /// selected 256-byte row (0/1/2), or `None` for no translation
    /// (the original always has a valid `dc_translation` pointer at
    /// call sites of `R_DrawTranslatedColumn`; `None` exists here only
    /// as this struct's `Default`).
    pub dc_translation: Option<usize>,

    pub ds_y: i32,
    pub ds_x1: i32,
    pub ds_x2: i32,
    pub ds_xfrac: Fixed,
    pub ds_yfrac: Fixed,
    pub ds_xstep: Fixed,
    pub ds_ystep: Fixed,
    /// Index into [`crate::r_data::RData::colormaps`] (`ds_colormap`).
    pub ds_colormap: usize,

    /// (`ylookup[MAXHEIGHT]`) — row start offset into `screens[0]` for
    /// each view row, see module docs.
    pub ylookup: Vec<i32>,
    /// (`columnofs[MAXWIDTH]`) — column offset for each screen column.
    pub columnofs: Vec<i32>,

    pub viewwindowx: i32,
    pub viewwindowy: i32,

    /// (`centery`) Half the view height, set by `R_ExecuteSetViewSize` —
    /// [`RDraw::r_init_buffer`] here. The column/span drawers measure
    /// texture rows from it, so it must follow the view size (a
    /// status-bar view is 168 high, `centery` 84, not the full-screen
    /// [`CENTERY`]).
    pub centery: i32,

    /// (`scaledviewwidth`/`viewheight`) The view window's size in
    /// screen pixels, set by [`RDraw::r_init_buffer`]; the border
    /// drawing reads them.
    pub scaledviewwidth: i32,
    pub viewheight: i32,

    /// (`fuzzpos`) — current index into [`FUZZOFFSET`], persists across
    /// [`RDraw::r_draw_fuzz_column`] calls.
    pub fuzzpos: usize,

    /// (`translations[3][256]`/`translationtables`) — see module docs.
    /// `None` until [`r_init_translation_tables`] runs, matching the
    /// original's uninitialized-until-`R_InitTranslationTables` state.
    pub translationtables: Option<[[u8; 256]; 3]>,
}

/// `dc_source[i]` for the column drawers. The original indexes with
/// `(frac >> FRACBITS) & 127` straight into the column, which for a
/// texture shorter than 128 can land one or more texels past the end of
/// the column (rounding at the bottom edge) and silently reads whatever
/// memory follows; here that would be an out-of-bounds panic, so the
/// index wraps around the column's own length (its natural tiling).
#[inline]
fn texel(source: &[u8], i: usize) -> u8 {
    match source.get(i) {
        Some(&t) => t,
        None => source[i % source.len()],
    }
}

impl RDraw {
    pub fn new() -> Self {
        RDraw {
            centery: CENTERY, // a full-screen view until R_ExecuteSetViewSize
            ..Self::default()
        }
    }

    /// Port of `R_InitBuffer`. Creates lookup tables that avoid
    /// multiplies and other hassles for getting the framebuffer address
    /// of a pixel to draw (the original's own comment, preserved).
    pub fn r_init_buffer(&mut self, width: i32, height: i32) {
        self.centery = height / 2;
        self.scaledviewwidth = width;
        self.viewheight = height;
        self.viewwindowx = (SCREENWIDTH - width) >> 1;

        self.columnofs = vec![0; width as usize];
        for (i, ofs) in self.columnofs.iter_mut().enumerate() {
            *ofs = self.viewwindowx + i as i32;
        }

        self.viewwindowy = if width == SCREENWIDTH {
            0
        } else {
            (SCREENHEIGHT - SBARHEIGHT - height) >> 1
        };

        self.ylookup = vec![0; height as usize];
        for (i, row) in self.ylookup.iter_mut().enumerate() {
            *row = (i as i32 + self.viewwindowy) * SCREENWIDTH;
        }
    }

    /// Port of `R_DrawColumn`. Source is the top of the column to scale
    /// (the original's own comment, preserved).
    ///
    /// `colormaps`/`source` stand in for indexing through
    /// `dc_colormap`/`dc_source` respectively — `source` is the cached
    /// column bytes (`dc_source[...]` in the original), `colormaps` is
    /// the full [`crate::r_data::RData::colormaps`] buffer indexed by
    /// `self.dc_colormap * 256 + ...`.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on out-of-range `dc_x`/
    /// `dc_yl`/`dc_yh`, same `#ifdef RANGECHECK` guard as the original
    /// (always compiled in here, matching `doomdef.h`'s `#define
    /// RANGECHECK`).
    pub fn r_draw_column(&self, screen: &mut [u8], colormaps: &[u8], source: &[u8]) {
        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        assert!(
            (self.dc_x as u32) < SCREENWIDTH as u32 && self.dc_yl >= 0 && self.dc_yh < SCREENHEIGHT,
            "R_DrawColumn: {} to {} at {}",
            self.dc_yl,
            self.dc_yh,
            self.dc_x
        );

        let mut dest =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[self.dc_x as usize]) as usize;

        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        let colormap = &colormaps[self.dc_colormap * 256..self.dc_colormap * 256 + 256];

        let mut count = count;
        loop {
            screen[dest] = colormap[texel(source, ((frac >> FRACBITS) & 127) as usize) as usize];
            dest += SCREENWIDTH as usize;
            frac = frac.wrapping_add(fracstep);

            if count == 0 {
                break;
            }
            count -= 1;
        }
    }

    /// Port of `R_DrawColumnLow`. Low resolution ("blocky") mode: writes
    /// each sampled pixel to two adjacent screen columns.
    ///
    /// # Panics
    /// Same as [`RDraw::r_draw_column`].
    pub fn r_draw_column_low(&mut self, screen: &mut [u8], colormaps: &[u8], source: &[u8]) {
        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        assert!(
            (self.dc_x as u32) < SCREENWIDTH as u32 && self.dc_yl >= 0 && self.dc_yh < SCREENHEIGHT,
            "R_DrawColumn: {} to {} at {}",
            self.dc_yl,
            self.dc_yh,
            self.dc_x
        );

        // Blocky mode, need to multiply by 2 (the original's own
        // comment, preserved) — mutates `dc_x` itself, matching the
        // original's own in-place `dc_x <<= 1`.
        self.dc_x <<= 1;

        let mut dest =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[self.dc_x as usize]) as usize;
        let mut dest2 =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[self.dc_x as usize + 1]) as usize;

        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        let colormap = &colormaps[self.dc_colormap * 256..self.dc_colormap * 256 + 256];

        let mut count = count;
        loop {
            let pixel = colormap[texel(source, ((frac >> FRACBITS) & 127) as usize) as usize];
            screen[dest] = pixel;
            screen[dest2] = pixel;
            dest += SCREENWIDTH as usize;
            dest2 += SCREENWIDTH as usize;
            frac = frac.wrapping_add(fracstep);

            if count == 0 {
                break;
            }
            count -= 1;
        }
    }

    /// Port of `R_DrawFuzzColumn`. Framebuffer postprocessing. Creates a
    /// fuzzy image by copying pixels from adjacent ones to left and
    /// right. Used with an all black colormap, this could create the
    /// SHADOW effect, i.e. spectres and invisible players (the
    /// original's own comment, preserved).
    ///
    /// `colormaps` must be [`crate::r_data::RData::colormaps`] (colormap
    /// row 6 of it is read directly, hardcoded by the original).
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on out-of-range `dc_x`/
    /// `dc_yl`/`dc_yh`, same as the original's `#ifdef RANGECHECK`
    /// guard.
    pub fn r_draw_fuzz_column(&mut self, screen: &mut [u8], colormaps: &[u8], viewheight: i32) {
        // Adjust borders. Low... (the original's own comment, preserved)
        if self.dc_yl == 0 {
            self.dc_yl = 1;
        }
        // .. and high.
        if self.dc_yh == viewheight - 1 {
            self.dc_yh = viewheight - 2;
        }

        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        assert!(
            (self.dc_x as u32) < SCREENWIDTH as u32 && self.dc_yl >= 0 && self.dc_yh < SCREENHEIGHT,
            "R_DrawFuzzColumn: {} to {} at {}",
            self.dc_yl,
            self.dc_yh,
            self.dc_x
        );

        let mut dest =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[self.dc_x as usize]) as usize;

        let colormap6 = &colormaps[6 * 256..6 * 256 + 256];

        let mut count = count;
        loop {
            // Lookup framebuffer, and retrieve a pixel that is either
            // one column left or right of the current one. Add index
            // from colormap to index (the original's own comment,
            // preserved).
            let neighbor = (dest as i32 + FUZZOFFSET[self.fuzzpos] * SCREENWIDTH) as usize;
            screen[dest] = colormap6[screen[neighbor] as usize];

            self.fuzzpos += 1;
            if self.fuzzpos == FUZZTABLE {
                self.fuzzpos = 0;
            }

            dest += SCREENWIDTH as usize;

            if count == 0 {
                break;
            }
            count -= 1;
        }
    }

    /// Port of `R_DrawTranslatedColumn`. Used to draw player sprites
    /// with the green colorramp mapped to others. Could be used with
    /// different translation tables, e.g. the lighter colored version
    /// of the BaronOfHell, the HellKnight, uses identical sprites, kinda
    /// brightened up (the original's own comment, preserved).
    ///
    /// # Panics
    /// Panics if `dc_translation` is `None`, or on out-of-range
    /// `dc_x`/`dc_yl`/`dc_yh` (same `#ifdef RANGECHECK` guard as the
    /// original).
    pub fn r_draw_translated_column(&self, screen: &mut [u8], colormaps: &[u8], source: &[u8]) {
        let count = self.dc_yh - self.dc_yl;
        if count < 0 {
            return;
        }

        assert!(
            (self.dc_x as u32) < SCREENWIDTH as u32 && self.dc_yl >= 0 && self.dc_yh < SCREENHEIGHT,
            "R_DrawColumn: {} to {} at {}",
            self.dc_yl,
            self.dc_yh,
            self.dc_x
        );

        let translation_row = self
            .dc_translation
            .expect("R_DrawTranslatedColumn: dc_translation not set");
        let translationtables = self
            .translationtables
            .as_ref()
            .expect("R_DrawTranslatedColumn: translation tables not initialized");
        let translation = &translationtables[translation_row];

        let mut dest =
            (self.ylookup[self.dc_yl as usize] + self.columnofs[self.dc_x as usize]) as usize;

        let fracstep = self.dc_iscale;
        let mut frac = self
            .dc_texturemid
            .wrapping_add((self.dc_yl - self.centery).wrapping_mul(fracstep));

        let colormap = &colormaps[self.dc_colormap * 256..self.dc_colormap * 256 + 256];

        let mut count = count;
        loop {
            // Translation tables are used to map certain colorramps to
            // other ones, used with PLAY sprites. Thus the "green" ramp
            // of the player 0 sprite is mapped to gray, red,
            // black/indigo (the original's own comment, preserved).
            let src_index = translation[texel(source, (frac >> FRACBITS) as usize) as usize];
            screen[dest] = colormap[src_index as usize];
            dest += SCREENWIDTH as usize;
            frac = frac.wrapping_add(fracstep);

            if count == 0 {
                break;
            }
            count -= 1;
        }
    }

    /// Port of `R_DrawSpan`. With DOOM style restrictions on view
    /// orientation, the floors and ceilings consist of horizontal
    /// slices or spans with constant z depth. However, rotation around
    /// the world z axis is possible, thus this mapping, while simpler
    /// and faster than perspective correct texture mapping, has to
    /// traverse the texture at an angle in all but a few cases. In
    /// consequence, flats are not stored by column (like walls), and
    /// the inner loop has to step in texture space u and v (the
    /// original's own comment, preserved).
    ///
    /// `source` is the 64*64 flat tile (`ds_source`).
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on out-of-range `ds_x1`/
    /// `ds_x2`/`ds_y`, same `#ifdef RANGECHECK` guard as the original.
    pub fn r_draw_span(&self, screen: &mut [u8], colormaps: &[u8], source: &[u8]) {
        assert!(
            self.ds_x2 >= self.ds_x1
                && self.ds_x1 >= 0
                && self.ds_x2 < SCREENWIDTH
                && (self.ds_y as u32) <= SCREENHEIGHT as u32,
            "R_DrawSpan: {} to {} at {}",
            self.ds_x1,
            self.ds_x2,
            self.ds_y
        );

        let mut xfrac = self.ds_xfrac;
        let mut yfrac = self.ds_yfrac;

        let mut dest =
            (self.ylookup[self.ds_y as usize] + self.columnofs[self.ds_x1 as usize]) as usize;

        let colormap = &colormaps[self.ds_colormap * 256..self.ds_colormap * 256 + 256];

        // We do not check for zero spans here? (the original's own
        // comment, preserved)
        let mut count = self.ds_x2 - self.ds_x1;
        loop {
            // Current texture index in u,v (the original's own comment,
            // preserved).
            let spot = (((yfrac >> (16 - 6)) & (63 * 64)) + ((xfrac >> 16) & 63)) as usize;

            // Lookup pixel from flat texture tile, re-index using
            // light/colormap (the original's own comment, preserved).
            screen[dest] = colormap[source[spot] as usize];
            dest += 1;

            xfrac = xfrac.wrapping_add(self.ds_xstep);
            yfrac = yfrac.wrapping_add(self.ds_ystep);

            if count == 0 {
                break;
            }
            count -= 1;
        }
    }

    /// Port of `R_DrawSpanLow`. Again.. (the original's own comment,
    /// preserved). Low resolution/blocky mode: writes each sampled
    /// pixel twice.
    ///
    /// # Panics
    /// Same as [`RDraw::r_draw_span`].
    pub fn r_draw_span_low(&mut self, screen: &mut [u8], colormaps: &[u8], source: &[u8]) {
        assert!(
            self.ds_x2 >= self.ds_x1
                && self.ds_x1 >= 0
                && self.ds_x2 < SCREENWIDTH
                && (self.ds_y as u32) <= SCREENHEIGHT as u32,
            "R_DrawSpan: {} to {} at {}",
            self.ds_x1,
            self.ds_x2,
            self.ds_y
        );

        let mut xfrac = self.ds_xfrac;
        let mut yfrac = self.ds_yfrac;

        // Blocky mode, need to multiply by 2 (the original's own
        // comment, preserved).
        self.ds_x1 <<= 1;
        self.ds_x2 <<= 1;

        let mut dest =
            (self.ylookup[self.ds_y as usize] + self.columnofs[self.ds_x1 as usize]) as usize;

        let colormap = &colormaps[self.ds_colormap * 256..self.ds_colormap * 256 + 256];

        let mut count = self.ds_x2 - self.ds_x1;
        loop {
            let spot = (((yfrac >> (16 - 6)) & (63 * 64)) + ((xfrac >> 16) & 63)) as usize;
            // Lowres/blocky mode does it twice, while scale is adjusted
            // appropriately (the original's own comment, preserved).
            let pixel = colormap[source[spot] as usize];
            screen[dest] = pixel;
            screen[dest + 1] = pixel;
            dest += 2;

            xfrac = xfrac.wrapping_add(self.ds_xstep);
            yfrac = yfrac.wrapping_add(self.ds_ystep);

            if count == 0 {
                break;
            }
            count -= 1;
        }
    }

    /// Port of `R_FillBackScreen`. Fills the back screen (`screens[1]`)
    /// with a flat and the border patches around the view window, so
    /// `R_VideoErase`/`R_DrawViewBorder` can restore it later. The flat
    /// is `FLOOR7_2` (`GRNROCK` in DOOM II).
    pub fn r_fill_back_screen(&self, v: &mut VVideo, wad: &mut WadFiles, gamemode: GameMode) {
        // If we are running full screen, there is no need to do any of
        // this, and the background buffer can be used as the wipe
        // screen (the original's own comment).
        if self.scaledviewwidth == 320 {
            return;
        }

        let name = if gamemode == GameMode::Commercial {
            "GRNROCK"
        } else {
            "FLOOR7_2"
        };

        let src = wad.cache_lump_name(name, PurgeTag::Cache).to_vec();
        let sw = SCREENWIDTH as usize;
        let mut dest = 0usize;

        for y in 0..(SCREENHEIGHT - SBARHEIGHT) as usize {
            let row = &src[(y & 63) << 6..((y & 63) << 6) + 64];
            for _ in 0..sw / 64 {
                v.screens[1][dest..dest + 64].copy_from_slice(row);
                dest += 64;
            }
            if sw & 63 != 0 {
                v.screens[1][dest..dest + (sw & 63)].copy_from_slice(&row[..sw & 63]);
                dest += sw & 63;
            }
        }

        let (vx, vy, vw, vh) = (
            self.viewwindowx,
            self.viewwindowy,
            self.scaledviewwidth,
            self.viewheight,
        );
        let mut patch = |name: &str, x: i32, y: i32| {
            let p = wad.cache_lump_name(name, PurgeTag::Cache).to_vec();
            v.v_draw_patch(x, y, 1, &p);
        };

        for x in (0..vw).step_by(8) {
            patch("brdr_t", vx + x, vy - 8);
        }
        for x in (0..vw).step_by(8) {
            patch("brdr_b", vx + x, vy + vh);
        }
        for y in (0..vh).step_by(8) {
            patch("brdr_l", vx - 8, vy + y);
        }
        for y in (0..vh).step_by(8) {
            patch("brdr_r", vx + vw, vy + y);
        }

        // Draw beveled edge.
        patch("brdr_tl", vx - 8, vy - 8);
        patch("brdr_tr", vx + vw, vy - 8);
        patch("brdr_bl", vx - 8, vy + vh);
        patch("brdr_br", vx + vw, vy + vh);
    }

    /// Port of `R_DrawViewBorder`. Draws the border around the view for
    /// different size windows, by copying it from the back screen.
    pub fn r_draw_view_border(&self, v: &mut VVideo) {
        if self.scaledviewwidth == SCREENWIDTH {
            return;
        }

        let sw = SCREENWIDTH as usize;
        let top = (((SCREENHEIGHT - SBARHEIGHT) - self.viewheight) / 2) as usize;
        let mut side = ((SCREENWIDTH - self.scaledviewwidth) / 2) as usize;
        let viewheight = self.viewheight as usize;

        // copy top and one line of left side
        self.r_video_erase(v, 0, top * sw + side);

        // copy one line of right side and bottom
        let mut ofs = (viewheight + top) * sw - side;
        self.r_video_erase(v, ofs, top * sw + side);

        // copy sides using wraparound
        ofs = top * sw + sw - side;
        side <<= 1;

        for _ in 1..viewheight {
            self.r_video_erase(v, ofs, side);
            ofs += sw;
        }

        // ?
        v.v_mark_rect(0, 0, SCREENWIDTH, SCREENHEIGHT - SBARHEIGHT);
    }

    /// Port of `R_VideoErase`. Copy a screen buffer (the original's own
    /// comment, preserved) — copies `count` bytes from `screens[1]` to
    /// `screens[0]` at offset `ofs`.
    pub fn r_video_erase(&self, v: &mut VVideo, ofs: usize, count: usize) {
        let (screen0, screen1) = v.screens.split_at_mut(1);
        screen0[0][ofs..ofs + count].copy_from_slice(&screen1[0][ofs..ofs + count]);
    }
}

/// Port of `R_InitTranslationTables`. Creates the translation tables to
/// map the green color ramp to gray, brown, red. Assumes a given
/// structure of the PLAYPAL. Could be read from a lump instead (the
/// original's own comment, preserved).
///
/// Returns the table directly (rather than writing through `&mut
/// RDraw`) since it has no dependency on any other `RDraw` field —
/// callers assign it to [`RDraw::translationtables`].
pub fn r_init_translation_tables() -> [[u8; 256]; 3] {
    let mut tables = [[0u8; 256]; 3];

    #[allow(clippy::needless_range_loop)] // writes all three sub-tables by the same index i
    for i in 0..256usize {
        if (0x70..=0x7f).contains(&i) {
            // map green ramp to gray, brown, red (the original's own
            // comment, preserved)
            tables[0][i] = (0x60 + (i & 0xf)) as u8;
            tables[1][i] = (0x40 + (i & 0xf)) as u8;
            tables[2][i] = (0x20 + (i & 0xf)) as u8;
        } else {
            // Keep all other colors as is (the original's own comment,
            // preserved).
            tables[0][i] = i as u8;
            tables[1][i] = i as u8;
            tables[2][i] = i as u8;
        }
    }

    tables
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v_video::VVideo;

    fn flat_colormaps(levels: usize) -> Vec<u8> {
        // Identity colormap: colormap[level][c] == c, so drawn pixels
        // equal the raw source index directly (easy to assert on).
        let mut cm = vec![0u8; levels * 256];
        for level in 0..levels {
            for c in 0..256 {
                cm[level * 256 + c] = c as u8;
            }
        }
        cm
    }

    #[test]
    fn r_init_buffer_matches_full_screen_layout() {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);

        assert_eq!(rdraw.viewwindowx, 0);
        assert_eq!(rdraw.viewwindowy, 0);
        assert_eq!(rdraw.columnofs.len(), SCREENWIDTH as usize);
        assert_eq!(rdraw.columnofs[0], 0);
        assert_eq!(rdraw.columnofs[10], 10);
        assert_eq!(rdraw.ylookup.len(), SCREENHEIGHT as usize);
        assert_eq!(rdraw.ylookup[0], 0);
        assert_eq!(rdraw.ylookup[5], 5 * SCREENWIDTH);
    }

    #[test]
    fn r_draw_column_writes_expected_pixels() {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        rdraw.dc_x = 10;
        rdraw.dc_yl = 5;
        rdraw.dc_yh = 8;
        rdraw.dc_iscale = FRACUNIT_TEST;
        rdraw.dc_texturemid = 0;
        rdraw.dc_colormap = 0;

        let colormaps = flat_colormaps(1);
        let source = [42u8; 128];
        let mut screen = vec![0u8; (SCREENWIDTH * SCREENHEIGHT) as usize];

        rdraw.r_draw_column(&mut screen, &colormaps, &source);

        for y in 5..=8 {
            let off = (rdraw.ylookup[y] + rdraw.columnofs[10]) as usize;
            assert_eq!(screen[off], 42, "row {y} not written");
        }
    }

    #[test]
    #[should_panic(expected = "R_DrawColumn")]
    fn r_draw_column_panics_out_of_range_x() {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        rdraw.dc_x = SCREENWIDTH; // one past valid range
        rdraw.dc_yl = 0;
        rdraw.dc_yh = 0;

        let colormaps = flat_colormaps(1);
        let source = [0u8; 128];
        let mut screen = vec![0u8; (SCREENWIDTH * SCREENHEIGHT) as usize];
        rdraw.r_draw_column(&mut screen, &colormaps, &source);
    }

    #[test]
    fn r_draw_column_zero_length_is_noop() {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        rdraw.dc_x = 0;
        rdraw.dc_yl = 5;
        rdraw.dc_yh = 4; // count < 0
        let colormaps = flat_colormaps(1);
        let source = [9u8; 128];
        let mut screen = vec![7u8; (SCREENWIDTH * SCREENHEIGHT) as usize];
        rdraw.r_draw_column(&mut screen, &colormaps, &source);
        assert!(screen.iter().all(|&b| b == 7));
    }

    #[test]
    fn r_draw_column_low_writes_two_adjacent_columns() {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        rdraw.dc_x = 5;
        rdraw.dc_yl = 0;
        rdraw.dc_yh = 0;
        rdraw.dc_iscale = FRACUNIT_TEST;
        rdraw.dc_texturemid = 0;
        rdraw.dc_colormap = 0;

        let colormaps = flat_colormaps(1);
        let source = [77u8; 128];
        let mut screen = vec![0u8; (SCREENWIDTH * SCREENHEIGHT) as usize];

        rdraw.r_draw_column_low(&mut screen, &colormaps, &source);

        // dc_x is doubled in place by the original algorithm: columns
        // 10 and 11 both get written.
        assert_eq!(screen[rdraw.columnofs[10] as usize], 77);
        assert_eq!(screen[rdraw.columnofs[11] as usize], 77);
    }

    #[test]
    fn r_draw_span_writes_expected_row() {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        rdraw.ds_y = 3;
        rdraw.ds_x1 = 0;
        rdraw.ds_x2 = 4;
        rdraw.ds_xfrac = 0;
        rdraw.ds_yfrac = 0;
        rdraw.ds_xstep = FRACUNIT_TEST;
        rdraw.ds_ystep = 0;
        rdraw.ds_colormap = 0;

        let colormaps = flat_colormaps(1);
        let mut source = [0u8; 64 * 64];
        source[0] = 5;
        source[1] = 6;
        let mut screen = vec![0u8; (SCREENWIDTH * SCREENHEIGHT) as usize];

        rdraw.r_draw_span(&mut screen, &colormaps, &source);

        let row_start = rdraw.ylookup[3] as usize;
        assert_eq!(screen[row_start], 5);
        assert_eq!(screen[row_start + 1], 6);
    }

    #[test]
    fn r_video_erase_copies_from_screen1_to_screen0() {
        let mut v = VVideo::new();
        v.screens[1][100] = 55;
        v.screens[1][101] = 56;

        let rdraw = RDraw::new();
        rdraw.r_video_erase(&mut v, 100, 2);

        assert_eq!(v.screens[0][100], 55);
        assert_eq!(v.screens[0][101], 56);
    }

    #[test]
    fn r_init_translation_tables_maps_green_ramp() {
        let tables = r_init_translation_tables();
        // 0x70..=0x7f maps to gray/brown/red ramps.
        assert_eq!(tables[0][0x70], 0x60);
        assert_eq!(tables[0][0x7f], 0x6f);
        assert_eq!(tables[1][0x70], 0x40);
        assert_eq!(tables[2][0x70], 0x20);
        // Everything else is identity.
        assert_eq!(tables[0][0x10], 0x10);
        assert_eq!(tables[1][0x10], 0x10);
        assert_eq!(tables[2][0x10], 0x10);
    }

    #[test]
    fn r_draw_fuzz_column_reads_neighbor_pixel_through_colormap6() {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        rdraw.dc_x = 10;
        rdraw.dc_yl = 5;
        rdraw.dc_yh = 5;
        rdraw.fuzzpos = 0; // FUZZOFFSET[0] == 1 (one row down)

        // colormap row 6 as identity so the written pixel equals
        // whatever's read from the neighboring row.
        let colormaps = flat_colormaps(7);
        let mut screen = vec![0u8; (SCREENWIDTH * SCREENHEIGHT) as usize];
        let neighbor_off = (rdraw.ylookup[5] + rdraw.columnofs[10]) as usize + SCREENWIDTH as usize;
        screen[neighbor_off] = 33;

        rdraw.r_draw_fuzz_column(&mut screen, &colormaps, SCREENHEIGHT);

        let dest_off = (rdraw.ylookup[5] + rdraw.columnofs[10]) as usize;
        assert_eq!(screen[dest_off], 33);
        // fuzzpos advanced by one entry.
        assert_eq!(rdraw.fuzzpos, 1);
    }

    #[test]
    fn r_draw_translated_column_applies_translation_then_colormap() {
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        rdraw.dc_x = 0;
        rdraw.dc_yl = 0;
        rdraw.dc_yh = 0;
        rdraw.dc_iscale = FRACUNIT_TEST;
        // Cancel out the (dc_yl - CENTERY) * fracstep term so frac == 0
        // at dc_yl == 0, landing squarely on source[0].
        rdraw.dc_texturemid = CENTERY * FRACUNIT_TEST;
        rdraw.dc_colormap = 0;
        rdraw.dc_translation = Some(0);
        rdraw.translationtables = Some(r_init_translation_tables());

        let colormaps = flat_colormaps(1);
        let mut source = [0u8; 1];
        source[0] = 0x75; // within the green ramp -> translated to 0x65
        let mut screen = vec![0u8; (SCREENWIDTH * SCREENHEIGHT) as usize];

        rdraw.r_draw_translated_column(&mut screen, &colormaps, &source);

        assert_eq!(screen[0], 0x65);
    }

    /// `FRACUNIT` re-exported locally to keep this test module's
    /// intent readable without a wildcard import colliding with
    /// `crate::m_fixed::FRACUNIT`'s type (`Fixed` == `i32`).
    const FRACUNIT_TEST: Fixed = crate::m_fixed::FRACUNIT;
}
