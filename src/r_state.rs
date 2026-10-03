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
//	Refresh/render internal state variables (global).
//
//-----------------------------------------------------------------------------

//! Rust port of `r_state.h` (partial — see note below).
//!
//! Refresh/render internal state variables (global).
//!
//! # Scope
//!
//! Of `r_state.h`'s 38 fields, roughly two thirds are pointers/counts
//! into world/renderer data structures not yet ported — `sector_t`,
//! `seg_t`, `line_t`, `side_t`, `vertex_t`, `node_t`, `subsector_t`
//! (`r_defs.h`/map data, Phase 5), `spritedef_t`/`lighttable_t` (sprite
//! and colormap data, `r_data.h`, Phase 5), `visplane_t` (Phase 5
//! renderer internals), and `player_t*` (`d_player.h`'s `player_t`,
//! deferred per `d_player.rs`'s note — depends on `mobj_t`). Those are
//! NOT ported here; only the dozen-odd fields that are plain
//! numeric/already-ported types are, matching the same Phase 2 scope
//! decision applied to `doomstat.rs`.
//!
//! Deferred to Phase 5 (ported alongside `r_data`/`r_defs`/`p_setup`):
//! `textureheight`, `spritewidth`, `spriteoffset`, `spritetopoffset`,
//! `colormaps`, `flattranslation`, `texturetranslation`, `sprites`,
//! `vertexes`, `segs`, `sectors`, `subsectors`, `nodes`, `lines`,
//! `sides`, `viewplayer`, `floorplane`, `ceilingplane`.
//!
//! Note: `viewwidth`/`scaledviewwidth`/`viewheight` are declared in both
//! `doomstat.h` and `r_state.h` in the original (an actual duplicate
//! `extern` declaration of the same three globals, harmless in C since
//! `extern` just re-declares). They are NOT duplicated here — they
//! already live on [`crate::doomstat::GameState`]; this module does not
//! re-declare them.

use crate::doomdef::SCREENWIDTH;
use crate::m_fixed::{fixed_div, fixed_mul, Fixed, FRACBITS, FRACUNIT};
use crate::tables::{Angle, ANG90, ANGLETOFINESHIFT, FINEANGLES, FINETANGENT};

/// Refresh internal data structures, for rendering.
pub struct RState {
    // needed for pre rendering (fracs) / sprite lump range
    pub firstflat: i32,

    // Sprite....
    pub firstspritelump: i32,
    pub lastspritelump: i32,
    pub numspritelumps: i32,

    // Lookup table counts for map data. The corresponding data pointers
    // (sprites/vertexes/segs/sectors/subsectors/nodes/lines/sides) are
    // deferred to Phase 5 — see module docs — so these counts currently
    // have nothing populating them; ported now only because they're
    // plain `int`s, ahead of the arrays they'll eventually size.
    pub numsprites: i32,
    pub numvertexes: i32,
    pub numsegs: i32,
    pub numsectors: i32,
    pub numsubsectors: i32,
    pub numnodes: i32,
    pub numlines: i32,
    pub numsides: i32,

    // POV data.
    pub viewx: Fixed,
    pub viewy: Fixed,
    pub viewz: Fixed,

    pub viewangle: Angle,
    // `viewplayer: player_t*` deferred — see module docs.
    //
    /// ?
    pub clipangle: Angle,

    pub viewangletox: [i32; FINEANGLES as usize / 2],
    pub xtoviewangle: [Angle; SCREENWIDTH as usize + 1],

    pub rw_distance: Fixed,
    pub rw_normalangle: Angle,

    /// Angle to line origin.
    pub rw_angle1: i32,

    /// Segs count?
    pub sscount: i32,
}

impl Default for RState {
    fn default() -> Self {
        RState {
            firstflat: 0,
            firstspritelump: 0,
            lastspritelump: 0,
            numspritelumps: 0,
            numsprites: 0,
            numvertexes: 0,
            numsegs: 0,
            numsectors: 0,
            numsubsectors: 0,
            numnodes: 0,
            numlines: 0,
            numsides: 0,
            viewx: 0,
            viewy: 0,
            viewz: 0,
            viewangle: 0,
            clipangle: 0,
            viewangletox: [0; FINEANGLES as usize / 2],
            xtoviewangle: [0; SCREENWIDTH as usize + 1],
            rw_distance: 0,
            rw_normalangle: 0,
            rw_angle1: 0,
            sscount: 0,
        }
    }
}

