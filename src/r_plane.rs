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
//	Here is a core component: drawing the floors and ceilings,
//	 while maintaining a per column clipping list only.
//	Moreover, the sky areas have to be determined.
//
//-----------------------------------------------------------------------------

//! Rust port of `r_plane.h` / `r_plane.c` (partial, see note
//! below). Here is a core component: drawing the floors and ceilings,
//! while maintaining a per column clipping list only. Moreover, the sky
//! areas have to be determined (the original's own comment, preserved).
//!
//! # Scope
//!
//! Ported: [`RPlane::r_clear_planes`] (`R_ClearPlanes`),
//! [`RPlane::r_find_plane`] (`R_FindPlane`), [`RPlane::r_check_plane`]
//! (`R_CheckPlane`), [`RPlane::r_map_plane`] (`R_MapPlane`),
//! [`RPlane::r_draw_planes`] (`R_DrawPlanes`, which folds in
//! `R_MakeSpans`'s logic directly — see that method's docs).
//! `R_InitPlanes` is a documented no-op in the original itself ("Doh!")
//! — not reproduced as a function here since there's nothing to call.
//!
//! Sky visplanes are drawn as columns of the sky texture, as in the
//! original (with [`crate::r_sky::Sky`] standing in for `r_sky.c`'s
//! globals).
//!
//! # Representation choices
//!
//! `extralight`/`fixedcolormap` (player state — `player_t` is Phase 6,
//! not ported, per `d_player.rs`'s note) are passed as explicit
//! parameters to [`RPlane::r_draw_planes`]/[`RPlane::r_map_plane`]
//! rather than read off a global, same pattern `r_main.rs`'s
//! [`crate::r_main::RMain::scale_from_global_angle`] already uses for
//! not-yet-ported state. `detailshift`/`colfunc`/`spanfunc`
//! (`R_ExecuteSetViewSize`'s function-pointer selection) are
//! likewise not modeled — [`RPlane::r_draw_planes`] always draws
//! through [`crate::r_draw::RDraw::r_draw_span`]/
//! [`crate::r_draw::RDraw::r_draw_column`] (the "high detail"
//! variants), leaving low-detail selection for whichever later phase
//! wires up detail-level selection.
//!
//! `planezlight`/`ds_colormap` become an index into
//! [`crate::r_main::RMain::zlight`] (already index-based, see that
//! struct's own docs) rather than a `lighttable_t**`/`lighttable_t*`
//! pair — one level of indirection collapses since both ends were
//! already indices in this port.
//!
//! `yslope[SCREENHEIGHT]`/`distscale[SCREENWIDTH]`/`centerxfrac`
//! (`R_ExecuteSetViewSize`'s precomputed view-size tables, which
//! `r_main.rs`'s port doesn't cache) are recomputed on demand by private helpers
//! ([`yslope`]/[`distscale`]/[`centerxfrac`]) from `viewwidth`/
//! `viewheight`/`xtoviewangle` instead of being cached fields — see
//! those functions' own docs.
//!
//! `visplanes`/`lastvisplane` become a growable `Vec<VisPlane>`
//! ([`RPlane::visplanes`]) instead of a fixed `[MAXVISPLANES]` array
//! plus a "one past the last used" pointer — `MAXVISPLANES` is kept as
//! a documented original limit (checked in [`RPlane::r_find_plane`],
//! matching the original's `I_Error` guard) rather than a hard
//! preallocation size. `openings`/`lastopening` (the original's shared
//! scratch buffer for `r_segs.c`'s `maskedtexturecol` and sprite-clip
//! snapshots, read back by `r_things.c`) become the growable
//! [`RPlane::openings`] `Vec<i32>`, cleared by [`RPlane::r_clear_planes`]
//! (the original's `lastopening = openings`) — same treatment as
//! `visplanes`, with [`MAXOPENINGS`] kept as the checked limit.
//!
//! `top[minx-1]`/`top[maxx+1]` out-of-bounds writes (a real, intentional
//! overrun in the original, backed by padding bytes in `visplane_t`) are
//! handled via [`crate::r_defs::VisPlane`]'s own `+1`-offset accessors —
//! see that struct's docs.

