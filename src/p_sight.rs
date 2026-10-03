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
//	LineOfSight/Visibility checks, uses REJECT Lookup Table.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_sight.c`. Line-of-sight/visibility checks,
//! using the REJECT lookup table.
//!
//! `sightzstart`/`topslope`/`bottomslope`/`strace`/`t2x`/`t2y` (the
//! original's file-scope scratch globals, written once per
//! [`p_check_sight`] call and read by the recursive
//! [`p_cross_subsector`]/[`p_cross_bsp_node`] helpers it calls) are
//! bundled into a local [`SightTrace`] struct threaded through those
//! helpers by `&mut` — same convention as `p_maputl.rs`'s
//! `LineOpening`/`MoveContext` for "the original's per-call scratch
//! globals". `sightcounts` (a debug/profiling counter, never read
//! anywhere else in the original) isn't ported.
//!
//! `validcount` is threaded as an explicit parameter, matching the
//! project-wide convention (see `p_map.rs`/`p_maputl.rs`) rather than
//! living in `doomstat`.

use crate::m_fixed::{fixed_div, fixed_mul, Fixed, FRACBITS};
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_defs::Line;

/// A divline, minimal enough for this file's needs (the original's
/// `divline_t`, reused here from `p_maputl.rs`'s BSP-adjacent uses
/// instead of redefining it, since sight tracing needs exactly the same
/// four fields).
#[derive(Debug, Clone, Copy, Default)]
struct Divline {
    x: Fixed,
    y: Fixed,
    dx: Fixed,
    dy: Fixed,
}

/// The original's file-scope scratch globals for one [`p_check_sight`]
/// call, bundled together. See module docs.
struct SightTrace {
    sightzstart: Fixed,
    topslope: Fixed,
    bottomslope: Fixed,
    strace: Divline,
    t2x: Fixed,
    t2y: Fixed,
}

/// Port of `P_DivlineSide`. Returns side 0 (front), 1 (back), or 2 (on).
fn p_divline_side(x: Fixed, y: Fixed, node: &Divline) -> i32 {
    if node.dx == 0 {
        if x == node.x {
            return 2;
        }
        if x <= node.x {
            return (node.dy > 0) as i32;
        }
        return (node.dy < 0) as i32;
    }

    if node.dy == 0 {
        if x == node.y {
            return 2;
        }
        if y <= node.y {
            return (node.dx < 0) as i32;
        }
        return (node.dx > 0) as i32;
    }

    let dx = x - node.x;
    let dy = y - node.y;

    let left = (node.dy >> FRACBITS) * (dx >> FRACBITS);
    let right = (dy >> FRACBITS) * (node.dx >> FRACBITS);

    if right < left {
        return 0; // front side
    }
    if left == right {
        return 2;
    }
    1 // back side
}

/// Port of `P_InterceptVector2`. Returns the fractional intercept point
/// along the first divline. This is only called by [`p_cross_subsector`]
/// (the original's own comment, preserved — it originally also
/// mentioned the addthings/addlines traversers, which are `p_maputl.c`'s
/// own `P_InterceptVector`, a distinct function not ported here).
fn p_intercept_vector2(v2: &Divline, v1: &Divline) -> Fixed {
    let den = fixed_mul(v1.dy >> 8, v2.dx) - fixed_mul(v1.dx >> 8, v2.dy);

    if den == 0 {
        return 0; // I_Error ("P_InterceptVector: parallel"); — the
                  // original's own commented-out error, preserved as a
                  // comment; it returns 0 on the parallel case instead.
    }

    let num = fixed_mul((v1.x - v2.x) >> 8, v1.dy) + fixed_mul((v2.y - v1.y) >> 8, v1.dx);
    fixed_div(num, den)
}

