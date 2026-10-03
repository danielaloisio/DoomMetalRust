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
//	BSP traversal, handling of LineSegs for rendering.
//
//-----------------------------------------------------------------------------

//! Rust port of `r_bsp.c` (partial, see note below). BSP
//! traversal, handling of LineSegs for rendering.
//!
//! # Scope
//!
//! Ported: the solid/pass wall-segment clip-range tracking
//! ([`RBsp::clear_clip_segs`], [`RBsp::clip_solid_wall_segment`],
//! [`RBsp::clip_pass_wall_segment`]), [`RBsp::check_bbox`]
//! (`R_CheckBBox`), and the BSP traversal entry points
//! ([`RBsp::add_line`] / `R_AddLine`, [`RBsp::subsector`] /
//! `R_Subsector`, [`RBsp::render_bsp_node`] / `R_RenderBSPNode`).
//!
//! The actual drawing calls this traversal drives — `R_StoreWallRange`
//! (`r_segs.rs`), `R_FindPlane` (`r_plane.rs`) and `R_AddSprites`
//! (`r_things.rs`) — go through the [`RenderHooks`] trait, so the
//! traversal is testable on its own ([`RecordingHooks`]) and
//! `r_main.rs`'s frame renderer plugs in the real implementations.
//!
//! The hooks receive the [`Level`] mutably on every call (the original
//! writes `ML_MAPPED` on lines and `validcount` on sectors from inside
//! those calls), so every traversal method here takes `&mut Level` and
//! only holds copies of level data across hook calls.
//!
//! `curline`/`rw_angle1` (`r_segs.c` state set by `R_AddLine`) and
//! `floorplane`/`ceilingplane` (`r_plane.c` state set by
//! `R_Subsector`, updated by `R_StoreWallRange`'s `R_CheckPlane`) are
//! fields on [`RBsp`], handed to [`RenderHooks::store_wall_range`].
//!
//! # `goto` handling
//!
//! `R_ClipSolidWallSegment`'s `goto crunch` (a forward jump to shared
//! cleanup code partway through the function, skipped entirely on one
//! branch) is restructured as an early-return-shaped `if`/`else` — the
//! `crunch:` block runs unconditionally except when `next == start`,
//! which naturally becomes a guard clause. `R_AddLine`'s `goto
//! clipsolid`/`goto clippass` (jumps to the function's tail) become a
//! plain `if backsector.is_none() { ... } else { ... }`.

use crate::m_fixed::Fixed;
use crate::p_setup::Level;
use crate::r_defs::{DrawSeg, MAXDRAWSEGS};
use crate::r_main::RMain;
use crate::r_state::RState;
use crate::tables::{Angle, ANG180, ANG90, ANGLETOFINESHIFT};

/// (`MAXSEGS`) — max span of solid-wall clip ranges tracked at once.
pub const MAXSEGS: usize = 32;

/// A clip range: `[first, last]` inclusive, in screen-x columns
/// (`cliprange_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClipRange {
    first: i32,
    last: i32,
}

/// Checkcoord lookup table for [`RBsp::check_bbox`], transcribed
/// verbatim from `r_bsp.c`'s `checkcoord[12][4]`.
///
/// The original declares this with 12 rows but supplies only 11
/// initializers (a real quirk in the source, same pattern as
/// `dstrings.c`'s `endmsg[]` from Phase 2) — C zero-initializes the
/// remaining row. `boxpos` (this table's index, `boxy<<2 + boxx` with
/// `boxx,boxy` each in `0..=2`, and never 5, which short-circuits
/// earlier in `R_CheckBBox`) only ever reaches 0..=8, so the missing
/// 12th row is genuinely unreachable in both the original and this
/// port — the 12-row shape is kept anyway, with the last row explicit
/// zeros, purely for fidelity to the original's declared size.
#[rustfmt::skip]
const CHECKCOORD: [[usize; 4]; 12] = [
    [3, 0, 2, 1],
    [3, 0, 2, 0],
    [3, 1, 2, 0],
    [0, 0, 0, 0],
    [2, 0, 2, 1],
    [0, 0, 0, 0],
    [3, 1, 3, 0],
    [0, 0, 0, 0],
    [2, 0, 3, 1],
    [2, 1, 3, 1],
    [2, 1, 3, 0],
    [0, 0, 0, 0],
];