use crate::doomdef::{SCREENHEIGHT, SCREENWIDTH};
use crate::m_fixed::{fixed_div, fixed_mul, Fixed, FRACBITS, FRACUNIT};
use crate::r_data::RData;
use crate::r_defs::VisPlane;
use crate::r_draw::RDraw;
use crate::r_main::{RMain, LIGHTLEVELS, LIGHTSEGSHIFT, LIGHTZSHIFT, MAXLIGHTZ};
use crate::r_sky::{Sky, ANGLETOSKYSHIFT};
use crate::r_state::RState;
use crate::tables::{fine_cosine, ANG90, ANGLETOFINESHIFT, FINESINE};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// (`MAXVISPLANES`) — original's fixed cap on simultaneous visplanes
/// per frame, kept as a documented/enforced limit (see module docs)
/// rather than a preallocation size.
pub const MAXVISPLANES: usize = 128;

/// Marker value for [`VisPlane`] `top`/`bottom` entries meaning "no
/// span at this column yet" (`0xff` in the original's `byte` arrays).
pub const PLANE_UNSET: u8 = 0xff;

/// (`MAXOPENINGS`) — original's fixed size of `openings[]`, kept as the
/// limit checked by [`RPlane::r_draw_planes`] (same as the original's
/// `R_DrawPlanes` overflow guard) rather than a preallocation size.
pub const MAXOPENINGS: usize = SCREENWIDTH as usize * 64;

/// Port of the `r_plane` module's per-frame visplane/span-clipping
/// state (`visplanes`/`lastvisplane`, `floorclip`/`ceilingclip`,
/// `spanstart`/`spanstop`, `planeheight`, `basexscale`/`baseyscale`,
/// `cachedheight`/`cacheddistance`/`cachedxstep`/`cachedystep`,
/// `planezlight`). See module docs for representation choices.
pub struct RPlane {
    /// (`visplanes[MAXVISPLANES]`/`lastvisplane`) — see module docs.
    pub visplanes: Vec<VisPlane>,

    /// (`floorclip[SCREENWIDTH]`) — solid pixel bounding the range,
    /// starts out `viewheight` (the original's own comment, preserved).
    pub floorclip: Vec<i32>,
    /// (`ceilingclip[SCREENWIDTH]`) — starts out `-1`.
    pub ceilingclip: Vec<i32>,

    /// (`spanstart[SCREENHEIGHT]`) — holds the start of a plane span,
    /// initialized to 0 at start (the original's own comment,
    /// preserved).
    pub spanstart: Vec<i32>,
    pub spanstop: Vec<i32>,

    /// (`planezlight`) — index of the light-level row currently active
    /// in [`crate::r_main::RMain::zlight`] (see module docs on why this
    /// collapses one level of indirection versus the original).
    pub planezlight: usize,
    pub planeheight: Fixed,

    pub basexscale: Fixed,
    pub baseyscale: Fixed,

    pub cachedheight: Vec<Fixed>,
    pub cacheddistance: Vec<Fixed>,
    pub cachedxstep: Vec<Fixed>,
    pub cachedystep: Vec<Fixed>,

    /// (`openings[MAXOPENINGS]`/`lastopening`) — see module docs.
    /// `openings.len()` is the original's `lastopening - openings`.
    /// Written by `r_segs.rs`, read by `r_segs.rs`/`r_things.rs`
    /// through [`crate::r_defs::DrawSeg`]'s indices into it.
    pub openings: Vec<i32>,
}

impl RPlane {
    pub fn new() -> Self {
        RPlane {
            visplanes: Vec::new(),
            floorclip: vec![0; SCREENWIDTH as usize],
            ceilingclip: vec![0; SCREENWIDTH as usize],
            spanstart: vec![0; SCREENHEIGHT as usize],
            spanstop: vec![0; SCREENHEIGHT as usize],
            planezlight: 0,
            planeheight: 0,
            basexscale: 0,
            baseyscale: 0,
            cachedheight: vec![0; SCREENHEIGHT as usize],
            cacheddistance: vec![0; SCREENHEIGHT as usize],
            cachedxstep: vec![0; SCREENHEIGHT as usize],
            cachedystep: vec![0; SCREENHEIGHT as usize],
            openings: Vec::new(),
        }
    }