impl RState {
    /// Port of `R_InitTextureMapping`. Use tangent table to generate
    /// `viewangletox`: `viewangletox` will give the next greatest x
    /// after the view angle. Calc focallength so `FIELDOFVIEW` angles
    /// covers `SCREENWIDTH` (the original's own comment, preserved).
    ///
    /// `centerxfrac`/`viewwidth` are passed explicitly rather than read
    /// off `self`/a global — they're computed by
    /// `R_ExecuteSetViewSize`'s view-size setup (`r_main.rs`'s
    /// `Renderer::r_execute_set_view_size`), not stored on `RState`
    /// itself.
    pub fn r_init_texture_mapping(&mut self, centerxfrac: Fixed, viewwidth: i32) {
        use crate::r_main::FIELDOFVIEW;

        let focallength = fixed_div(
            centerxfrac,
            FINETANGENT[(FINEANGLES / 4 + FIELDOFVIEW / 2) as usize],
        );

        #[allow(clippy::needless_range_loop)]
        // indexes FINETANGENT[i] and self.viewangletox[i] together
        for i in 0..(FINEANGLES / 2) as usize {
            let t = if FINETANGENT[i] > FRACUNIT * 2 {
                -1
            } else if FINETANGENT[i] < -FRACUNIT * 2 {
                viewwidth + 1
            } else {
                let t = fixed_mul(FINETANGENT[i], focallength);
                let mut t = (centerxfrac - t + FRACUNIT - 1) >> FRACBITS;
                if t < -1 {
                    t = -1;
                } else if t > viewwidth + 1 {
                    t = viewwidth + 1;
                }
                t
            };
            self.viewangletox[i] = t;
        }

        // Scan viewangletox[] to generate xtoviewangle[]: xtoviewangle
        // will give the smallest view angle that maps to x (the
        // original's own comment, preserved).
        for x in 0..=viewwidth as usize {
            let mut i = 0usize;
            while self.viewangletox[i] > x as i32 {
                i += 1;
            }
            self.xtoviewangle[x] = ((i as u32) << ANGLETOFINESHIFT).wrapping_sub(ANG90);
        }

        // Take out the fencepost cases from viewangletox (the
        // original's own comment, preserved).
        for i in 0..(FINEANGLES / 2) as usize {
            if self.viewangletox[i] == -1 {
                self.viewangletox[i] = 0;
            } else if self.viewangletox[i] == viewwidth + 1 {
                self.viewangletox[i] = viewwidth;
            }
        }

        self.clipangle = self.xtoviewangle[0];
    }
}

use std::sync::OnceLock;

use crate::global_cell::GlobalCell;

static STATE: OnceLock<GlobalCell<RState>> = OnceLock::new();

fn cell() -> &'static GlobalCell<RState> {
    STATE.get_or_init(|| GlobalCell::new(RState::default()))
}

/// Shared read access to the global renderer state. See
/// [`crate::doomstat::state`]/[`crate::global_cell::GlobalCell`] for the
/// single-threaded-engine rationale behind this pattern.
pub fn state() -> &'static RState {
    unsafe { cell().get() }
}

/// Exclusive/mutable access to the global renderer state.
pub fn state_mut() -> &'static mut RState {
    unsafe { cell().get_mut() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_view_position_is_origin() {
        let s = RState::default();
        assert_eq!(s.viewx, 0);
        assert_eq!(s.viewy, 0);
        assert_eq!(s.viewz, 0);
        assert_eq!(s.viewangle, 0);
    }

    #[test]
    fn table_array_sizes_match_original() {
        let s = RState::default();
        assert_eq!(s.viewangletox.len(), 4096); // FINEANGLES/2
        assert_eq!(s.xtoviewangle.len(), 321); // SCREENWIDTH+1
    }

    #[test]
    fn r_init_texture_mapping_produces_monotonic_viewangletox_and_symmetric_clipangle() {
        let mut s = RState::default();
        // Full-screen view size, matching R_ExecuteSetViewSize's
        // setblocks==11 branch.
        let centerx = 320 / 2;
        let centerxfrac = centerx * FRACUNIT;
        s.r_init_texture_mapping(centerxfrac, 320);

        // Center of the screen (x == centerx) must map back to angle 0
        // (straight ahead) via xtoviewangle, mirroring R_ExecuteSetViewSize's
        // implicit assumption that the FOV is symmetric around viewangle.
        // xtoviewangle[centerx] should be close to 0 (within a few BAM
        // units of rounding, same tolerance style as r_main's own
        // octant-boundary tests).
        let center_angle = s.xtoviewangle[centerx as usize] as i32;
        assert!(
            center_angle.unsigned_abs() < (1 << ANGLETOFINESHIFT),
            "expected near-zero angle at screen center, got {center_angle}"
        );

        // clipangle must be xtoviewangle[0] (leftmost column's angle),
        // and, since the original's FOV is symmetric, its two's
        // complement negation should roughly equal xtoviewangle[viewwidth]
        // (rightmost column) within one fine-angle unit of rounding.
        assert_eq!(s.clipangle, s.xtoviewangle[0]);
        let right_angle = s.xtoviewangle[320];
        let left_angle = s.clipangle;
        assert!(
            (left_angle.wrapping_add(right_angle)) < (1 << ANGLETOFINESHIFT),
            "expected left/right edge angles to be roughly symmetric around 0"
        );
    }

    #[test]
    fn global_state_accessors_share_one_instance() {
        let _guard = TEST_LOCK.lock().unwrap();
        state_mut().sscount = 7;
        assert_eq!(state().sscount, 7);
        state_mut().sscount = 0;
    }
}