/// The `r_segs`/`r_plane`/`r_things` entry points this module drives
/// — see module docs.
pub trait RenderHooks {
    /// `R_StoreWallRange` for `seg` (the original's `curline`) over
    /// columns `first..=last`. `floorplane`/`ceilingplane` are the
    /// current subsector's visplanes, which the call may replace (the
    /// original's `R_CheckPlane` reassigning the globals). Returns the
    /// new drawseg, if any.
    #[allow(clippy::too_many_arguments)]
    fn store_wall_range(
        &mut self,
        level: &mut Level,
        seg: usize,
        rw_angle1: Angle,
        floorplane: &mut Option<usize>,
        ceilingplane: &mut Option<usize>,
        first: i32,
        last: i32,
    ) -> Option<DrawSeg>;

    /// `R_FindPlane`. Returns the visplane's index.
    fn find_plane(&mut self, height: Fixed, picnum: i32, lightlevel: i32) -> usize;

    /// `R_AddSprites`. Queues a sector's mobjs for sprite drawing.
    fn add_sprites(&mut self, level: &mut Level, sector: usize);
}

/// A [`RenderHooks`] implementation that records calls instead of
/// drawing anything, so traversal can be tested on its own.
#[derive(Debug, Default)]
pub struct RecordingHooks {
    pub wall_ranges: Vec<(i32, i32)>,
    pub find_plane_calls: Vec<(Fixed, i32, i32)>,
    pub sprite_sectors: Vec<usize>,
}

impl RenderHooks for RecordingHooks {
    fn store_wall_range(
        &mut self,
        _level: &mut Level,
        _seg: usize,
        _rw_angle1: Angle,
        _floorplane: &mut Option<usize>,
        _ceilingplane: &mut Option<usize>,
        first: i32,
        last: i32,
    ) -> Option<DrawSeg> {
        self.wall_ranges.push((first, last));
        None
    }

    fn find_plane(&mut self, height: Fixed, picnum: i32, lightlevel: i32) -> usize {
        self.find_plane_calls.push((height, picnum, lightlevel));
        self.find_plane_calls.len() - 1
    }

    fn add_sprites(&mut self, _level: &mut Level, sector: usize) {
        self.sprite_sectors.push(sector);
    }
}

/// Port of the `r_bsp` module state (`curline`/`sidedef`/`linedef`/
/// `frontsector`/`backsector`/`drawsegs`/`ds_p`/`solidsegs`/`newend`).
///
/// Indices (`Option<usize>`) replace the original's raw pointers into
/// shared level arrays, per the plan's general representation choice
/// (see `r_defs`'s module docs) — `curline`/`sidedef`/`linedef`/
/// `frontsector`/`backsector` become indices into the
/// [`crate::p_setup::Level`] arrays passed into each call, rather than
/// being resolved against a global.
pub struct RBsp {
    /// (`drawsegs`/`ds_p`) — solid wall ranges recorded this frame. The
    /// original preallocates a fixed `[MAXDRAWSEGS]` array and tracks
    /// `ds_p` as the next-free slot; a growable `Vec` replaces both
    /// (see `r_defs::DrawSeg`, Phase 5a) — `MAXDRAWSEGS` is kept as a
    /// the original limit, enforced like the original: once full, further
    /// wall ranges are skipped rather than drawn.
    pub drawsegs: Vec<DrawSeg>,

    /// (`solidsegs`/`newend`) — the solid-wall clip-range list. Starts
    /// with two sentinel ranges (see [`RBsp::clear_clip_segs`]);
    /// `newend` is implicit as `solidsegs.len()` on a `Vec` instead of a
    /// separate pointer.
    solidsegs: Vec<ClipRange>,

    /// Current seg/side/line/sector context, indices into the active
    /// [`Level`]'s arrays — set by [`RBsp::add_line`]/
    /// [`RBsp::subsector`], read by whatever Phase 5e code eventually
    /// consumes them (mirroring the original's global scratch
    /// variables, scoped to this struct instead).
    pub curline: Option<usize>,
    pub frontsector: Option<usize>,
    pub backsector: Option<usize>,

    /// (`rw_angle1`) — global angle of `curline`'s first vertex, set by
    /// [`RBsp::add_line`] and needed by `R_StoreWallRange`'s segcalc.
    pub rw_angle1: Angle,