    /// Port of `R_ClearPlanes`. At beginning of frame (the original's
    /// own comment, preserved).
    pub fn r_clear_planes(&mut self, rmain: &RMain, viewwidth: i32, viewheight: i32) {
        for i in 0..viewwidth as usize {
            self.floorclip[i] = viewheight;
            self.ceilingclip[i] = -1;
        }

        self.visplanes.clear();
        self.openings.clear();

        // texture calculation (the original's own comment, preserved)
        self.cachedheight.iter_mut().for_each(|h| *h = 0);

        // left to right mapping (the original's own comment, preserved)
        let angle = ((rmain.viewangle.wrapping_sub(ANG90)) >> ANGLETOFINESHIFT) as usize;

        // scale will be unit scale at SCREENWIDTH/2 distance (the
        // original's own comment, preserved)
        let cxf = centerxfrac(viewwidth);
        self.basexscale = fixed_div(fine_cosine(angle), cxf);
        self.baseyscale = -fixed_div(FINESINE[angle], cxf);
    }

    /// Port of `R_FindPlane`.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) once [`MAXVISPLANES`] would be
    /// exceeded, same as the original.
    pub fn r_find_plane(
        &mut self,
        mut height: Fixed,
        picnum: i32,
        mut lightlevel: i32,
        skyflatnum: i32,
    ) -> usize {
        if picnum == skyflatnum {
            // all skys map together (the original's own comment,
            // preserved)
            height = 0;
            lightlevel = 0;
        }

        for (i, check) in self.visplanes.iter().enumerate() {
            if height == check.height && picnum == check.picnum && lightlevel == check.lightlevel {
                return i;
            }
        }

        assert!(
            self.visplanes.len() < MAXVISPLANES,
            "R_FindPlane: no more visplanes"
        );

        let mut plane = VisPlane::new(SCREENWIDTH as usize);
        plane.height = height;
        plane.picnum = picnum;
        plane.lightlevel = lightlevel;
        plane.minx = SCREENWIDTH;
        plane.maxx = -1;
        plane.reset_top();

        self.visplanes.push(plane);
        self.visplanes.len() - 1
    }

    /// Port of `R_CheckPlane`. Returns the index of the visplane to use
    /// (either `pl` itself, extended, or a freshly split-off copy —
    /// same "same one or a new one" contract as the original, indices
    /// instead of pointers).
    pub fn r_check_plane(&mut self, pl: usize, start: i32, stop: i32) -> usize {
        let (unionl, intrl) = if start < self.visplanes[pl].minx {
            (start, self.visplanes[pl].minx)
        } else {
            (self.visplanes[pl].minx, start)
        };

        let (unionh, intrh) = if stop > self.visplanes[pl].maxx {
            (stop, self.visplanes[pl].maxx)
        } else {
            (self.visplanes[pl].maxx, stop)
        };

        let mut x = intrl;
        while x <= intrh && self.visplanes[pl].top_at(x) == PLANE_UNSET {
            x += 1;
        }

        if x > intrh {
            self.visplanes[pl].minx = unionl;
            self.visplanes[pl].maxx = unionh;
            // use the same one (the original's own comment, preserved)
            return pl;
        }

        // make a new visplane (the original's own comment, preserved)
        let mut new_plane = VisPlane::new(SCREENWIDTH as usize);
        new_plane.height = self.visplanes[pl].height;
        new_plane.picnum = self.visplanes[pl].picnum;
        new_plane.lightlevel = self.visplanes[pl].lightlevel;
        new_plane.minx = start;
        new_plane.maxx = stop;
        new_plane.reset_top();

        self.visplanes.push(new_plane);
        self.visplanes.len() - 1
    }