/// Port of `P_CrossSubsector`. Returns `true` if `trace.strace` crosses
/// the given subsector successfully.
fn p_cross_subsector(
    level: &mut crate::p_setup::Level,
    validcount: i32,
    trace: &mut SightTrace,
    num: usize,
) -> bool {
    let sub = level.subsectors[num];

    for i in 0..sub.numlines {
        let seg = level.segs[sub.firstline as usize + i as usize];
        let line_idx = seg.linedef;

        // allready checked other side? (the original's own comment,
        // preserved).
        if level.lines[line_idx].validcount == validcount {
            continue;
        }
        level.lines[line_idx].validcount = validcount;

        let line: Line = level.lines[line_idx];
        let v1 = level.vertexes[line.v1];
        let v2 = level.vertexes[line.v2];

        let s1 = p_divline_side(v1.x, v1.y, &trace.strace);
        let s2 = p_divline_side(v2.x, v2.y, &trace.strace);

        // line isn't crossed? (the original's own comment, preserved).
        if s1 == s2 {
            continue;
        }

        let divl = Divline {
            x: v1.x,
            y: v1.y,
            dx: v2.x - v1.x,
            dy: v2.y - v1.y,
        };
        let s1 = p_divline_side(trace.strace.x, trace.strace.y, &divl);
        let s2 = p_divline_side(trace.t2x, trace.t2y, &divl);

        // line isn't crossed? (the original's own comment, preserved).
        if s1 == s2 {
            continue;
        }

        // stop because it is not two sided anyway. might do this after
        // updating validcount? (the original's own comment, preserved).
        if line.flags & crate::doomdata::ML_TWOSIDED == 0 {
            return false;
        }

        // crosses a two sided line (the original's own comment,
        // preserved).
        let front_idx = line.frontsector.unwrap();
        let back_idx = line.backsector.unwrap();
        let front = &level.sectors[front_idx];
        let back = &level.sectors[back_idx];

        // no wall to block sight with? (the original's own comment,
        // preserved).
        if front.floorheight == back.floorheight && front.ceilingheight == back.ceilingheight {
            continue;
        }

        // possible occluder because of ceiling height differences (the
        // original's own comment, preserved).
        let opentop = front.ceilingheight.min(back.ceilingheight);
        // because of ceiling height differences (the original's own
        // comment, preserved — this computes the open bottom, despite
        // the copy-pasted comment about ceilings, faithfully preserved
        // from the original).
        let openbottom = front.floorheight.max(back.floorheight);

        // quick test for totally closed doors (the original's own
        // comment, preserved).
        if openbottom >= opentop {
            return false; // stop
        }

        let frac = p_intercept_vector2(&trace.strace, &divl);

        if front.floorheight != back.floorheight {
            let slope = fixed_div(openbottom - trace.sightzstart, frac);
            if slope > trace.bottomslope {
                trace.bottomslope = slope;
            }
        }

        if front.ceilingheight != back.ceilingheight {
            let slope = fixed_div(opentop - trace.sightzstart, frac);
            if slope < trace.topslope {
                trace.topslope = slope;
            }
        }

        if trace.topslope <= trace.bottomslope {
            return false; // stop
        }
    }
    // passed the subsector ok (the original's own comment, preserved).
    true
}

/// Port of `P_CrossBSPNode`. Returns `true` if `trace.strace` crosses
/// the given node successfully.
fn p_cross_bsp_node(
    level: &mut crate::p_setup::Level,
    validcount: i32,
    trace: &mut SightTrace,
    bspnum: i32,
) -> bool {
    if bspnum & (crate::doomdata::NF_SUBSECTOR as i32) != 0 {
        let num = if bspnum == -1 {
            0
        } else {
            (bspnum & !(crate::doomdata::NF_SUBSECTOR as i32)) as usize
        };
        return p_cross_subsector(level, validcount, trace, num);
    }

    let bsp = level.nodes[bspnum as usize];
    let partition = Divline {
        x: bsp.x,
        y: bsp.y,
        dx: bsp.dx,
        dy: bsp.dy,
    };

    // decide which side the start point is on (the original's own
    // comment, preserved).
    let mut side = p_divline_side(trace.strace.x, trace.strace.y, &partition);
    if side == 2 {
        side = 0; // an "on" should cross both sides (the original's own
                  // comment, preserved).
    }

    // cross the starting side (the original's own comment, preserved).
    if !p_cross_bsp_node(level, validcount, trace, bsp.children[side as usize] as i32) {
        return false;
    }

    // the partition plane is crossed here (the original's own comment,
    // preserved).
    if side == p_divline_side(trace.t2x, trace.t2y, &partition) {
        // the line doesn't touch the other side (the original's own
        // comment, preserved).
        return true;
    }

    // cross the ending side (the original's own comment, preserved).
    p_cross_bsp_node(
        level,
        validcount,
        trace,
        bsp.children[side as usize ^ 1] as i32,
    )
}

