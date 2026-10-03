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
//	Movement/collision utility functions,
//	as used by function in p_map.c.
//	BLOCKMAP Iterator functions,
//	and some PIT_* functions to use for iteration.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_maputl.h` / `p_maputl.c`. Movement/collision
//! utility functions, as used by functions in `p_map.c`. BLOCKMAP
//! iterator functions, and some `PIT_*` functions to use for iteration
//! (the original's own description, preserved).
//!
//! `P_SetThingPosition`/`P_UnsetThingPosition` are *not* here — despite
//! living in `p_maputl.c` in the original, they only make sense
//! alongside the mobj/thinker arena they link/unlink, so they were
//! ported into `p_mobj.rs` back in Phase 6b (see that module's docs).
//!
//! # Representation: traversal state is a value, not four globals
//!
//! The original threads `P_PathTraverse`'s scan through four module
//! globals: `intercepts[MAXINTERCEPTS]`/`intercept_p` (the found-so-far
//! list), `trace` (the divline being traced) and `earlyout`. This port
//! collects them into [`PathTraverse`], passed explicitly rather than
//! reintroducing that shared mutable state — same information, same
//! scan order and early-out behavior, just not global. `traverser_t`
//! (a `boolean(*)(intercept_t*)` function pointer) becomes a generic
//! `FnMut(&Intercept) -> bool` closure parameter on
//! [`PathTraverse::traverse_intercepts`]/[`p_path_traverse`].
//!
//! `validcount` (used by [`p_block_lines_iterator`] to skip a line
//! that's already been checked once in this scan — a line can span
//! several blockmap cells) is [`crate::r_main::RMain::validcount`],
//! already ported in Phase 5 and already incremented once per rendered
//! frame; callers here bump it again before a scan, exactly like the
//! original bumps its `validcount` global before calling
//! `P_BlockLinesIterator`.

use crate::m_bbox::{BBox, BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};
use crate::m_fixed::{fixed_div, fixed_mul, Fixed, FRACBITS, FRACUNIT};
use crate::p_setup::{Level, MAPBLOCKSHIFT};
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_defs::{Line, SlopeType};
use crate::tables::Angle;

/// (`MAPBLOCKUNITS*FRACUNIT`)
pub const MAPBLOCKSIZE: i32 = 128 * FRACUNIT;
/// (`MAPBLOCKSHIFT-FRACBITS`)
pub const MAPBTOFRAC: i32 = MAPBLOCKSHIFT - FRACBITS;

/// Add lines to the intercepts list (`PT_ADDLINES`).
pub const PT_ADDLINES: i32 = 1;
/// Add things to the intercepts list (`PT_ADDTHINGS`).
pub const PT_ADDTHINGS: i32 = 2;
/// Stop as soon as a solid line is hit (`PT_EARLYOUT`).
pub const PT_EARLYOUT: i32 = 4;

/// Max intercepts, matching the original's `MAXINTERCEPTS`. The
/// original's `intercepts[]` array is a fixed-size overflow risk (a
/// long trace through a dense area could in principle exceed it,
/// silently corrupting adjacent memory in the original); this port
/// uses a growable `Vec` instead — same scan behavior, minus that
/// particular buffer-overflow failure mode, which was never
/// intentional in the original to begin with.
pub const MAXINTERCEPTS: usize = 128;

/// (`divline_t`)
#[derive(Debug, Clone, Copy, Default)]
pub struct Divline {
    pub x: Fixed,
    pub y: Fixed,
    pub dx: Fixed,
    pub dy: Fixed,
}

/// What an [`Intercept`] hit — a line or a mobj (`intercept_t`'s
/// `isaline`-tagged union, made an explicit Rust enum instead).
#[derive(Debug, Clone, Copy)]
pub enum InterceptTarget {
    Line(usize),
    Thing(ThinkerId),
}

