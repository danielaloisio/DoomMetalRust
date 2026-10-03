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
//	All the clipping: columns, horizontal spans, sky columns.
//
//-----------------------------------------------------------------------------

//! Rust port of `r_segs.h` / `r_segs.c`. All the clipping:
//! columns, horizontal spans, sky columns (the original's own comment,
//! preserved).
//!
//! # Scope
//!
//! Ported: [`RSegs::r_store_wall_range`] (`R_StoreWallRange`, which
//! folds in `R_RenderSegLoop`'s loop body directly — see that method's
//! docs), including its "save sprite clipping info" tail, and
//! [`RSegs::r_render_masked_seg_range`] (`R_RenderMaskedSegRange`),
//! which draws through `r_things.rs`'s
//! [`crate::r_things::r_draw_masked_column`] like the original.
//!
//! Not ported: the `if (ds_p == &drawsegs[MAXDRAWSEGS]) return;` guard
//! at the top of `R_StoreWallRange` — this method returns its
//! [`DrawSeg`] instead of writing through `ds_p`, so the cap belongs to
//! whichever caller pushes onto `RBsp::drawsegs` (see that field's docs).
//!
//! # Representation choices
//!
//! This module's enormous list of module-global scratch variables
//! (`rw_x`/`rw_stopx`/`rw_centerangle`/`rw_offset`/`rw_distance`/
//! `rw_scale`/`rw_scalestep`/`rw_midtexturemid`/`rw_toptexturemid`/
//! `rw_bottomtexturemid`, `worldtop`/`worldbottom`/`worldhigh`/
//! `worldlow`, `pixhigh`/`pixlow`/`pixhighstep`/`pixlowstep`,
//! `topfrac`/`topstep`/`bottomfrac`/`bottomstep`, `segtextured`,
//! `markfloor`/`markceiling`, `midtexture`/`toptexture`/
//! `bottomtexture`/`maskedtexture`, `walllights`) become fields on
//! [`RSegs`], mirroring the plan's globals-become-struct-fields pattern
//! — every one of them is set by `R_StoreWallRange` and consumed within
//! that same call (this port folds `R_RenderSegLoop`'s body directly
//! into `R_StoreWallRange` rather than as a separate method, since
//! nothing else ever calls it and splitting it back out would just
//! re-pass the same half-dozen fields across a method boundary).
//!
//! `walllights` becomes an index into [`crate::r_main::RMain::scalelight`]
//! (see that field's own docs on why it's zeroed until Phase 6) instead
//! of a `lighttable_t**` — same collapse-one-indirection choice already
//! used throughout `r_plane.rs`/`r_main.rs`.
//!
//! `maskedtexturecol`, `sprtopclip` and `sprbottomclip` live in
//! [`crate::r_plane::RPlane::openings`], as in the original — see
//! [`crate::r_defs::SpriteClip`] and [`DrawSeg`] for the index
//! convention. `MAXSHORT` (`maskedtexturecol[x] == MAXSHORT` meaning "no
//! masked column here") becomes [`MASKEDTEXTURECOL_NONE`] (`i32::MAX`) —
//! kept as a literal sentinel value (rather than `Option<i32>`) since
//! that's exactly how the original code tests it.
//!
//! `extralight`/`fixedcolormap` (player state, Phase 6) and `projection`/
//! `centeryfrac` (view-size state from `R_ExecuteSetViewSize`, owned by
//! `r_main.rs`'s `Renderer`) are passed as explicit parameters, matching
//! every other Phase 5e module's treatment of the same globals —
//! [`RSegs::r_render_masked_seg_range`] takes them bundled as
//! [`crate::r_things::ViewParams`], since it shares its call path with
//! `r_things.rs`'s sprite drawing.

use crate::doomdata::{ML_DONTPEGBOTTOM, ML_DONTPEGTOP, ML_MAPPED};
use crate::doomtype::{MAXINT, MININT};
use crate::m_fixed::{fixed_mul, Fixed, FRACBITS};
use crate::p_setup::Level;
use crate::r_data::RData;
use crate::r_defs::{DrawSeg, SpriteClip, SIL_BOTH, SIL_BOTTOM, SIL_NONE, SIL_TOP};
use crate::r_draw::RDraw;
use crate::r_main::{RMain, LIGHTLEVELS, LIGHTSCALESHIFT, LIGHTSEGSHIFT, MAXLIGHTSCALE};
use crate::r_plane::RPlane;
use crate::r_state::RState;
use crate::r_things::{r_draw_masked_column, ColFunc, ViewParams};
use crate::tables::{Angle, ANG180, ANG90, ANGLETOFINESHIFT, FINESINE, FINETANGENT};
use crate::w_wad::WadFiles;