    /// (`floorplane`/`ceilingplane`) — the current subsector's visplane
    /// indices, `None` for the original's `NULL`.
    pub floorplane: Option<usize>,
    pub ceilingplane: Option<usize>,

    /// (`skyflatnum`) — read by [`RBsp::subsector`]'s ceiling test.
    /// `-1` (matching no flat) until a level sets it.
    pub skyflatnum: i32,

    /// (`sscount`) — subsectors rendered this frame, for profiling
    /// purposes (the original's own comment on the underlying
    /// `r_main.c` global, preserved) — kept here since
    /// [`RBsp::subsector`] is what increments it.
    pub sscount: i32,
}

impl Default for RBsp {
    fn default() -> Self {
        Self::new()
    }
}

impl RBsp {
    pub fn new() -> Self {
        RBsp {
            drawsegs: Vec::new(),
            solidsegs: Vec::new(),
            curline: None,
            frontsector: None,
            backsector: None,
            rw_angle1: 0,
            floorplane: None,
            ceilingplane: None,
            skyflatnum: -1,
            sscount: 0,
        }
    }

    /// `R_StoreWallRange` call site shared by both clippers: skips the
    /// range once [`MAXDRAWSEGS`] drawsegs exist (the original's "don't
    /// overflow and crash" early return at the top of
    /// `R_StoreWallRange`) and keeps the returned drawseg.
    fn store_wall_range(
        &mut self,
        hooks: &mut impl RenderHooks,
        level: &mut Level,
        first: i32,
        last: i32,
    ) {
        if self.drawsegs.len() == MAXDRAWSEGS {
            return;
        }
        let seg = self
            .curline
            .expect("R_StoreWallRange: no curline (clip called outside R_AddLine)");
        if let Some(ds) = hooks.store_wall_range(
            level,
            seg,
            self.rw_angle1,
            &mut self.floorplane,
            &mut self.ceilingplane,
            first,
            last,
        ) {
            self.drawsegs.push(ds);
        }
    }

    /// Port of `R_ClearDrawSegs`.
    pub fn clear_draw_segs(&mut self) {
        self.drawsegs.clear();
    }

    /// Port of `R_ClearClipSegs`.
    pub fn clear_clip_segs(&mut self, viewwidth: i32) {
        self.solidsegs = vec![
            ClipRange {
                first: -0x7fffffff,
                last: -1,
            },
            ClipRange {
                first: viewwidth,
                last: 0x7fffffff,
            },
        ];
    }

    /// Port of `R_ClipSolidWallSegment`. Does handle solid walls, e.g.
    /// single sided LineDefs (middle texture) that entirely block the
    /// view (the original's own comment, preserved).
    ///
    /// See module docs for how the original's `goto crunch` is
    /// restructured.
    pub fn clip_solid_wall_segment(
        &mut self,
        hooks: &mut impl RenderHooks,
        level: &mut Level,
        first: i32,
        last: i32,
    ) {
        // Find the first range that touches the range (adjacent pixels
        // are touching) (the original's own comment, preserved).
        let mut start = 0usize;
        while self.solidsegs[start].last < first - 1 {
            start += 1;
        }

        if first < self.solidsegs[start].first {
            if last < self.solidsegs[start].first - 1 {
                // Post is entirely visible (above start), so insert a
                // new clippost (the original's own comment, preserved).
                self.store_wall_range(hooks, level, first, last);
                self.solidsegs.insert(start, ClipRange { first, last });
                return;
            }

            // There is a fragment above *start.
            self.store_wall_range(hooks, level, first, self.solidsegs[start].first - 1);
            self.solidsegs[start].first = first;
        }

        // Bottom contained in start?
        if last <= self.solidsegs[start].last {
            return;
        }

        let mut next = start;
        let mut crunch = true;
        while last >= self.solidsegs[next + 1].first - 1 {
            // There is a fragment between two posts.
            let (a, b) = (
                self.solidsegs[next].last + 1,
                self.solidsegs[next + 1].first - 1,
            );
            self.store_wall_range(hooks, level, a, b);
            next += 1;

            if last <= self.solidsegs[next].last {
                // Bottom is contained in next. Adjust the clip size.
                self.solidsegs[start].last = self.solidsegs[next].last;
                crunch = false;
                break;
            }

            if next + 1 >= self.solidsegs.len() {
                break;
            }
        }

        if crunch {
            // There is a fragment after *next.
            self.store_wall_range(hooks, level, self.solidsegs[next].last + 1, last);
            self.solidsegs[start].last = last;
        }

        // Remove start+1 to next from the clip list, because start now
        // covers their area (the original's own comment, preserved).
        if next == start {
            // Post just extended past the bottom of one post.
            return;
        }

        let tail: Vec<ClipRange> = self.solidsegs[next + 1..].to_vec();
        self.solidsegs.truncate(start + 1);
        self.solidsegs.extend(tail);
    }