/// (`intercept_t`)
#[derive(Debug, Clone, Copy)]
pub struct Intercept {
    /// Along the trace line.
    pub frac: Fixed,
    pub target: InterceptTarget,
}

/// Port of `P_AproxDistance`. Gives an estimation of distance (not
/// exact) (the original's own comment, preserved).
pub fn p_aprox_distance(dx: Fixed, dy: Fixed) -> Fixed {
    let dx = dx.wrapping_abs();
    let dy = dy.wrapping_abs();
    if dx < dy {
        dx.wrapping_add(dy).wrapping_sub(dx >> 1)
    } else {
        dx.wrapping_add(dy).wrapping_sub(dy >> 1)
    }
}

/// Port of `P_PointOnLineSide`. Returns 0 or 1.
pub fn p_point_on_line_side(x: Fixed, y: Fixed, level: &Level, line: &Line) -> i32 {
    let v1 = level.vertexes[line.v1];

    if line.dx == 0 {
        if x <= v1.x {
            return (line.dy > 0) as i32;
        }
        return (line.dy < 0) as i32;
    }
    if line.dy == 0 {
        if y <= v1.y {
            return (line.dx < 0) as i32;
        }
        return (line.dx > 0) as i32;
    }

    let dx = x.wrapping_sub(v1.x);
    let dy = y.wrapping_sub(v1.y);

    let left = fixed_mul(line.dy >> FRACBITS, dx);
    let right = fixed_mul(dy, line.dx >> FRACBITS);

    if right < left {
        0 // front side
    } else {
        1 // back side
    }
}

/// Port of `P_BoxOnLineSide`. Considers the line to be infinite. Returns
/// side 0 or 1, -1 if box crosses the line (the original's own comment,
/// preserved).
pub fn p_box_on_line_side(tmbox: &BBox, level: &Level, line: &Line) -> i32 {
    let v1 = level.vertexes[line.v1];

    let (p1, p2) = match line.slopetype {
        SlopeType::Horizontal => {
            let mut p1 = (tmbox[BOXTOP] > v1.y) as i32;
            let mut p2 = (tmbox[BOXBOTTOM] > v1.y) as i32;
            if line.dx < 0 {
                p1 ^= 1;
                p2 ^= 1;
            }
            (p1, p2)
        }
        SlopeType::Vertical => {
            let mut p1 = (tmbox[BOXRIGHT] < v1.x) as i32;
            let mut p2 = (tmbox[BOXLEFT] < v1.x) as i32;
            if line.dy < 0 {
                p1 ^= 1;
                p2 ^= 1;
            }
            (p1, p2)
        }
        SlopeType::Positive => (
            p_point_on_line_side(tmbox[BOXLEFT], tmbox[BOXTOP], level, line),
            p_point_on_line_side(tmbox[BOXRIGHT], tmbox[BOXBOTTOM], level, line),
        ),
        SlopeType::Negative => (
            p_point_on_line_side(tmbox[BOXRIGHT], tmbox[BOXTOP], level, line),
            p_point_on_line_side(tmbox[BOXLEFT], tmbox[BOXBOTTOM], level, line),
        ),
    };

    if p1 == p2 {
        p1
    } else {
        -1
    }
}

/// Port of `P_PointOnDivlineSide`. Returns 0 or 1.
pub fn p_point_on_divline_side(x: Fixed, y: Fixed, line: &Divline) -> i32 {
    if line.dx == 0 {
        if x <= line.x {
            return (line.dy > 0) as i32;
        }
        return (line.dy < 0) as i32;
    }
    if line.dy == 0 {
        if y <= line.y {
            return (line.dx < 0) as i32;
        }
        return (line.dx > 0) as i32;
    }

    let dx = x.wrapping_sub(line.x);
    let dy = y.wrapping_sub(line.y);

    // try to quickly decide by looking at sign bits (the original's own
    // comment, preserved)
    if ((line.dy ^ line.dx ^ dx ^ dy) as u32) & 0x8000_0000 != 0 {
        return (((line.dy ^ dx) as u32) & 0x8000_0000 != 0) as i32;
    }

    let left = fixed_mul(line.dy >> 8, dx >> 8);
    let right = fixed_mul(dy >> 8, line.dx >> 8);

    if right < left {
        0 // front side
    } else {
        1 // back side
    }
}