    /// Port of `R_MapPlane`. Uses global vars: `planeheight`,
    /// `ds_source`, `basexscale`, `baseyscale`, `viewx`, `viewy` (the
    /// original's own comment, preserved). BASIC PRIMITIVE (the
    /// original's own comment, preserved).
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) on out-of-range `x1`/`x2`/`y`,
    /// same `#ifdef RANGECHECK` guard as the original.
    #[allow(clippy::too_many_arguments)]
    pub fn r_map_plane(
        &mut self,
        rdraw: &mut RDraw,
        rmain: &RMain,
        rstate: &RState,
        y: i32,
        x1: i32,
        x2: i32,
        viewwidth: i32,
        viewheight: i32,
        fixedcolormap: Option<usize>,
    ) {
        assert!(
            x2 >= x1 && x1 >= 0 && x2 < viewwidth && (y as u32) <= viewheight as u32,
            "R_MapPlane: {x1}, {x2} at {y}"
        );

        let distance;
        if self.planeheight != self.cachedheight[y as usize] {
            self.cachedheight[y as usize] = self.planeheight;
            distance = fixed_mul(self.planeheight, yslope(y, viewheight, viewwidth));
            self.cacheddistance[y as usize] = distance;
            rdraw.ds_xstep = fixed_mul(distance, self.basexscale);
            rdraw.ds_ystep = fixed_mul(distance, self.baseyscale);
            self.cachedxstep[y as usize] = rdraw.ds_xstep;
            self.cachedystep[y as usize] = rdraw.ds_ystep;
        } else {
            distance = self.cacheddistance[y as usize];
            rdraw.ds_xstep = self.cachedxstep[y as usize];
            rdraw.ds_ystep = self.cachedystep[y as usize];
        }

        let length = fixed_mul(distance, distscale(x1, rstate));
        let angle = ((rmain
            .viewangle
            .wrapping_add(rstate.xtoviewangle[x1 as usize]))
            >> ANGLETOFINESHIFT) as usize;
        rdraw.ds_xfrac = rmain
            .viewx
            .wrapping_add(fixed_mul(fine_cosine(angle), length));
        rdraw.ds_yfrac = (-rmain.viewy).wrapping_sub(fixed_mul(FINESINE[angle], length));

        rdraw.ds_colormap = match fixedcolormap {
            Some(cm) => cm,
            None => {
                let mut index = (distance >> LIGHTZSHIFT) as usize;
                if index >= MAXLIGHTZ {
                    index = MAXLIGHTZ - 1;
                }
                rmain.zlight[self.planezlight][index] as usize
            }
        };

        rdraw.ds_y = y;
        rdraw.ds_x1 = x1;
        rdraw.ds_x2 = x2;
    }

    /// Port of `R_DrawPlanes`. At the end of each frame (the original's
    /// own comment, preserved).
    ///
    /// Folds `R_MakeSpans`'s logic directly into this method's per-plane
    /// loop (rather than as a separate function) since every one of its
    /// calls to `R_MapPlane` needs the `RDraw`/`RMain`/`RData` context
    /// this method already threads through — splitting it out would
    /// just re-pass the same six parameters through an extra layer.
    ///
    /// `extralight`/`fixedcolormap` stand in for the not-yet-ported
    /// player-state globals — see module docs.
    #[allow(clippy::too_many_arguments)]
    pub fn r_draw_planes(
        &mut self,
        rdraw: &mut RDraw,
        rmain: &RMain,
        rstate: &RState,
        rdata: &mut RData,
        wad: &mut WadFiles,
        screen: &mut [u8],
        viewwidth: i32,
        viewheight: i32,
        extralight: i32,
        fixedcolormap: Option<usize>,
        sky: &Sky,
    ) {
        // Only the original's openings overflow check is reproduced
        // here: visplanes is already capped in `r_find_plane`, and
        // drawsegs live in `r_bsp.rs`'s `RBsp`, which this method
        // doesn't see (and which documents `MAXDRAWSEGS` as unenforced).
        assert!(
            self.openings.len() <= MAXOPENINGS,
            "R_DrawPlanes: opening overflow ({})",
            self.openings.len()
        );

        for i in 0..self.visplanes.len() {
            let (minx, maxx, picnum, height, lightlevel) = {
                let pl = &self.visplanes[i];
                (pl.minx, pl.maxx, pl.picnum, pl.height, pl.lightlevel)
            };

            if minx > maxx {
                continue;
            }

            // sky flat (the original's own comment, preserved)
            if picnum == sky.skyflatnum {
                // `pspriteiscale >> detailshift`, for a high-detail view
                // (see module docs).
                rdraw.dc_iscale = FRACUNIT * SCREENWIDTH / viewwidth;

                // Sky is allways drawn full bright, i.e. colormaps[0] is
                // used. Because of this hack, sky is not affected by
                // INVUL inverse mapping (the original's own comment,
                // preserved).
                rdraw.dc_colormap = 0;
                rdraw.dc_texturemid = sky.skytexturemid;
                for x in minx..=maxx {
                    let (top, bottom) = {
                        let pl = &self.visplanes[i];
                        (pl.top_at(x) as i32, pl.bottom_at(x) as i32)
                    };
                    rdraw.dc_yl = top;
                    rdraw.dc_yh = bottom;

                    if rdraw.dc_yl <= rdraw.dc_yh {
                        let angle = rmain
                            .viewangle
                            .wrapping_add(rstate.xtoviewangle[x as usize])
                            >> ANGLETOSKYSHIFT;
                        rdraw.dc_x = x;
                        let source = rdata.get_column(wad, sky.skytexture as usize, angle as i32);
                        rdraw.r_draw_column(screen, &rdata.colormaps, &source);
                    }
                }
                continue;
            }

            // regular flat (the original's own comment, preserved)
            let flat_lump = (rdata.firstflat + rdata.flattranslation[picnum as usize]) as usize;
            let flat = wad.cache_lump_num(flat_lump, PurgeTag::Static).to_vec();

            self.planeheight = (height - rmain.viewz).abs();
            let mut light = (lightlevel >> LIGHTSEGSHIFT) + extralight;
            if light >= LIGHTLEVELS as i32 {
                light = LIGHTLEVELS as i32 - 1;
            }
            if light < 0 {
                light = 0;
            }
            self.planezlight = light as usize;

            {
                let pl = &mut self.visplanes[i];
                pl.set_top_at(pl.maxx + 1, PLANE_UNSET);
                pl.set_top_at(pl.minx - 1, PLANE_UNSET);
            }

            let stop = maxx + 1;
            for x in minx..=stop {
                let (mut t1, mut b1, t2, mut b2) = {
                    let pl = &self.visplanes[i];
                    (
                        pl.top_at(x - 1) as i32,
                        pl.bottom_at(x - 1) as i32,
                        pl.top_at(x) as i32,
                        pl.bottom_at(x) as i32,
                    )
                };

                while t1 < t2 && t1 <= b1 {
                    self.r_map_plane(
                        rdraw,
                        rmain,
                        rstate,
                        t1,
                        self.spanstart[t1 as usize],
                        x - 1,
                        viewwidth,
                        viewheight,
                        fixedcolormap,
                    );
                    rdraw.r_draw_span(screen, &rdata.colormaps, &flat);
                    t1 += 1;
                }
                while b1 > b2 && b1 >= t1 {
                    self.r_map_plane(
                        rdraw,
                        rmain,
                        rstate,
                        b1,
                        self.spanstart[b1 as usize],
                        x - 1,
                        viewwidth,
                        viewheight,
                        fixedcolormap,
                    );
                    rdraw.r_draw_span(screen, &rdata.colormaps, &flat);
                    b1 -= 1;
                }

                let mut t2v = t2;
                while t2v < t1 && t2v <= b2 {
                    self.spanstart[t2v as usize] = x;
                    t2v += 1;
                }
                while b2 > b1 && b2 >= t2v {
                    self.spanstart[b2 as usize] = x;
                    b2 -= 1;
                }
            }

            // Z_ChangeTag(ds_source, PU_CACHE) (the original's own
            // call) — marks the flat purgeable again now that this
            // plane is done reading it. `cache_lump_num` re-applies the
            // tag on an already-cached lump as part of its own cache-hit
            // path (see that method's docs), so calling it again with
            // `PurgeTag::Cache` reproduces the same effect without a
            // separate change-tag entry point.
            wad.cache_lump_num(flat_lump, PurgeTag::Cache);
        }
    }
}