    /// Port of `R_ClipPassWallSegment`. Clips the given range of
    /// columns, but does not include it in the clip list. Does handle
    /// windows, e.g. LineDefs with upper and lower texture (the
    /// original's own comment, preserved).
    pub fn clip_pass_wall_segment(
        &mut self,
        hooks: &mut impl RenderHooks,
        level: &mut Level,
        first: i32,
        last: i32,
    ) {
        let mut start = 0usize;
        while self.solidsegs[start].last < first - 1 {
            start += 1;
        }

        if first < self.solidsegs[start].first {
            if last < self.solidsegs[start].first - 1 {
                // Post is entirely visible (above start).
                self.store_wall_range(hooks, level, first, last);
                return;
            }

            // There is a fragment above *start.
            self.store_wall_range(hooks, level, first, self.solidsegs[start].first - 1);
        }

        // Bottom contained in start?
        if last <= self.solidsegs[start].last {
            return;
        }

        while last >= self.solidsegs[start + 1].first - 1 {
            // There is a fragment between two posts.
            let (a, b) = (
                self.solidsegs[start].last + 1,
                self.solidsegs[start + 1].first - 1,
            );
            self.store_wall_range(hooks, level, a, b);
            start += 1;

            if last <= self.solidsegs[start].last {
                return;
            }
        }

        // There is a fragment after *next.
        self.store_wall_range(hooks, level, self.solidsegs[start].last + 1, last);
    }

    /// Port of `R_AddLine`. Clips the given segment and adds any visible
    /// pieces to the line list (the original's own comment, preserved).
    ///
    /// See module docs for how the original's `goto clipsolid`/`goto
    /// clippass` are restructured.
    pub fn add_line(
        &mut self,
        hooks: &mut impl RenderHooks,
        rmain: &RMain,
        rstate: &RState,
        level: &mut Level,
        seg_idx: usize,
    ) {
        self.curline = Some(seg_idx);
        let seg = level.segs[seg_idx];
        let v1 = level.vertexes[seg.v1];
        let v2 = level.vertexes[seg.v2];

        // OPTIMIZE: quickly reject orthogonal back sides (the
        // original's own comment, preserved).
        let angle1 = rmain.point_to_angle(v1.x, v1.y);
        let angle2 = rmain.point_to_angle(v2.x, v2.y);

        // Clip to view edges. OPTIMIZE: make constant out of
        // 2*clipangle (FIELDOFVIEW) (the original's own comment,
        // preserved).
        let span = angle1.wrapping_sub(angle2);

        // Back side? I.e. backface culling? (the original's own
        // comment, preserved)
        if span >= ANG180 {
            return;
        }

        // Global angle needed by segcalc (the original's own comment,
        // preserved).
        self.rw_angle1 = angle1;

        let mut angle1 = angle1.wrapping_sub(rmain.viewangle);
        let mut angle2 = angle2.wrapping_sub(rmain.viewangle);

        let mut tspan = angle1.wrapping_add(rstate.clipangle);
        if tspan > 2u32.wrapping_mul(rstate.clipangle) {
            tspan = tspan.wrapping_sub(2u32.wrapping_mul(rstate.clipangle));
            // Totally off the left edge?
            if tspan >= span {
                return;
            }
            angle1 = rstate.clipangle;
        }
        let mut tspan2 = rstate.clipangle.wrapping_sub(angle2);
        if tspan2 > 2u32.wrapping_mul(rstate.clipangle) {
            tspan2 = tspan2.wrapping_sub(2u32.wrapping_mul(rstate.clipangle));
            // Totally off the left edge?
            if tspan2 >= span {
                return;
            }
            angle2 = (rstate.clipangle as i32).wrapping_neg() as Angle;
        }

        // The seg is in the view range, but not necessarily visible
        // (the original's own comment, preserved).
        let angle1 = (angle1.wrapping_add(ANG90)) >> ANGLETOFINESHIFT;
        let angle2 = (angle2.wrapping_add(ANG90)) >> ANGLETOFINESHIFT;
        let x1 = rstate.viewangletox[angle1 as usize];
        let x2 = rstate.viewangletox[angle2 as usize];

        // Does not cross a pixel?
        if x1 == x2 {
            return;
        }

        self.backsector = seg.backsector;
        self.frontsector = Some(seg.frontsector);

        let is_solid = match self.backsector {
            // Single sided line?
            None => true,
            Some(back_idx) => {
                let back = &level.sectors[back_idx];
                let front = &level.sectors[seg.frontsector];

                if back.ceilingheight <= front.floorheight
                    || back.floorheight >= front.ceilingheight
                {
                    // Closed door.
                    true
                } else if back.ceilingheight != front.ceilingheight
                    || back.floorheight != front.floorheight
                {
                    // Window.
                    false
                } else if back.ceilingpic == front.ceilingpic
                    && back.floorpic == front.floorpic
                    && back.lightlevel == front.lightlevel
                    && level.sides[seg.sidedef].midtexture == 0
                {
                    // Reject empty lines used for triggers and special
                    // events. Identical floor and ceiling on both
                    // sides, identical light levels on both sides, and
                    // no middle texture (the original's own comment,
                    // preserved).
                    return;
                } else {
                    false
                }
            }
        };

        if is_solid {
            self.clip_solid_wall_segment(hooks, level, x1, x2 - 1);
        } else {
            self.clip_pass_wall_segment(hooks, level, x1, x2 - 1);
        }
    }