/// Port of `P_MakeDivline`.
pub fn p_make_divline(level: &Level, line: &Line) -> Divline {
    let v1 = level.vertexes[line.v1];
    Divline {
        x: v1.x,
        y: v1.y,
        dx: line.dx,
        dy: line.dy,
    }
}

/// Port of `P_InterceptVector`. Returns the fractional intercept point
/// along the first divline. This is only called by the addthings and
/// addlines traversers (the original's own comment, preserved). The
/// original's `#if 0` float-debug branch isn't ported (dead code in the
/// original too).
pub fn p_intercept_vector(v2: &Divline, v1: &Divline) -> Fixed {
    let den = fixed_mul(v1.dy >> 8, v2.dx).wrapping_sub(fixed_mul(v1.dx >> 8, v2.dy));

    if den == 0 {
        return 0;
    }

    let num = fixed_mul(v1.x.wrapping_sub(v2.x) >> 8, v1.dy)
        .wrapping_add(fixed_mul(v2.y.wrapping_sub(v1.y) >> 8, v1.dx));

    fixed_div(num, den)
}

/// The window through a two-sided line, standing in for the original's
/// `opentop`/`openbottom`/`openrange`/`lowfloor` globals (`P_LineOpening`
/// sets opentop and openbottom to the window through a two sided line —
/// the original's own comment, preserved).
#[derive(Debug, Clone, Copy, Default)]
pub struct LineOpening {
    pub opentop: Fixed,
    pub openbottom: Fixed,
    pub openrange: Fixed,
    pub lowfloor: Fixed,
}

/// Port of `P_LineOpening`.
pub fn p_line_opening(level: &Level, line: &Line) -> LineOpening {
    if line.sidenum[1].is_none() {
        // single sided line (the original's own comment, preserved).
        return LineOpening {
            openrange: 0,
            ..Default::default()
        };
    }

    let front = &level.sectors[line.frontsector.unwrap()];
    let back = &level.sectors[line.backsector.unwrap()];

    let opentop = front.ceilingheight.min(back.ceilingheight);
    let (openbottom, lowfloor) = if front.floorheight > back.floorheight {
        (front.floorheight, back.floorheight)
    } else {
        (back.floorheight, front.floorheight)
    };

    LineOpening {
        opentop,
        openbottom,
        openrange: opentop.wrapping_sub(openbottom),
        lowfloor,
    }
}

thread_local! {
    /// The original's global `validcount` (starts at 1).
    static VALIDCOUNT: std::cell::Cell<i32> = const { std::cell::Cell::new(1) };
}

/// `validcount++`: the original's global "already looked at this line/
/// sector in this traversal" counter, bumped at the start of every
/// `P_CheckPosition`/`P_PathTraverse`/`P_CheckSight`/`P_NoiseAlert`.
/// Returns the new value.
///
/// The port used to thread one `validcount` value per *tic* through
/// every signature; that let a second traversal in the same tic (a
/// slide-move retry, another mobj moving) skip lines the first one had
/// marked — walls that stopped the first mover let the next through.
/// The functions that bump now take their value from here, and the
/// `validcount: i32` parameters left in their signatures are ignored.
pub fn bump_validcount() -> i32 {
    VALIDCOUNT.with(|v| {
        let n = v.get().wrapping_add(1);
        v.set(n);
        n
    })
}