/// (`HEIGHTBITS`)/(`HEIGHTUNIT`) — fixed-point precision used for
/// `topfrac`/`bottomfrac`/`pixhigh`/`pixlow` texture-edge stepping.
const HEIGHTBITS: i32 = 12;
const HEIGHTUNIT: i32 = 1 << HEIGHTBITS;

/// Sentinel for "no masked column at this screen x" (`MAXSHORT` in the
/// original's `short maskedtexturecol[]`) — see module docs.
pub const MASKEDTEXTURECOL_NONE: i32 = i32::MAX;

/// Port of the `r_segs` module's per-wall-range scratch state. See
/// module docs for the wholesale globals-to-fields treatment.
#[derive(Default)]
pub struct RSegs {
    pub segtextured: bool,
    pub markfloor: bool,
    pub markceiling: bool,
    pub maskedtexture: bool,
    pub toptexture: i32,
    pub bottomtexture: i32,
    pub midtexture: i32,

    pub rw_normalangle: Angle,
    pub rw_angle1: Angle,

    pub rw_x: i32,
    pub rw_stopx: i32,
    pub rw_centerangle: Angle,
    pub rw_offset: Fixed,
    pub rw_distance: Fixed,
    pub rw_scale: Fixed,
    pub rw_scalestep: Fixed,
    pub rw_midtexturemid: Fixed,
    pub rw_toptexturemid: Fixed,
    pub rw_bottomtexturemid: Fixed,

    pub worldtop: i32,
    pub worldbottom: i32,
    pub worldhigh: i32,
    pub worldlow: i32,

    pub pixhigh: Fixed,
    pub pixlow: Fixed,
    pub pixhighstep: Fixed,
    pub pixlowstep: Fixed,

    pub topfrac: Fixed,
    pub topstep: Fixed,

    pub bottomfrac: Fixed,
    pub bottomstep: Fixed,

    /// Index into [`crate::r_main::RMain::scalelight`]'s light-level
    /// row (`walllights`, see module docs).
    pub walllights: usize,

    /// (`maskedtexturecol`) — index into
    /// [`crate::r_plane::RPlane::openings`] of the current wall range's
    /// first column, see module docs. Only meaningful while
    /// [`RSegs::maskedtexture`] is set.
    pub maskedtexturecol: usize,
}