    /// Port of `R_CheckBBox`. Checks BSP node/subtree bounding box.
    /// Returns true if some part of the bbox might be visible (the
    /// original's own comment, preserved).
    pub fn check_bbox(&self, rmain: &RMain, rstate: &RState, bspcoord: &[Fixed; 4]) -> bool {
        use crate::m_bbox::{BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};

        // Find the corners of the box that define the edges from
        // current viewpoint (the original's own comment, preserved).
        let boxx = if rmain.viewx <= bspcoord[BOXLEFT] {
            0
        } else if rmain.viewx < bspcoord[BOXRIGHT] {
            1
        } else {
            2
        };

        let boxy = if rmain.viewy >= bspcoord[BOXTOP] {
            0
        } else if rmain.viewy > bspcoord[BOXBOTTOM] {
            1
        } else {
            2
        };

        let boxpos = (boxy << 2) + boxx;
        if boxpos == 5 {
            return true;
        }

        let coords = CHECKCOORD[boxpos];
        let x1 = bspcoord[coords[0]];
        let y1 = bspcoord[coords[1]];
        let x2 = bspcoord[coords[2]];
        let y2 = bspcoord[coords[3]];

        // check clip list for an open space (the original's own
        // comment, preserved)
        let angle1 = rmain.point_to_angle(x1, y1).wrapping_sub(rmain.viewangle);
        let angle2 = rmain.point_to_angle(x2, y2).wrapping_sub(rmain.viewangle);

        let span = angle1.wrapping_sub(angle2);

        // Sitting on a line?
        if span >= ANG180 {
            return true;
        }

        let mut angle1 = angle1;
        let mut angle2 = angle2;

        let mut tspan = angle1.wrapping_add(rstate.clipangle);
        if tspan > 2u32.wrapping_mul(rstate.clipangle) {
            tspan = tspan.wrapping_sub(2u32.wrapping_mul(rstate.clipangle));
            if tspan >= span {
                return false;
            }
            angle1 = rstate.clipangle;
        }
        let mut tspan2 = rstate.clipangle.wrapping_sub(angle2);
        if tspan2 > 2u32.wrapping_mul(rstate.clipangle) {
            tspan2 = tspan2.wrapping_sub(2u32.wrapping_mul(rstate.clipangle));
            if tspan2 >= span {
                return false;
            }
            angle2 = (rstate.clipangle as i32).wrapping_neg() as Angle;
        }

        // Find the first clippost that touches the source post
        // (adjacent pixels are touching) (the original's own comment,
        // preserved).
        let angle1 = (angle1.wrapping_add(ANG90)) >> ANGLETOFINESHIFT;
        let angle2 = (angle2.wrapping_add(ANG90)) >> ANGLETOFINESHIFT;
        let sx1 = rstate.viewangletox[angle1 as usize];
        let mut sx2 = rstate.viewangletox[angle2 as usize];

        // Does not cross a pixel.
        if sx1 == sx2 {
            return false;
        }
        sx2 -= 1;

        let mut start = 0usize;
        while self.solidsegs[start].last < sx2 {
            start += 1;
        }

        if sx1 >= self.solidsegs[start].first && sx2 <= self.solidsegs[start].last {
            // The clippost contains the new span.
            return false;
        }

        true
    }