/// Port of `P_BlockLinesIterator`. The validcount flags are used to
/// avoid checking lines that are marked in multiple mapblocks, so
/// increment validcount before the first call to
/// `P_BlockLinesIterator`, then make one or more calls to it (the
/// original's own comment, preserved). Returns `false` (matching the
/// original's early-out) as soon as `func` returns `false`.
pub fn p_block_lines_iterator(
    level: &mut Level,
    validcount: i32,
    x: i32,
    y: i32,
    mut func: impl FnMut(&mut Level, usize) -> bool,
) -> bool {
    if x < 0 || y < 0 || x >= level.bmapwidth || y >= level.bmapheight {
        return true;
    }

    let offset = (y * level.bmapwidth + x) as usize;
    let offset = level.blockmap[offset] as usize;

    let mut i = offset;
    loop {
        let entry = level.blockmaplump[i];
        if entry == -1 {
            break;
        }
        let line_idx = entry as usize;

        if level.lines[line_idx].validcount == validcount {
            i += 1;
            continue; // line has already been checked
        }
        level.lines[line_idx].validcount = validcount;

        if !func(level, line_idx) {
            return false;
        }
        i += 1;
    }
    true // everything was checked
}

/// Port of `P_BlockThingsIterator`.
pub fn p_block_things_iterator(
    thinkers: &mut Thinkers,
    level: &Level,
    x: i32,
    y: i32,
    mut func: impl FnMut(&mut Thinkers, ThinkerId) -> bool,
) -> bool {
    if x < 0 || y < 0 || x >= level.bmapwidth || y >= level.bmapheight {
        return true;
    }

    let mut cur = level.blocklinks[(y * level.bmapwidth + x) as usize];
    while let Some(id) = cur {
        cur = thinkers.mobj(id).unwrap().bnext;
        if !func(thinkers, id) {
            return false;
        }
    }
    true
}

/// The scan state for [`p_path_traverse`], standing in for the
/// original's `intercepts`/`intercept_p`/`trace`/`earlyout` globals —
/// see module docs.
#[derive(Debug, Clone, Copy, Default)]
pub struct PathTraverse {
    pub trace: Divline,
    pub earlyout: bool,
}

/// Found-so-far intercepts, alongside a [`PathTraverse`] scan — split
/// out from it since [`PIT_AddLineIntercepts`]/[`PIT_AddThingIntercepts`]
/// need `&PathTraverse` (to read `trace`/`earlyout`) at the same time as
/// `&mut Vec<Intercept>` (to push to it), which a single struct
/// couldn't lend out both halves of at once.
pub type Intercepts = Vec<Intercept>;

/// Port of `PIT_AddLineIntercepts`. Looks for lines in the given block
/// that intercept the given trace to add to the intercepts list. A line
/// is crossed if its endpoints are on opposite sides of the trace.
/// Returns true if earlyout and a solid line hit (the original's own
/// comment, preserved... mostly — the original's doc comment is stale,
/// see the original itself; kept verbatim for fidelity to the source).
pub fn pit_add_line_intercepts(
    scan: &PathTraverse,
    intercepts: &mut Intercepts,
    level: &Level,
    line_idx: usize,
) -> bool {
    let line = &level.lines[line_idx];
    let trace = &scan.trace;

    // avoid precision problems with two routines (the original's own
    // comment, preserved)
    let (s1, s2) = if trace.dx > FRACUNIT * 16
        || trace.dy > FRACUNIT * 16
        || trace.dx < -FRACUNIT * 16
        || trace.dy < -FRACUNIT * 16
    {
        let v1 = level.vertexes[line.v1];
        let v2 = level.vertexes[line.v2];
        (
            p_point_on_divline_side(v1.x, v1.y, trace),
            p_point_on_divline_side(v2.x, v2.y, trace),
        )
    } else {
        (
            p_point_on_line_side(trace.x, trace.y, level, line),
            p_point_on_line_side(
                trace.x.wrapping_add(trace.dx),
                trace.y.wrapping_add(trace.dy),
                level,
                line,
            ),
        )
    };

    if s1 == s2 {
        return true; // line isn't crossed
    }

    // hit the line
    let dl = p_make_divline(level, line);
    let frac = p_intercept_vector(trace, &dl);

    if frac < 0 {
        return true; // behind source
    }

    // try to early out the check (the original's own comment, preserved)
    if scan.earlyout && frac < FRACUNIT && line.backsector.is_none() {
        return false; // stop checking
    }

    intercepts.push(Intercept {
        frac,
        target: InterceptTarget::Line(line_idx),
    });

    true // continue
}