impl Default for RPlane {
    fn default() -> Self {
        Self::new()
    }
}

/// (`yslope[y]`) computed on demand from view geometry rather than
/// cached in a persistent `[SCREENHEIGHT]` array set up once by
/// `R_ExecuteSetViewSize` — this recomputes the same formula per call
/// instead (see module docs).
fn yslope(y: i32, viewheight: i32, viewwidth: i32) -> Fixed {
    let dy = (((y - viewheight / 2) << FRACBITS) + FRACUNIT / 2).abs();
    fixed_div((viewwidth / 2) * FRACUNIT, dy)
}

/// (`distscale[x1]`) — see [`yslope`]'s docs for why this is computed
/// on demand instead of cached.
fn distscale(x: i32, rstate: &RState) -> Fixed {
    let angle = (rstate.xtoviewangle[x as usize] >> ANGLETOFINESHIFT) as usize;
    let cosadj = fine_cosine(angle).abs();
    fixed_div(FRACUNIT, cosadj)
}

/// `centerxfrac` for a full-detail view of the given width — see
/// [`yslope`]'s docs on why view-size setup isn't cached here.
fn centerxfrac(viewwidth: i32) -> Fixed {
    (viewwidth / 2) * FRACUNIT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r_find_plane_reuses_matching_visplane() {
        let mut rp = RPlane::new();
        let a = rp.r_find_plane(10, 5, 200, -1);
        let b = rp.r_find_plane(10, 5, 200, -1);
        assert_eq!(a, b);
        assert_eq!(rp.visplanes.len(), 1);
    }

    #[test]
    fn r_find_plane_creates_new_for_different_params() {
        let mut rp = RPlane::new();
        let a = rp.r_find_plane(10, 5, 200, -1);
        let b = rp.r_find_plane(20, 5, 200, -1);
        assert_ne!(a, b);
        assert_eq!(rp.visplanes.len(), 2);
    }

    #[test]
    fn r_find_plane_sky_flat_collapses_height_and_light() {
        let mut rp = RPlane::new();
        let a = rp.r_find_plane(10, 99, 200, 99);
        let b = rp.r_find_plane(999, 99, 1, 99);
        // Both are skyflatnum (99): height/lightlevel forced to 0/0,
        // so they must resolve to the same visplane.
        assert_eq!(a, b);
    }

    #[test]
    fn r_check_plane_reuses_when_columns_unset() {
        let mut rp = RPlane::new();
        let idx = rp.r_find_plane(10, 5, 200, -1);
        let reused = rp.r_check_plane(idx, 0, 10);
        assert_eq!(reused, idx);
        assert_eq!(rp.visplanes[idx].minx, 0);
        assert_eq!(rp.visplanes[idx].maxx, 10);
    }

    #[test]
    fn r_check_plane_splits_when_columns_already_set() {
        let mut rp = RPlane::new();
        let idx = rp.r_find_plane(10, 5, 200, -1);
        rp.visplanes[idx].minx = 0;
        rp.visplanes[idx].maxx = 10;
        rp.visplanes[idx].set_top_at(5, 3); // mark column 5 as already used

        let result = rp.r_check_plane(idx, 0, 10);
        assert_ne!(result, idx);
        assert_eq!(rp.visplanes.len(), 2);
        assert_eq!(rp.visplanes[result].minx, 0);
        assert_eq!(rp.visplanes[result].maxx, 10);
    }

    #[test]
    fn r_clear_planes_resets_clip_arrays() {
        let mut rp = RPlane::new();
        let rmain = RMain::new();
        rp.floorclip[0] = 999;
        rp.ceilingclip[0] = 999;
        rp.r_clear_planes(&rmain, 320, 200);
        assert_eq!(rp.floorclip[0], 200);
        assert_eq!(rp.ceilingclip[0], -1);
        assert!(rp.visplanes.is_empty());
    }

    #[test]
    #[should_panic(expected = "R_FindPlane")]
    fn r_find_plane_panics_past_maxvisplanes() {
        let mut rp = RPlane::new();
        for i in 0..(MAXVISPLANES as i32 + 1) {
            rp.r_find_plane(i as Fixed, i, i, -1);
        }
    }

    #[test]
    fn r_map_plane_sets_span_and_colormap() {
        let mut rp = RPlane::new();
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        let mut rmain = RMain::new();
        rmain.viewangle = 0;
        let rstate = RState::default();

        rp.planeheight = 10 * FRACUNIT;
        rp.basexscale = FRACUNIT;
        rp.baseyscale = FRACUNIT;

        rp.r_map_plane(
            &mut rdraw,
            &rmain,
            &rstate,
            100,
            0,
            10,
            SCREENWIDTH,
            SCREENHEIGHT,
            Some(0),
        );

        assert_eq!(rdraw.ds_y, 100);
        assert_eq!(rdraw.ds_x1, 0);
        assert_eq!(rdraw.ds_x2, 10);
        assert_eq!(rdraw.ds_colormap, 0);
    }

    #[test]
    #[should_panic(expected = "R_MapPlane")]
    fn r_map_plane_panics_out_of_range() {
        let mut rp = RPlane::new();
        let mut rdraw = RDraw::new();
        rdraw.r_init_buffer(SCREENWIDTH, SCREENHEIGHT);
        let rmain = RMain::new();
        let rstate = RState::default();

        rp.r_map_plane(
            &mut rdraw,
            &rmain,
            &rstate,
            0,
            10,
            5, // x2 < x1
            SCREENWIDTH,
            SCREENHEIGHT,
            Some(0),
        );
    }
}