    /// Port of `R_Subsector`. Determine floor/ceiling planes. Add
    /// sprites of things in sector. Draw one or more line segments (the
    /// original's own comment, preserved).
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if `num` is out of range, same
    /// as the original's `#ifdef RANGECHECK` guard (always compiled in
    /// here, matching `doomdef.h`'s `#define RANGECHECK`).
    pub fn subsector(
        &mut self,
        hooks: &mut impl RenderHooks,
        rmain: &RMain,
        rstate: &RState,
        level: &mut Level,
        num: usize,
    ) {
        assert!(
            num < level.subsectors.len(),
            "R_Subsector: ss {num} with numss = {}",
            level.subsectors.len()
        );

        self.sscount += 1;
        let sub = level.subsectors[num];
        self.frontsector = Some(sub.sector);
        let front = &level.sectors[sub.sector];
        let (floorheight, floorpic) = (front.floorheight, front.floorpic as i32);
        let (ceilingheight, ceilingpic) = (front.ceilingheight, front.ceilingpic as i32);
        let lightlevel = front.lightlevel as i32;

        self.floorplane = if floorheight < rmain.viewz {
            Some(hooks.find_plane(floorheight, floorpic, lightlevel))
        } else {
            None
        };

        self.ceilingplane = if ceilingheight > rmain.viewz || ceilingpic == self.skyflatnum {
            Some(hooks.find_plane(ceilingheight, ceilingpic, lightlevel))
        } else {
            None
        };

        hooks.add_sprites(level, sub.sector);

        let count = sub.numlines as usize;
        let firstline = sub.firstline as usize;
        for line_offset in 0..count {
            self.add_line(hooks, rmain, rstate, level, firstline + line_offset);
        }
    }