/// Port of `PIT_AddThingIntercepts`.
pub fn pit_add_thing_intercepts(
    scan: &PathTraverse,
    intercepts: &mut Intercepts,
    thinkers: &Thinkers,
    id: ThinkerId,
) -> bool {
    let trace = &scan.trace;
    let thing = thinkers.mobj(id).unwrap();
    let tracepositive = (trace.dx ^ trace.dy) > 0;

    // check a corner to corner crossection for hit (the original's own
    // comment, preserved)
    let (x1, y1, x2, y2) = if tracepositive {
        (
            thing.x.wrapping_sub(thing.radius),
            thing.y.wrapping_add(thing.radius),
            thing.x.wrapping_add(thing.radius),
            thing.y.wrapping_sub(thing.radius),
        )
    } else {
        (
            thing.x.wrapping_sub(thing.radius),
            thing.y.wrapping_sub(thing.radius),
            thing.x.wrapping_add(thing.radius),
            thing.y.wrapping_add(thing.radius),
        )
    };

    let s1 = p_point_on_divline_side(x1, y1, trace);
    let s2 = p_point_on_divline_side(x2, y2, trace);

    if s1 == s2 {
        return true; // line isn't crossed
    }

    let dl = Divline {
        x: x1,
        y: y1,
        dx: x2.wrapping_sub(x1),
        dy: y2.wrapping_sub(y1),
    };

    let frac = p_intercept_vector(trace, &dl);

    if frac < 0 {
        return true; // behind source
    }

    intercepts.push(Intercept {
        frac,
        target: InterceptTarget::Thing(id),
    });

    true // keep going
}

/// Port of `P_TraverseIntercepts`. Returns true if the traverser
/// function returns true for all lines (the original's own comment,
/// preserved). `func` also receives `level`/`thinkers` (unlike the
/// original's plain `traverser_t`) so callers like
/// [`crate::p_map::ptr_slide_traverse`] can look up the hit line/thing
/// without capturing `&Level`/`&Thinkers` themselves — see
/// [`p_path_traverse`]'s docs on why that capture would conflict with
/// its own `&mut Level`/`&mut Thinkers` parameters.
pub fn p_traverse_intercepts(
    intercepts: &mut Intercepts,
    level: &Level,
    thinkers: &Thinkers,
    maxfrac: Fixed,
    mut func: impl FnMut(&Level, &Thinkers, &Intercept) -> bool,
) -> bool {
    // The original picks the closest remaining intercept each pass by
    // linear scan (an O(n^2) selection sort) rather than sorting once —
    // reproduced as-is: `func` may append more intercepts as it runs
    // (not exercised by any Phase 6c caller, but the original allows
    // it), which a single upfront sort couldn't account for.
    let mut done = vec![false; intercepts.len()];
    let mut remaining = intercepts.len();

    while remaining > 0 {
        let mut best: Option<usize> = None;
        let mut best_frac = Fixed::MAX;
        for (i, ic) in intercepts.iter().enumerate() {
            if i < done.len() && done[i] {
                continue;
            }
            if ic.frac < best_frac {
                best_frac = ic.frac;
                best = Some(i);
            }
        }

        let Some(i) = best else { break };

        if best_frac > maxfrac {
            return true; // checked everything in range
        }

        if !func(level, thinkers, &intercepts[i]) {
            return false; // don't bother going farther
        }

        if i >= done.len() {
            done.resize(intercepts.len(), false);
        }
        done[i] = true;
        remaining -= 1;
    }

    true // everything was traversed
}