impl RSegs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `R_StoreWallRange`, folding `R_RenderSegLoop`'s loop body
    /// in directly (see module docs). A wall segment will be drawn
    /// between start and stop pixels (inclusive) (the original's own
    /// comment, preserved).
    ///
    /// `rdraw`/`screen` are the real drawing context (column-drawing
    /// scratch state and destination framebuffer) — unlike
    /// `r_bsp.rs`'s `RenderHooks`, this method draws directly rather
    /// than through a trait, since `R_DrawColumn`/`RDraw` are already
    /// fully ported (Phase 5e's own `r_draw.rs` piece), so there's a
    /// real implementation to call.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `start >= viewwidth ||
    /// start > stop`, same `#ifdef RANGECHECK` guard as the original.
    #[allow(clippy::too_many_arguments)]
    pub fn r_store_wall_range(
        &mut self,
        rmain: &RMain,
        rstate: &RState,
        rplane: &mut RPlane,
        rdata: &mut RData,
        wad: &mut WadFiles,
        level: &mut Level,
        rdraw: &mut RDraw,
        screen: &mut [u8],
        seg_idx: usize,
        start: i32,
        stop: i32,
        viewwidth: i32,
        viewheight: i32,
        centeryfrac: Fixed,
        // `projection` (== `centerxfrac` in the original, set by
        // `R_ExecuteSetViewSize`) — required by
        // `RMain::scale_from_global_angle`, see that method's docs.
        projection: Fixed,
        extralight: i32,
        fixedcolormap: Option<usize>,
        skyflatnum: i32,
        // (`floorplane`/`ceilingplane`) — the current subsector's
        // visplanes from `R_Subsector`, replaced by `R_CheckPlane` here
        // exactly like the original's globals.
        floorplane: &mut Option<usize>,
        ceilingplane: &mut Option<usize>,
    ) -> DrawSeg {
        assert!(
            start < viewwidth && start <= stop,
            "Bad R_RenderWallRange: {start} to {stop}"
        );

        let seg = level.segs[seg_idx];
        let sidedef_idx = seg.sidedef;
        let linedef_idx = seg.linedef;

        // mark the segment as visible for auto map (the original's own
        // comment, preserved)
        level.lines[linedef_idx].flags |= ML_MAPPED;

        let sidedef = level.sides[sidedef_idx];
        let linedef_flags = level.lines[linedef_idx].flags;
        let frontsector_idx = seg.frontsector;
        let backsector_idx = seg.backsector;

        // calculate rw_distance for scale calculation (the original's
        // own comment, preserved)
        self.rw_normalangle = seg.angle.wrapping_add(ANG90);
        let mut offsetangle = self.rw_normalangle.wrapping_sub(self.rw_angle1) as i32;
        offsetangle = offsetangle.unsigned_abs() as i32;
        let mut offsetangle = offsetangle as Angle;
        if offsetangle > ANG90 {
            offsetangle = ANG90;
        }

        let distangle = ANG90.wrapping_sub(offsetangle);
        let v1 = level.vertexes[seg.v1];
        let hyp = rmain.point_to_dist(v1.x, v1.y);
        let sineval = FINESINE[(distangle >> ANGLETOFINESHIFT) as usize];
        self.rw_distance = fixed_mul(hyp, sineval);

        self.rw_x = start;
        self.rw_stopx = stop + 1;

        // calculate scale at both ends and step (the original's own
        // comment, preserved)
        self.rw_scale = rmain.scale_from_global_angle(
            rmain
                .viewangle
                .wrapping_add(rstate.xtoviewangle[start as usize]),
            projection,
            self.rw_distance,
            self.rw_normalangle,
            0,
        );

        let mut ds = DrawSeg {
            curline: seg_idx,
            x1: start,
            x2: stop,
            scale1: self.rw_scale,
            scale2: self.rw_scale,
            scalestep: 0,
            silhouette: SIL_NONE,
            bsilheight: 0,
            tsilheight: 0,
            sprtopclip: None,
            sprbottomclip: None,
            maskedtexturecol: None,
        };

        if stop > start {
            let scale2 = rmain.scale_from_global_angle(
                rmain
                    .viewangle
                    .wrapping_add(rstate.xtoviewangle[stop as usize]),
                projection,
                self.rw_distance,
                self.rw_normalangle,
                0,
            );
            ds.scale2 = scale2;
            self.rw_scalestep = (scale2 - self.rw_scale) / (stop - start);
            ds.scalestep = self.rw_scalestep;
        } else {
            ds.scale2 = ds.scale1;
        }

        // calculate texture boundaries and decide if floor / ceiling
        // marks are needed (the original's own comment, preserved)
        let frontsector = level.sectors[frontsector_idx].clone();
        self.worldtop = frontsector.ceilingheight - rmain.viewz;
        self.worldbottom = frontsector.floorheight - rmain.viewz;

        self.midtexture = 0;
        self.toptexture = 0;
        self.bottomtexture = 0;
        self.maskedtexture = false;
        ds.maskedtexturecol = None;

        match backsector_idx {
            None => {
                // single sided line (the original's own comment,
                // preserved)
                self.midtexture = rdata.texturetranslation[sidedef.midtexture as usize];
                // a single sided line is terminal, so it must mark ends
                // (the original's own comment, preserved)
                self.markfloor = true;
                self.markceiling = true;

                if linedef_flags & ML_DONTPEGBOTTOM != 0 {
                    let vtop =
                        frontsector.floorheight + rdata.textureheight[sidedef.midtexture as usize];
                    // bottom of texture at bottom
                    self.rw_midtexturemid = vtop - rmain.viewz;
                } else {
                    // top of texture at top
                    self.rw_midtexturemid = self.worldtop as Fixed;
                }
                self.rw_midtexturemid += sidedef.rowoffset;

                ds.silhouette = SIL_BOTH;
                ds.sprtopclip = Some(SpriteClip::ScreenHeight);
                ds.sprbottomclip = Some(SpriteClip::NegOne);
                ds.bsilheight = MAXINT;
                ds.tsilheight = MININT;
            }
            Some(back_idx) => {
                let backsector = level.sectors[back_idx].clone();

                ds.silhouette = SIL_NONE;

                if frontsector.floorheight > backsector.floorheight {
                    ds.silhouette = SIL_BOTTOM;
                    ds.bsilheight = frontsector.floorheight;
                } else if backsector.floorheight > rmain.viewz {
                    ds.silhouette = SIL_BOTTOM;
                    ds.bsilheight = MAXINT;
                }

                if frontsector.ceilingheight < backsector.ceilingheight {
                    ds.silhouette |= SIL_TOP;
                    ds.tsilheight = frontsector.ceilingheight;
                } else if backsector.ceilingheight < rmain.viewz {
                    ds.silhouette |= SIL_TOP;
                    ds.tsilheight = MININT;
                }

                if backsector.ceilingheight <= frontsector.floorheight {
                    ds.sprbottomclip = Some(SpriteClip::NegOne);
                    ds.bsilheight = MAXINT;
                    ds.silhouette |= SIL_BOTTOM;
                }

                if backsector.floorheight >= frontsector.ceilingheight {
                    ds.sprtopclip = Some(SpriteClip::ScreenHeight);
                    ds.tsilheight = MININT;
                    ds.silhouette |= SIL_TOP;
                }

                self.worldhigh = backsector.ceilingheight - rmain.viewz;
                self.worldlow = backsector.floorheight - rmain.viewz;

                // hack to allow height changes in outdoor areas (the
                // original's own comment, preserved)
                if frontsector.ceilingpic as i32 == skyflatnum
                    && backsector.ceilingpic as i32 == skyflatnum
                {
                    self.worldtop = self.worldhigh;
                }

                self.markfloor = self.worldlow != self.worldbottom
                    || backsector.floorpic != frontsector.floorpic
                    || backsector.lightlevel != frontsector.lightlevel;

                self.markceiling = self.worldhigh != self.worldtop
                    || backsector.ceilingpic != frontsector.ceilingpic
                    || backsector.lightlevel != frontsector.lightlevel;

                if backsector.ceilingheight <= frontsector.floorheight
                    || backsector.floorheight >= frontsector.ceilingheight
                {
                    // closed door (the original's own comment,
                    // preserved)
                    self.markceiling = true;
                    self.markfloor = true;
                }

                if self.worldhigh < self.worldtop {
                    // top texture (the original's own comment,
                    // preserved)
                    self.toptexture = rdata.texturetranslation[sidedef.toptexture as usize];
                    if linedef_flags & ML_DONTPEGTOP != 0 {
                        // top of texture at top
                        self.rw_toptexturemid = self.worldtop as Fixed;
                    } else {
                        let vtop = backsector.ceilingheight
                            + rdata.textureheight[sidedef.toptexture as usize];
                        // bottom of texture
                        self.rw_toptexturemid = vtop - rmain.viewz;
                    }
                }
                if self.worldlow > self.worldbottom {
                    // bottom texture (the original's own comment,
                    // preserved)
                    self.bottomtexture = rdata.texturetranslation[sidedef.bottomtexture as usize];

                    if linedef_flags & ML_DONTPEGBOTTOM != 0 {
                        // bottom of texture at bottom, top of texture at
                        // top (the original's own comment, preserved)
                        self.rw_bottomtexturemid = self.worldtop as Fixed;
                    } else {
                        self.rw_bottomtexturemid = self.worldlow as Fixed;
                    }
                }
                self.rw_toptexturemid = self.rw_toptexturemid.wrapping_add(sidedef.rowoffset);
                self.rw_bottomtexturemid = self.rw_bottomtexturemid.wrapping_add(sidedef.rowoffset);

                // allocate space for masked texture tables (the
                // original's own comment, preserved)
                if sidedef.midtexture != 0 {
                    // masked midtexture (the original's own comment,
                    // preserved)
                    self.maskedtexture = true;
                    // Every entry is written by the loop below before
                    // anything reads it (the original leaves them
                    // uninitialized); the fill value is irrelevant.
                    self.maskedtexturecol = rplane.openings.len();
                    ds.maskedtexturecol = Some(self.maskedtexturecol);
                    rplane.openings.resize(
                        rplane.openings.len() + (self.rw_stopx - self.rw_x) as usize,
                        MASKEDTEXTURECOL_NONE,
                    );
                }
            }
        }

        // calculate rw_offset (only needed for textured lines) (the
        // original's own comment, preserved)
        self.segtextured = self.midtexture != 0
            || self.toptexture != 0
            || self.bottomtexture != 0
            || self.maskedtexture;

        if self.segtextured {
            let mut offsetangle = self.rw_normalangle.wrapping_sub(self.rw_angle1);

            if offsetangle > ANG180 {
                offsetangle = (offsetangle as i32).wrapping_neg() as Angle;
            }
            if offsetangle > ANG90 {
                offsetangle = ANG90;
            }

            let sineval = FINESINE[(offsetangle >> ANGLETOFINESHIFT) as usize];
            self.rw_offset = fixed_mul(hyp, sineval);

            if self.rw_normalangle.wrapping_sub(self.rw_angle1) < ANG180 {
                self.rw_offset = -self.rw_offset;
            }

            self.rw_offset += sidedef.textureoffset + seg.offset;
            self.rw_centerangle = ANG90
                .wrapping_add(rmain.viewangle)
                .wrapping_sub(self.rw_normalangle);

            // calculate light table use different light tables for
            // horizontal / vertical / diagonal (the original's own
            // comment, preserved)
            if fixedcolormap.is_none() {
                let v2 = level.vertexes[seg.v2];
                let mut lightnum = (frontsector.lightlevel as i32 >> LIGHTSEGSHIFT) + extralight;

                if v1.y == v2.y {
                    lightnum -= 1;
                } else if v1.x == v2.x {
                    lightnum += 1;
                }

                self.walllights = if lightnum < 0 {
                    0
                } else if lightnum >= LIGHTLEVELS as i32 {
                    LIGHTLEVELS - 1
                } else {
                    lightnum as usize
                };
            }
        }

        // if a floor / ceiling plane is on the wrong side of the view
        // plane, it is definitely invisible and doesn't need to be
        // marked (the original's own comment, preserved)
        if frontsector.floorheight >= rmain.viewz {
            // above view plane
            self.markfloor = false;
        }

        if frontsector.ceilingheight <= rmain.viewz && frontsector.ceilingpic as i32 != skyflatnum {
            // below view plane
            self.markceiling = false;
        }

        // calculate incremental stepping values for texture edges (the
        // original's own comment, preserved)
        self.worldtop >>= 4;
        self.worldbottom >>= 4;

        self.topstep = -fixed_mul(self.rw_scalestep, self.worldtop);
        self.topfrac = (centeryfrac >> 4) - fixed_mul(self.worldtop, self.rw_scale);

        self.bottomstep = -fixed_mul(self.rw_scalestep, self.worldbottom);
        self.bottomfrac = (centeryfrac >> 4) - fixed_mul(self.worldbottom, self.rw_scale);

        if backsector_idx.is_some() {
            self.worldhigh >>= 4;
            self.worldlow >>= 4;

            if self.worldhigh < self.worldtop {
                self.pixhigh = (centeryfrac >> 4) - fixed_mul(self.worldhigh, self.rw_scale);
                self.pixhighstep = -fixed_mul(self.rw_scalestep, self.worldhigh);
            }

            if self.worldlow > self.worldbottom {
                self.pixlow = (centeryfrac >> 4) - fixed_mul(self.worldlow, self.rw_scale);
                self.pixlowstep = -fixed_mul(self.rw_scalestep, self.worldlow);
            }
        }

        // render it (the original's own comment, preserved)
        // A mark with no plane would dereference NULL in the original;
        // it can't happen, since R_Subsector finds a plane under exactly
        // the height conditions that leave the mark set above.
        if self.markceiling {
            let pl = ceilingplane.expect("R_StoreWallRange: markceiling without a ceilingplane");
            *ceilingplane = Some(rplane.r_check_plane(pl, self.rw_x, self.rw_stopx - 1));
        }
        if self.markfloor {
            let pl = floorplane.expect("R_StoreWallRange: markfloor without a floorplane");
            *floorplane = Some(rplane.r_check_plane(pl, self.rw_x, self.rw_stopx - 1));
        }
        let ceilingplane = if self.markceiling {
            *ceilingplane
        } else {
            None
        };
        let floorplane = if self.markfloor { *floorplane } else { None };

        // R_RenderSegLoop, folded in directly — see module docs.
        while self.rw_x < self.rw_stopx {
            let yl = {
                let mut yl = (self.topfrac + HEIGHTUNIT - 1) >> HEIGHTBITS;
                if yl < rplane.ceilingclip[self.rw_x as usize] + 1 {
                    yl = rplane.ceilingclip[self.rw_x as usize] + 1;
                }
                yl
            };

            if self.markceiling {
                if let Some(plane_idx) = ceilingplane {
                    let top = rplane.ceilingclip[self.rw_x as usize] + 1;
                    let mut bottom = yl - 1;
                    if bottom >= rplane.floorclip[self.rw_x as usize] {
                        bottom = rplane.floorclip[self.rw_x as usize] - 1;
                    }
                    if top <= bottom {
                        rplane.visplanes[plane_idx].set_top_at(self.rw_x, top as u8);
                        rplane.visplanes[plane_idx].set_bottom_at(self.rw_x, bottom as u8);
                    }
                }
            }

            let mut yh = self.bottomfrac >> HEIGHTBITS;
            if yh >= rplane.floorclip[self.rw_x as usize] {
                yh = rplane.floorclip[self.rw_x as usize] - 1;
            }

            if self.markfloor {
                if let Some(plane_idx) = floorplane {
                    let mut top = yh + 1;
                    let bottom = rplane.floorclip[self.rw_x as usize] - 1;
                    if top <= rplane.ceilingclip[self.rw_x as usize] {
                        top = rplane.ceilingclip[self.rw_x as usize] + 1;
                    }
                    if top <= bottom {
                        rplane.visplanes[plane_idx].set_top_at(self.rw_x, top as u8);
                        rplane.visplanes[plane_idx].set_bottom_at(self.rw_x, bottom as u8);
                    }
                }
            }

            let mut texturecolumn = 0;
            if self.segtextured {
                // calculate texture offset (the original's own comment,
                // preserved)
                let angle = ((self
                    .rw_centerangle
                    .wrapping_add(rstate.xtoviewangle[self.rw_x as usize]))
                    >> ANGLETOFINESHIFT) as usize;
                // `finetangent` has 4096 entries but the angle can reach
                // 8191 (a wall seen edge-on from behind); the original
                // then reads past the table into `finesine`, which
                // follows it in `tables.c` — reproduced here instead of
                // panicking.
                let tangent = match FINETANGENT.get(angle) {
                    Some(&t) => t,
                    None => FINESINE[(angle - FINETANGENT.len()).min(FINESINE.len() - 1)],
                };
                texturecolumn = (self.rw_offset - fixed_mul(tangent, self.rw_distance)) >> FRACBITS;

                // calculate lighting (the original's own comment,
                // preserved)
                let mut index = (self.rw_scale >> LIGHTSCALESHIFT) as usize;
                if index >= MAXLIGHTSCALE {
                    index = MAXLIGHTSCALE - 1;
                }
                // With a fixed colormap the original points `walllights`
                // at `scalelightfixed` (every entry the fixed map).
                rdraw.dc_colormap = match fixedcolormap {
                    Some(fixed) => fixed,
                    None => rmain.scalelight[self.walllights][index] as usize,
                };
                rdraw.dc_x = self.rw_x;
                rdraw.dc_iscale = ((0xffffffffu32) / (self.rw_scale as u32)) as i32;
            }

            // draw the wall tiers (the original's own comment,
            // preserved)
            if self.midtexture != 0 {
                // single sided line (the original's own comment,
                // preserved)
                rdraw.dc_yl = yl;
                rdraw.dc_yh = yh;
                rdraw.dc_texturemid = self.rw_midtexturemid;
                let source = rdata.get_column(wad, self.midtexture as usize, texturecolumn);
                rdraw.r_draw_column(screen, &rdata.colormaps, &source);
                rplane.ceilingclip[self.rw_x as usize] = viewheight;
                rplane.floorclip[self.rw_x as usize] = -1;
            } else {
                // two sided line (the original's own comment, preserved)
                if self.toptexture != 0 {
                    // top wall (the original's own comment, preserved)
                    let mut mid = self.pixhigh >> HEIGHTBITS;
                    self.pixhigh += self.pixhighstep;

                    if mid >= rplane.floorclip[self.rw_x as usize] {
                        mid = rplane.floorclip[self.rw_x as usize] - 1;
                    }

                    if mid >= yl {
                        rdraw.dc_yl = yl;
                        rdraw.dc_yh = mid;
                        rdraw.dc_texturemid = self.rw_toptexturemid;
                        let source = rdata.get_column(wad, self.toptexture as usize, texturecolumn);
                        rdraw.r_draw_column(screen, &rdata.colormaps, &source);
                        rplane.ceilingclip[self.rw_x as usize] = mid;
                    } else {
                        rplane.ceilingclip[self.rw_x as usize] = yl - 1;
                    }
                } else if self.markceiling {
                    // no top wall (the original's own comment,
                    // preserved)
                    rplane.ceilingclip[self.rw_x as usize] = yl - 1;
                }

                if self.bottomtexture != 0 {
                    // bottom wall (the original's own comment,
                    // preserved)
                    let mut mid = (self.pixlow + HEIGHTUNIT - 1) >> HEIGHTBITS;
                    self.pixlow += self.pixlowstep;

                    if mid <= rplane.ceilingclip[self.rw_x as usize] {
                        mid = rplane.ceilingclip[self.rw_x as usize] + 1;
                    }

                    if mid <= yh {
                        rdraw.dc_yl = mid;
                        rdraw.dc_yh = yh;
                        rdraw.dc_texturemid = self.rw_bottomtexturemid;
                        let source =
                            rdata.get_column(wad, self.bottomtexture as usize, texturecolumn);
                        rdraw.r_draw_column(screen, &rdata.colormaps, &source);
                        rplane.floorclip[self.rw_x as usize] = mid;
                    } else {
                        rplane.floorclip[self.rw_x as usize] = yh + 1;
                    }
                } else if self.markfloor {
                    // no bottom wall (the original's own comment,
                    // preserved)
                    rplane.floorclip[self.rw_x as usize] = yh + 1;
                }

                if self.maskedtexture {
                    // save texturecol for backdrawing of masked mid
                    // texture (the original's own comment, preserved)
                    let idx = self.maskedtexturecol + (self.rw_x - start) as usize;
                    rplane.openings[idx] = texturecolumn;
                }
            }

            self.rw_scale += self.rw_scalestep;
            self.topfrac += self.topstep;
            self.bottomfrac += self.bottomstep;
            self.rw_x += 1;
        }

        // save sprite clipping info (the original's own comment,
        // preserved)
        let range = start as usize..self.rw_stopx as usize;
        if (ds.silhouette & SIL_TOP != 0 || self.maskedtexture) && ds.sprtopclip.is_none() {
            ds.sprtopclip = Some(SpriteClip::Openings(rplane.openings.len()));
            rplane
                .openings
                .extend_from_slice(&rplane.ceilingclip[range.clone()]);
        }

        if (ds.silhouette & SIL_BOTTOM != 0 || self.maskedtexture) && ds.sprbottomclip.is_none() {
            ds.sprbottomclip = Some(SpriteClip::Openings(rplane.openings.len()));
            rplane.openings.extend_from_slice(&rplane.floorclip[range]);
        }

        if self.maskedtexture && ds.silhouette & SIL_TOP == 0 {
            ds.silhouette |= SIL_TOP;
            ds.tsilheight = MININT;
        }
        if self.maskedtexture && ds.silhouette & SIL_BOTTOM == 0 {
            ds.silhouette |= SIL_BOTTOM;
            ds.bsilheight = MAXINT;
        }

        ds
    }

    /// Port of `R_RenderMaskedSegRange`.
    ///
    /// Marks every drawn column as done in `ds`'s `maskedtexturecol`
    /// (in [`RPlane::openings`]), exactly like the original, so a later
    /// call covering the same columns (`R_DrawMasked`'s final pass after
    /// `R_DrawSprite` already drew part of the range) skips them.
    ///
    /// # Panics
    /// Panics if `ds` isn't a masked two-sided seg (no backsector, or no
    /// `maskedtexturecol`/sprite clips) — the original would dereference
    /// `NULL` there instead; no caller ever passes one.
    #[allow(clippy::too_many_arguments)]
    pub fn r_render_masked_seg_range(
        &mut self,
        rmain: &RMain,
        rdata: &mut RData,
        wad: &mut WadFiles,
        level: &Level,
        rplane: &mut RPlane,
        rdraw: &mut RDraw,
        screen: &mut [u8],
        view: &ViewParams,
        ds: &DrawSeg,
        x1: i32,
        x2: i32,
    ) {
        // Calculate light table. Use different light tables for
        // horizontal / vertical / diagonal. Diagonal? OPTIMIZE: get rid
        // of LIGHTSEGSHIFT globally (the original's own comment,
        // preserved)
        let seg = level.segs[ds.curline];
        let frontsector = &level.sectors[seg.frontsector];
        let backsector_idx = seg.backsector.expect(
            "R_RenderMaskedSegRange: masked seg range requires a two-sided line (backsector)",
        );
        let backsector = &level.sectors[backsector_idx];
        let sidedef = level.sides[seg.sidedef];
        let texnum = rdata.texturetranslation[sidedef.midtexture as usize];
        let linedef_flags = level.lines[seg.linedef].flags;

        let mut lightnum = (frontsector.lightlevel as i32 >> LIGHTSEGSHIFT) + view.extralight;

        let v1 = level.vertexes[seg.v1];
        let v2 = level.vertexes[seg.v2];
        if v1.y == v2.y {
            lightnum -= 1;
        } else if v1.x == v2.x {
            lightnum += 1;
        }

        self.walllights = if lightnum < 0 {
            0
        } else if lightnum >= LIGHTLEVELS as i32 {
            LIGHTLEVELS - 1
        } else {
            lightnum as usize
        };

        self.maskedtexturecol = ds
            .maskedtexturecol
            .expect("R_RenderMaskedSegRange: drawseg has no maskedtexturecol");
        let mfloorclip = ds
            .sprbottomclip
            .expect("R_RenderMaskedSegRange: drawseg has no sprbottomclip");
        let mceilingclip = ds
            .sprtopclip
            .expect("R_RenderMaskedSegRange: drawseg has no sprtopclip");

        self.rw_scalestep = ds.scalestep;
        let mut spryscale = ds
            .scale1
            .wrapping_add((x1 - ds.x1).wrapping_mul(self.rw_scalestep));

        // find positioning (the original's own comment, preserved)
        rdraw.dc_texturemid = if linedef_flags & ML_DONTPEGBOTTOM != 0 {
            let base = frontsector.floorheight.max(backsector.floorheight);
            base + rdata.textureheight[texnum as usize] - rmain.viewz
        } else {
            let base = frontsector.ceilingheight.min(backsector.ceilingheight);
            base - rmain.viewz
        };
        rdraw.dc_texturemid += sidedef.rowoffset;

        if let Some(fixed) = view.fixedcolormap {
            rdraw.dc_colormap = fixed;
        }

        // draw the columns (the original's own comment, preserved)
        let colfunc = ColFunc::base(view.detailshift);
        for dc_x in x1..=x2 {
            let col_idx = self.maskedtexturecol + (dc_x - ds.x1) as usize;
            let texturecolumn = rplane.openings[col_idx];

            // calculate lighting (the original's own comment,
            // preserved)
            if texturecolumn != MASKEDTEXTURECOL_NONE {
                if view.fixedcolormap.is_none() {
                    let mut index = (spryscale >> LIGHTSCALESHIFT) as u32 as usize;
                    if index >= MAXLIGHTSCALE {
                        index = MAXLIGHTSCALE - 1;
                    }
                    rdraw.dc_colormap = rmain.scalelight[self.walllights][index] as usize;
                }

                let sprtopscreen = view.centeryfrac - fixed_mul(rdraw.dc_texturemid, spryscale);
                rdraw.dc_iscale = (0xffffffffu32 / (spryscale as u32)) as i32;

                // draw the texture (the original's own comment,
                // preserved)
                let column = rdata.get_masked_column(wad, texnum as usize, texturecolumn);
                let floorclip = mfloorclip.at(dc_x, ds.x1, &rplane.openings, view.viewheight);
                let ceilingclip = mceilingclip.at(dc_x, ds.x1, &rplane.openings, view.viewheight);
                r_draw_masked_column(
                    rdraw,
                    screen,
                    &rdata.colormaps,
                    &column,
                    dc_x,
                    spryscale,
                    sprtopscreen,
                    floorclip,
                    ceilingclip,
                    colfunc,
                    view.viewheight,
                );
                rplane.openings[col_idx] = MASKEDTEXTURECOL_NONE;
            }
            spryscale = spryscale.wrapping_add(self.rw_scalestep);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_texture_col_none_sentinel_matches_i32_max() {
        assert_eq!(MASKEDTEXTURECOL_NONE, i32::MAX);
    }
}