    /// Port of `R_RenderBSPNode`. Renders all subsectors below a given
    /// node, traversing subtree recursively. Just call with BSP root
    /// (the original's own comment, preserved).
    pub fn render_bsp_node(
        &mut self,
        hooks: &mut impl RenderHooks,
        rmain: &RMain,
        rstate: &RState,
        level: &mut Level,
        bspnum: i32,
    ) {
        // Found a subsector?
        if bspnum & (crate::doomdata::NF_SUBSECTOR as i32) != 0 {
            if bspnum == -1 {
                self.subsector(hooks, rmain, rstate, level, 0);
            } else {
                self.subsector(
                    hooks,
                    rmain,
                    rstate,
                    level,
                    (bspnum & !(crate::doomdata::NF_SUBSECTOR as i32)) as usize,
                );
            }
            return;
        }

        let bsp = level.nodes[bspnum as usize];

        // Decide which side the view point is on.
        let side = RMain::point_on_side(rmain.viewx, rmain.viewy, &bsp);

        // Recursively divide front space.
        self.render_bsp_node(
            hooks,
            rmain,
            rstate,
            level,
            bsp.children[side as usize] as i32,
        );

        // Possibly divide back space.
        if self.check_bbox(rmain, rstate, &bsp.bbox[(side ^ 1) as usize]) {
            self.render_bsp_node(
                hooks,
                rmain,
                rstate,
                level,
                bsp.children[(side ^ 1) as usize] as i32,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_clip_segs_sets_sentinel_ranges() {
        let mut bsp = RBsp::new();
        bsp.clear_clip_segs(320);
        assert_eq!(bsp.solidsegs.len(), 2);
        assert_eq!(
            bsp.solidsegs[0],
            ClipRange {
                first: -0x7fffffff,
                last: -1
            }
        );
        assert_eq!(
            bsp.solidsegs[1],
            ClipRange {
                first: 320,
                last: 0x7fffffff
            }
        );
    }

    /// An `RBsp` mid-`R_AddLine` (so the clippers have a `curline`).
    fn bsp_with_curline() -> RBsp {
        let mut bsp = RBsp::new();
        bsp.clear_clip_segs(320);
        bsp.curline = Some(0);
        bsp
    }

    #[test]
    fn clip_solid_wall_segment_inserts_entirely_visible_range() {
        let mut bsp = bsp_with_curline();
        let mut level = Level::default();
        let mut hooks = RecordingHooks::default();

        bsp.clip_solid_wall_segment(&mut hooks, &mut level, 10, 50);

        assert_eq!(hooks.wall_ranges, vec![(10, 50)]);
        // The new range should now be recorded between the two sentinels.
        assert_eq!(bsp.solidsegs.len(), 3);
        assert_eq!(
            bsp.solidsegs[1],
            ClipRange {
                first: 10,
                last: 50
            }
        );
    }

    #[test]
    fn clip_solid_wall_segment_merges_overlapping_ranges() {
        let mut bsp = bsp_with_curline();
        let mut level = Level::default();
        let mut hooks = RecordingHooks::default();

        bsp.clip_solid_wall_segment(&mut hooks, &mut level, 10, 50);
        bsp.clip_solid_wall_segment(&mut hooks, &mut level, 40, 80);

        // Second call should have stored only the new fragment (51..80),
        // not re-drawn 40..50 which was already covered.
        assert_eq!(hooks.wall_ranges, vec![(10, 50), (51, 80)]);
    }

    #[test]
    fn clip_pass_wall_segment_does_not_modify_clip_list() {
        let mut bsp = bsp_with_curline();
        let mut level = Level::default();
        let mut hooks = RecordingHooks::default();

        bsp.clip_pass_wall_segment(&mut hooks, &mut level, 10, 50);

        assert_eq!(hooks.wall_ranges, vec![(10, 50)]);
        // Pass segments never get added to solidsegs.
        assert_eq!(bsp.solidsegs.len(), 2);
    }

    #[test]
    fn check_bbox_true_when_view_inside_box() {
        let bsp = RBsp::new();
        let rmain = RMain::new();
        // boxpos == 5 (view inside box) short-circuits to true.
        use crate::m_bbox::{BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};
        let mut bspcoord = [0; 4];
        bspcoord[BOXTOP] = 100 * crate::m_fixed::FRACUNIT;
        bspcoord[BOXBOTTOM] = -100 * crate::m_fixed::FRACUNIT;
        bspcoord[BOXLEFT] = -100 * crate::m_fixed::FRACUNIT;
        bspcoord[BOXRIGHT] = 100 * crate::m_fixed::FRACUNIT;
        assert!(bsp.check_bbox(&rmain, &RState::default(), &bspcoord));
    }

    /// Hooks that produce a drawseg for every wall range.
    struct DrawSegHooks(usize);

    impl RenderHooks for DrawSegHooks {
        fn store_wall_range(
            &mut self,
            _level: &mut Level,
            seg: usize,
            _rw_angle1: Angle,
            _floorplane: &mut Option<usize>,
            _ceilingplane: &mut Option<usize>,
            first: i32,
            last: i32,
        ) -> Option<DrawSeg> {
            self.0 += 1;
            Some(DrawSeg {
                curline: seg,
                x1: first,
                x2: last,
                scale1: 0,
                scale2: 0,
                scalestep: 0,
                silhouette: 0,
                bsilheight: 0,
                tsilheight: 0,
                sprtopclip: None,
                sprbottomclip: None,
                maskedtexturecol: None,
            })
        }

        fn find_plane(&mut self, _height: Fixed, _picnum: i32, _lightlevel: i32) -> usize {
            0
        }

        fn add_sprites(&mut self, _level: &mut Level, _sector: usize) {}
    }

    #[test]
    fn wall_ranges_past_maxdrawsegs_are_skipped() {
        let mut bsp = bsp_with_curline();
        let mut level = Level::default();
        let mut hooks = DrawSegHooks(0);

        // Pass segments don't fill the clip list, so every call reaches
        // R_StoreWallRange.
        for _ in 0..MAXDRAWSEGS + 10 {
            bsp.clip_pass_wall_segment(&mut hooks, &mut level, 10, 20);
        }

        assert_eq!(bsp.drawsegs.len(), MAXDRAWSEGS);
        assert_eq!(hooks.0, MAXDRAWSEGS, "no draw once drawsegs are full");
    }
}