/// Port of `P_PathTraverse`. Traces a line from x1,y1 to x2,y2, calling
/// the traverser function for each (the original's own comment,
/// preserved). `validcount` is bumped by the caller's [`Level`]-wide
/// counter exactly like the original bumps its global — see module
/// docs.
#[allow(clippy::too_many_arguments)]
pub fn p_path_traverse(
    level: &mut Level,
    thinkers: &mut Thinkers,
    _validcount: i32,
    mut x1: Fixed,
    mut y1: Fixed,
    x2: Fixed,
    y2: Fixed,
    flags: i32,
    mut trav: impl FnMut(&Level, &Thinkers, &Intercept) -> bool,
) -> bool {
    let earlyout = flags & PT_EARLYOUT != 0;
    let validcount = bump_validcount();

    if (x1.wrapping_sub(level.bmaporgx)) & (MAPBLOCKSIZE - 1) == 0 {
        x1 = x1.wrapping_add(FRACUNIT); // don't side exactly on a line
    }
    if (y1.wrapping_sub(level.bmaporgy)) & (MAPBLOCKSIZE - 1) == 0 {
        y1 = y1.wrapping_add(FRACUNIT);
    }

    let scan = PathTraverse {
        trace: Divline {
            x: x1,
            y: y1,
            dx: x2.wrapping_sub(x1),
            dy: y2.wrapping_sub(y1),
        },
        earlyout,
    };
    let mut intercepts: Intercepts = Vec::with_capacity(MAXINTERCEPTS);

    let bx1 = x1.wrapping_sub(level.bmaporgx);
    let by1 = y1.wrapping_sub(level.bmaporgy);
    let xt1 = bx1 >> MAPBLOCKSHIFT;
    let yt1 = by1 >> MAPBLOCKSHIFT;

    let bx2 = x2.wrapping_sub(level.bmaporgx);
    let by2 = y2.wrapping_sub(level.bmaporgy);
    let xt2 = bx2 >> MAPBLOCKSHIFT;
    let yt2 = by2 >> MAPBLOCKSHIFT;

    // x-major step: how mapx advances per block, and how much y changes
    // per unit x (the original's own `mapxstep`/`ystep`/`partial`).
    let (mapxstep, ystep, x_partial) = if xt2 > xt1 {
        let partial = FRACUNIT - ((bx1 >> MAPBTOFRAC) & (FRACUNIT - 1));
        let ystep = fixed_div(y2.wrapping_sub(y1), (x2.wrapping_sub(x1)).wrapping_abs());
        (1, ystep, partial)
    } else if xt2 < xt1 {
        let partial = (bx1 >> MAPBTOFRAC) & (FRACUNIT - 1);
        let ystep = fixed_div(y2.wrapping_sub(y1), (x2.wrapping_sub(x1)).wrapping_abs());
        (-1, ystep, partial)
    } else {
        (0, 256 * FRACUNIT, FRACUNIT)
    };
    let mut yintercept = (by1 >> MAPBTOFRAC).wrapping_add(fixed_mul(x_partial, ystep));

    // y-major step: how mapy advances per block, and how much x changes
    // per unit y (the original's own `mapystep`/`xstep`/`partial`).
    let (mapystep, xstep, y_partial) = if yt2 > yt1 {
        let partial = FRACUNIT - ((by1 >> MAPBTOFRAC) & (FRACUNIT - 1));
        let xstep = fixed_div(x2.wrapping_sub(x1), (y2.wrapping_sub(y1)).wrapping_abs());
        (1, xstep, partial)
    } else if yt2 < yt1 {
        let partial = (by1 >> MAPBTOFRAC) & (FRACUNIT - 1);
        let xstep = fixed_div(x2.wrapping_sub(x1), (y2.wrapping_sub(y1)).wrapping_abs());
        (-1, xstep, partial)
    } else {
        (0, 256 * FRACUNIT, FRACUNIT)
    };
    let mut xintercept = (bx1 >> MAPBTOFRAC).wrapping_add(fixed_mul(y_partial, xstep));

    // Step through map blocks. Count is present to prevent a round off
    // error from skipping the break (the original's own comment,
    // preserved).
    let mut mapx = xt1;
    let mut mapy = yt1;

    for _ in 0..64 {
        if flags & PT_ADDLINES != 0 {
            let ok = p_block_lines_iterator(level, validcount, mapx, mapy, |level, line_idx| {
                pit_add_line_intercepts(&scan, &mut intercepts, level, line_idx)
            });
            if !ok {
                return false; // early out
            }
        }

        if flags & PT_ADDTHINGS != 0 {
            let ok = p_block_things_iterator(thinkers, level, mapx, mapy, |thinkers, id| {
                pit_add_thing_intercepts(&scan, &mut intercepts, thinkers, id)
            });
            if !ok {
                return false; // early out
            }
        }

        if mapx == xt2 && mapy == yt2 {
            break;
        }

        if (yintercept >> FRACBITS) == mapy {
            yintercept = yintercept.wrapping_add(ystep);
            mapx += mapxstep;
        } else if (xintercept >> FRACBITS) == mapx {
            xintercept = xintercept.wrapping_add(xstep);
            mapy += mapystep;
        }
    }

    // go through the sorted list (the original's own comment, preserved)
    p_traverse_intercepts(&mut intercepts, level, thinkers, FRACUNIT, &mut trav)
}