/// Port of `P_CheckSight`. Returns `true` if a straight line between
/// `t1` and `t2` is unobstructed. Uses REJECT.
///
/// `validcount` stands in for the original's global counter — callers
/// own it and must persist the post-call value the same way every other
/// `validcount`-threading function in this port does (this function
/// bumps it by exactly 1, like the original's `validcount++`).
pub fn p_check_sight(
    thinkers: &Thinkers,
    level: &mut crate::p_setup::Level,
    validcount: i32,
    t1: ThinkerId,
    t2: ThinkerId,
) -> (bool, i32) {
    let t1 = *thinkers.mobj(t1).unwrap();
    let t2 = *thinkers.mobj(t2).unwrap();

    // First check for trivial rejection. Determine subsector entries in
    // REJECT table (the original's own comment, preserved).
    let s1 = level.subsectors[t1.subsector.unwrap()].sector;
    let s2 = level.subsectors[t2.subsector.unwrap()].sector;
    let numsectors = level.sectors.len();
    let pnum = s1 * numsectors + s2;
    let bytenum = pnum >> 3;
    let bitnum = 1u8 << (pnum & 7);

    // Check in REJECT table (the original's own comment, preserved).
    if level.rejectmatrix[bytenum] & bitnum != 0 {
        // can't possibly be connected (the original's own comment,
        // preserved).
        return (false, validcount);
    }

    // An unobstructed LOS is possible. Now look from eyes of t1 to any
    // part of t2 (the original's own comment, preserved).
    let validcount = crate::p_maputl::bump_validcount();

    let sightzstart = t1.z + t1.height - (t1.height >> 2);
    let mut trace = SightTrace {
        sightzstart,
        topslope: (t2.z + t2.height) - sightzstart,
        bottomslope: t2.z - sightzstart,
        strace: Divline {
            x: t1.x,
            y: t1.y,
            dx: t2.x - t1.x,
            dy: t2.y - t1.y,
        },
        t2x: t2.x,
        t2y: t2.y,
    };

    // the head node is the last node output (the original's own
    // comment, preserved).
    let numnodes = level.nodes.len();
    let result = p_cross_bsp_node(level, validcount, &mut trace, numnodes as i32 - 1);
    (result, validcount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::MobjType;
    use crate::p_mobj::{p_spawn_mobj, SpawnZ};
    use crate::w_wad::WadFiles;

    fn load_e1m1() -> Option<(WadFiles, crate::p_setup::Level)> {
        let candidates = ["doom.wad"];
        let path = candidates
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut wad = WadFiles::new();
        wad.init_file(&path);
        let level = crate::p_setup::Level::load(&mut wad, "E1M1");
        Some((wad, level))
    }

    // Player 1's start, from E1M1 (see tests/real_frame.rs).
    const START_X: Fixed = 1056 * crate::m_fixed::FRACUNIT;
    const START_Y: Fixed = -3616 * crate::m_fixed::FRACUNIT;

    #[test]
    fn check_sight_of_self_is_always_true() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let id = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let (sight, _validcount) = p_check_sight(&thinkers, &mut level, 0, id, id);
        assert!(sight, "a mobj must always see itself (same subsector)");
    }

    #[test]
    fn check_sight_of_nearby_mobj_in_same_room_is_true() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let a = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        // A few units away, still in the spawn room: nothing should
        // occlude line of sight this close.
        let b = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X + 32 * crate::m_fixed::FRACUNIT,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let (sight, _validcount) = p_check_sight(&thinkers, &mut level, 0, a, b);
        assert!(
            sight,
            "two mobjs a few units apart in the same room should see each other"
        );
    }

    #[test]
    fn check_sight_bumps_validcount_by_one_on_success() {
        let Some((_wad, mut level)) = load_e1m1() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut thinkers = Thinkers::new();
        let id = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            START_X,
            START_Y,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let before = crate::p_maputl::bump_validcount();
        let (sight, validcount) = p_check_sight(&thinkers, &mut level, 41, id, id);
        assert!(sight);
        assert_eq!(
            validcount,
            before + 1,
            "P_CheckSight bumps validcount by 1 on a non-trivially-rejected check"
        );
    }

    #[test]
    fn divline_side_on_the_line_returns_two() {
        let node = Divline {
            x: 0,
            y: 0,
            dx: 10 * crate::m_fixed::FRACUNIT,
            dy: 0,
        };
        assert_eq!(p_divline_side(0, 0, &node), 2);
    }

    #[test]
    fn divline_side_front_and_back_are_opposite() {
        // A diagonal partition line (both dx and dy nonzero) so the
        // general-case branch of `P_DivlineSide` runs, not one of its
        // axis-aligned special cases.
        let node = Divline {
            x: 0,
            y: 0,
            dx: 10 * crate::m_fixed::FRACUNIT,
            dy: 10 * crate::m_fixed::FRACUNIT,
        };
        let front = p_divline_side(10 * crate::m_fixed::FRACUNIT, 0, &node);
        let back = p_divline_side(0, 10 * crate::m_fixed::FRACUNIT, &node);
        assert_ne!(front, back);
        assert_ne!(front, 2);
        assert_ne!(back, 2);
    }
}