/// Unused by any Phase 6c caller, but kept for parity/documentation:
/// the original's `R_PointToAngle2`-based line angle used by
/// `P_HitSlideLine` (`p_map.rs`) needs `RMain::point_to_angle2`, which
/// mutates `viewx`/`viewy` as a side effect — see that function's docs.
/// This type alias just documents which `Angle` producer `p_map.rs`
/// must use, so it isn't rediscovered as a bug later.
pub type LineAngle = Angle;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aprox_distance_matches_original_formula() {
        // dx < dy: dx + dy - (dx>>1).
        assert_eq!(
            p_aprox_distance(3 * FRACUNIT, 4 * FRACUNIT),
            3 * FRACUNIT + 4 * FRACUNIT - (3 * FRACUNIT) / 2
        );
        assert_eq!(
            p_aprox_distance(-3 * FRACUNIT, 4 * FRACUNIT),
            3 * FRACUNIT + 4 * FRACUNIT - (3 * FRACUNIT) / 2
        );
        // dx >= dy: dx + dy - (dy>>1).
        assert_eq!(
            p_aprox_distance(4 * FRACUNIT, 3 * FRACUNIT),
            4 * FRACUNIT + 3 * FRACUNIT - (3 * FRACUNIT) / 2
        );
    }

    #[test]
    fn intercept_vector_of_parallel_lines_is_zero() {
        let v1 = Divline {
            x: 0,
            y: 0,
            dx: FRACUNIT,
            dy: 0,
        };
        let v2 = Divline {
            x: 0,
            y: FRACUNIT,
            dx: FRACUNIT,
            dy: 0,
        };
        assert_eq!(p_intercept_vector(&v2, &v1), 0);
    }

    #[test]
    fn line_opening_of_single_sided_line_has_zero_range() {
        let mut level = Level::default();
        level.vertexes.push(crate::r_defs::Vertex { x: 0, y: 0 });
        level
            .vertexes
            .push(crate::r_defs::Vertex { x: FRACUNIT, y: 0 });
        let line = crate::r_defs::Line {
            v1: 0,
            v2: 1,
            ..Default::default()
        };
        let opening = p_line_opening(&level, &line);
        assert_eq!(opening.openrange, 0);
    }
}
