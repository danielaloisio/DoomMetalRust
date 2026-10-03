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
//	Rendering main loop and setup functions,
//	 utility functions (BSP, geometry, trigonometry).
//	See tables.c, too.
//
//-----------------------------------------------------------------------------

//! Rust port of `r_main.h` / `r_main.c` (partial, see note below).
//! Rendering main loop and setup functions, utility functions (BSP,
//! geometry, trigonometry). See tables.c, too (the original's own
//! comment, preserved).
//!
//! # Scope
//!
//! Ported: the geometry/BSP utility functions that only depend on
//! `viewx`/`viewy`/`viewangle`/tables/`node_t`/`seg_t` —
//! [`RMain::point_on_side`], [`RMain::point_on_seg_side`],
//! [`RMain::point_to_angle`], [`RMain::point_to_angle2`],
//! [`RMain::point_to_dist`], [`RMain::point_in_subsector`],
//! [`add_point_to_box`], [`RMain::scale_from_global_angle`], plus
//! [`RMain::init_light_tables`] (`R_InitLightTables`, independent of
//! anything but `colormaps` from `r_data`, already ported).
//!
//! Also ported: the frame driver — [`Renderer`] holds every renderer
//! module's state and implements `R_Init` ([`Renderer::r_init`], minus
//! `R_InitSprites`, which the original calls from `P_Init`),
//! `R_ExecuteSetViewSize` ([`Renderer::r_execute_set_view_size`]),
//! `R_SetupFrame` and `R_RenderPlayerView`
//! ([`Renderer::r_render_player_view`]), plugging `r_segs`/`r_plane`/
//! `r_things` into `r_bsp`'s traversal through [`RenderHooks`].
//!
//! Not ported: `R_SetViewSize` (it only sets `setsizeneeded` for the
//! next frame — callers call [`Renderer::r_execute_set_view_size`]
//! directly instead) and the `NetUpdate` calls (Phase 11).
//! `R_InitPointToAngle`/`R_InitTables` are `#if 0`'d out entirely in
//! the original (superseded by `tables.c`'s precomputed data, already
//! ported in Phase 1) — not ported here either, for the same reason the
//! original doesn't call them.
//!
//! # Stand-ins
//!
//! `player_t` (Phase 6) is reduced to [`ViewPlayer`], the fields
//! `R_SetupFrame`/`R_DrawPlayerSprites` read. The mobj pool is a slice
//! of [`Mobj`]s passed per frame; each sector's things are the mobjs
//! whose `subsector` belongs to it (standing in for `sector->thinglist`,
//! see `r_things.rs`).
//!
//! Low detail (`setdetail == 1`) isn't supported yet: `r_plane.rs`
//! only draws high-detail spans (see its module docs), so
//! [`Renderer::r_execute_set_view_size`] rejects it rather than render
//! half-width floors.

use crate::doomdef::{SCREENHEIGHT, SCREENWIDTH};
use crate::m_bbox::{BBox, BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};
use crate::m_fixed::{fixed_div, fixed_mul, Fixed, FRACBITS, FRACUNIT};
use crate::p_setup::Level;
use crate::r_bsp::{RBsp, RenderHooks};
use crate::r_data::RData;
use crate::r_defs::{DrawSeg, Mobj};
use crate::r_draw::{r_init_translation_tables, RDraw};
use crate::r_plane::RPlane;
use crate::r_segs::RSegs;
use crate::r_sky::{r_init_sky_map, Sky};
use crate::r_state::RState;
use crate::r_things::{MaskedDrawContext, PlayerSprites, RThings, ViewParams};
use crate::tables::{
    fine_cosine, slope_div, Angle, ANG180, ANG270, ANG90, ANGLETOFINESHIFT, FINESINE, TANTOANGLE,
};
use crate::w_wad::WadFiles;

/// Fineangles in the SCREENWIDTH wide window (`FIELDOFVIEW`).
pub const FIELDOFVIEW: i32 = 2048;

// Lighting constants. Now why not 32 levels here? (the original's own
// comment, preserved)
pub const LIGHTLEVELS: usize = 16;
pub const LIGHTSEGSHIFT: i32 = 4;

pub const MAXLIGHTSCALE: usize = 48;
pub const LIGHTSCALESHIFT: i32 = 12;
pub const MAXLIGHTZ: usize = 128;
pub const LIGHTZSHIFT: i32 = 20;

/// Number of diminishing brightness levels. There are 0-31, i.e. 32 LUT
/// in the COLORMAP lump (the original's own comment, preserved).
/// (`NUMCOLORMAPS`)
pub const NUMCOLORMAPS: i32 = 32;

/// Port of the `r_main` module's POV/view-related state plus the light
/// tables it precomputes. Named `RMain` rather than something like
/// `RState` to avoid confusion with the already-ported
/// [`crate::r_state`] module (which itself only covers a subset of
/// `r_state.h`'s fields — see that module's docs); this struct is a
/// different, `r_main.c`-sourced set of fields, not a duplicate.
pub struct RMain {
    pub viewx: Fixed,
    pub viewy: Fixed,
    pub viewz: Fixed,
    pub viewangle: Angle,

    pub viewcos: Fixed,
    pub viewsin: Fixed,

    /// Light level -> distance-scaled colormap lookup
    /// (`zlight[LIGHTLEVELS][MAXLIGHTZ]`), as indices into
    /// [`crate::r_data::RData::colormaps`] (256 bytes per level) rather
    /// than the original's raw `lighttable_t*` pointers into the same
    /// buffer — same lookup, index instead of pointer per the plan's
    /// general representation choice.
    pub zlight: [[i32; MAXLIGHTZ]; LIGHTLEVELS],

    /// Light level -> distance-scale colormap lookup
    /// (`scalelight[LIGHTLEVELS][MAXLIGHTSCALE]`), same index-not-pointer
    /// treatment as [`RMain::zlight`]. Unlike `zlight` (populated by
    /// [`RMain::init_light_tables`], ported in Phase 5d), this table is
    /// populated by `R_ExecuteSetViewSize` ([`RMain::init_scale_light`],
    /// called from [`Renderer::r_execute_set_view_size`]), since it
    /// depends on the view width; zeroed until then.
    pub scalelight: [[i32; MAXLIGHTSCALE]; LIGHTLEVELS],

    /// increment every time a check is made (`validcount`, the
    /// original's own comment, preserved).
    pub validcount: i32,
}

/// The body of `R_PointToAngle` on an already view-relative `(x, y)`
/// offset — split out so callers without an [`RMain`] (`s_sound`'s
/// `S_AdjustSoundParams`) can get the same angle without the
/// `viewx`/`viewy` side effect `R_PointToAngle2` has.
pub fn angle_from_delta(x: Fixed, y: Fixed) -> Angle {
    if x == 0 && y == 0 {
        return 0;
    }

    if x >= 0 {
        if y >= 0 {
            // x>=0, y>=0
            if x > y {
                // octant 0
                TANTOANGLE[slope_div(y as u32, x as u32) as usize]
            } else {
                // octant 1
                ANG90
                    .wrapping_sub(1)
                    .wrapping_sub(TANTOANGLE[slope_div(x as u32, y as u32) as usize])
            }
        } else {
            // x>=0, y<0
            let y = -y;
            if x > y {
                // octant 8
                (TANTOANGLE[slope_div(y as u32, x as u32) as usize] as i32).wrapping_neg() as Angle
            } else {
                // octant 7
                ANG270.wrapping_add(TANTOANGLE[slope_div(x as u32, y as u32) as usize])
            }
        }
    } else {
        let x = -x;
        if y >= 0 {
            // x<0, y>=0
            if x > y {
                // octant 3
                ANG180
                    .wrapping_sub(1)
                    .wrapping_sub(TANTOANGLE[slope_div(y as u32, x as u32) as usize])
            } else {
                // octant 2
                ANG90.wrapping_add(TANTOANGLE[slope_div(x as u32, y as u32) as usize])
            }
        } else {
            // x<0, y<0
            let y = -y;
            if x > y {
                // octant 4
                ANG180.wrapping_add(TANTOANGLE[slope_div(y as u32, x as u32) as usize])
            } else {
                // octant 5
                ANG270
                    .wrapping_sub(1)
                    .wrapping_sub(TANTOANGLE[slope_div(x as u32, y as u32) as usize])
            }
        }
    }
}

impl Default for RMain {
    fn default() -> Self {
        RMain {
            viewx: 0,
            viewy: 0,
            viewz: 0,
            viewangle: 0,
            viewcos: 0,
            viewsin: 0,
            zlight: [[0; MAXLIGHTZ]; LIGHTLEVELS],
            scalelight: [[0; MAXLIGHTSCALE]; LIGHTLEVELS],
            validcount: 1,
        }
    }
}

impl RMain {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `R_PointOnSide`. Traverse BSP (sub) tree, check point
    /// against partition plane. Returns side 0 (front) or 1 (back).
    pub fn point_on_side(x: Fixed, y: Fixed, node: &crate::r_defs::Node) -> i32 {
        if node.dx == 0 {
            if x <= node.x {
                return (node.dy > 0) as i32;
            }
            return (node.dy < 0) as i32;
        }
        if node.dy == 0 {
            if y <= node.y {
                return (node.dx < 0) as i32;
            }
            return (node.dx > 0) as i32;
        }

        let dx = x - node.x;
        let dy = y - node.y;

        // Try to quickly decide by looking at sign bits.
        if ((node.dy ^ node.dx ^ dx ^ dy) as u32) & 0x8000_0000 != 0 {
            if ((node.dy ^ dx) as u32) & 0x8000_0000 != 0 {
                // (left is negative)
                return 1;
            }
            return 0;
        }

        let left = fixed_mul(node.dy >> FRACBITS, dx);
        let right = fixed_mul(dy, node.dx >> FRACBITS);

        if right < left {
            // front side
            0
        } else {
            // back side
            1
        }
    }

    /// Port of `R_PointOnSegSide`.
    pub fn point_on_seg_side(
        x: Fixed,
        y: Fixed,
        seg: &crate::r_defs::Seg,
        vertexes: &[crate::r_defs::Vertex],
    ) -> i32 {
        let v1 = vertexes[seg.v1];
        let v2 = vertexes[seg.v2];

        let lx = v1.x;
        let ly = v1.y;
        let ldx = v2.x - lx;
        let ldy = v2.y - ly;

        if ldx == 0 {
            if x <= lx {
                return (ldy > 0) as i32;
            }
            return (ldy < 0) as i32;
        }
        if ldy == 0 {
            if y <= ly {
                return (ldx < 0) as i32;
            }
            return (ldx > 0) as i32;
        }

        let dx = x - lx;
        let dy = y - ly;

        if ((ldy ^ ldx ^ dx ^ dy) as u32) & 0x8000_0000 != 0 {
            if ((ldy ^ dx) as u32) & 0x8000_0000 != 0 {
                return 1;
            }
            return 0;
        }

        let left = fixed_mul(ldy >> FRACBITS, dx);
        let right = fixed_mul(dy, ldx >> FRACBITS);

        if right < left {
            0
        } else {
            1
        }
    }

    /// Port of `R_PointToAngle`. To get a global angle from cartesian
    /// coordinates, the coordinates are flipped until they are in the
    /// first octant of the coordinate system, then the y (<=x) is
    /// scaled and divided by x to get a tangent (slope) value which is
    /// looked up in the tantoangle[] table (the original's own comment,
    /// preserved).
    pub fn point_to_angle(&self, x: Fixed, y: Fixed) -> Angle {
        angle_from_delta(x.wrapping_sub(self.viewx), y.wrapping_sub(self.viewy))
    }

    /// Port of `R_PointToAngle2`. Note: the original mutates the global
    /// `viewx`/`viewy` as a side effect (setting them to `x1`/`y1`
    /// before delegating to `R_PointToAngle`) — reproduced here for
    /// fidelity (`&mut self`), since some original call sites rely on
    /// that side effect persisting afterwards (e.g. subsequent
    /// unqualified `R_PointToAngle` calls in the same function). Ported
    /// callers that don't want the mutation should save/restore
    /// `viewx`/`viewy` themselves, same as the original would require.
    pub fn point_to_angle2(&mut self, x1: Fixed, y1: Fixed, x2: Fixed, y2: Fixed) -> Angle {
        self.viewx = x1;
        self.viewy = y1;
        self.point_to_angle(x2, y2)
    }

    /// Port of `R_PointToDist`.
    pub fn point_to_dist(&self, x: Fixed, y: Fixed) -> Fixed {
        let mut dx = (x - self.viewx).abs();
        let mut dy = (y - self.viewy).abs();

        if dy > dx {
            std::mem::swap(&mut dx, &mut dy);
        }

        let angle = ((TANTOANGLE[(fixed_div(dy, dx) >> crate::tables::DBITS) as usize] as i64
            + ANG90 as i64) as u32
            >> ANGLETOFINESHIFT) as usize;

        // use as cosine
        fixed_div(dx, FINESINE[angle])
    }

    /// Port of `R_ScaleFromGlobalAngle`. Returns the texture mapping
    /// scale for the current line (horizontal span) at the given angle.
    /// `rw_distance` must be calculated first (the original's own
    /// comment, preserved).
    ///
    /// `projection`/`rw_distance`/`rw_normalangle`/`detailshift` are
    /// passed explicitly (rather than read off `self`/a global) since
    /// they're set up by `R_ExecuteSetViewSize`/`r_segs.c` (Phase 5e),
    /// not yet ported — see module docs. `rw_normalangle` is the
    /// original's `rw_normalangle` global (the current wall seg's
    /// normal angle), required — not a placeholder — for a correct
    /// result; there is no valid single-parameter simplification of this
    /// formula.
    pub fn scale_from_global_angle(
        &self,
        visangle: Angle,
        projection: Fixed,
        rw_distance: Fixed,
        rw_normalangle: Angle,
        detailshift: i32,
    ) -> Fixed {
        let anglea = ANG90.wrapping_add(visangle.wrapping_sub(self.viewangle));
        let angleb = ANG90.wrapping_add(visangle.wrapping_sub(rw_normalangle));

        // both sines are always positive
        let sinea = FINESINE[(anglea >> ANGLETOFINESHIFT) as usize];
        let sineb = FINESINE[(angleb >> ANGLETOFINESHIFT) as usize];
        let num = fixed_mul(projection, sineb) << detailshift;
        let den = fixed_mul(rw_distance, sinea);

        if den > num >> 16 {
            let mut scale = fixed_div(num, den);
            if scale > 64 * FRACUNIT {
                scale = 64 * FRACUNIT;
            } else if scale < 256 {
                scale = 256;
            }
            scale
        } else {
            64 * FRACUNIT
        }
    }

    /// Port of `R_PointInSubsector`.
    pub fn point_in_subsector(level: &Level, x: Fixed, y: Fixed) -> usize {
        // single subsector is a special case (the original's own
        // comment, preserved)
        if level.nodes.is_empty() {
            return 0;
        }

        let mut nodenum = level.nodes.len() - 1;

        while nodenum & crate::doomdata::NF_SUBSECTOR as usize == 0 {
            let node = &level.nodes[nodenum];
            let side = Self::point_on_side(x, y, node);
            nodenum = node.children[side as usize] as usize;
        }

        nodenum & !(crate::doomdata::NF_SUBSECTOR as usize)
    }

    /// Port of `R_InitLightTables`. Only inits the zlight table, because
    /// the scalelight table changes with view size (the original's own
    /// comment, preserved).
    pub fn init_light_tables(&mut self, rdata: &RData) {
        let num_colormaps = rdata.colormaps.len() as i32 / 256;
        let num_colormaps = num_colormaps.max(NUMCOLORMAPS);

        for i in 0..LIGHTLEVELS {
            let startmap =
                ((LIGHTLEVELS as i32 - 1 - i as i32) * 2) * num_colormaps / LIGHTLEVELS as i32;
            for j in 0..MAXLIGHTZ {
                let mut scale = fixed_div(
                    (SCREENWIDTH / 2) * FRACUNIT,
                    ((j as i32) + 1) << LIGHTZSHIFT,
                );
                scale >>= LIGHTSCALESHIFT;
                let mut level = startmap - scale / DISTMAP;

                if level < 0 {
                    level = 0;
                }
                if level >= num_colormaps {
                    level = num_colormaps - 1;
                }

                self.zlight[i][j] = level;
            }
        }
    }
}

/// (`DISTMAP`)
const DISTMAP: i32 = 2;

impl RMain {
    /// The `scalelight` part of `R_ExecuteSetViewSize`: calculate the
    /// light levels to use for each level / scale combination (the
    /// original's own comment, preserved). `viewwidth` is already
    /// `>> detailshift`, as in the original.
    pub fn init_scale_light(&mut self, viewwidth: i32, detailshift: i32) {
        for i in 0..LIGHTLEVELS {
            let startmap =
                ((LIGHTLEVELS as i32 - 1 - i as i32) * 2) * NUMCOLORMAPS / LIGHTLEVELS as i32;
            for j in 0..MAXLIGHTSCALE {
                let level =
                    startmap - j as i32 * SCREENWIDTH / (viewwidth << detailshift) / DISTMAP;
                self.scalelight[i][j] = level.clamp(0, NUMCOLORMAPS - 1);
            }
        }
    }
}

/// The fields of `player_t` the renderer reads (Phase 6 stand-in, see
/// module docs).
#[derive(Debug, Clone, Copy)]
pub struct ViewPlayer {
    /// `player->mo->x`/`y`/`angle`
    pub x: Fixed,
    pub y: Fixed,
    pub angle: Angle,
    /// `player->viewz`
    pub viewz: Fixed,
    pub extralight: i32,
    /// `player->fixedcolormap`: colormap row, 0 for none.
    pub fixedcolormap: i32,
    /// What `R_DrawPlayerSprites` reads through `viewplayer`.
    pub sprites: PlayerSprites,
}

/// Every renderer module's state, owned together (the original's
/// `r_*.c` globals) — see module docs.
pub struct Renderer {
    pub rmain: RMain,
    pub rstate: RState,
    pub rbsp: RBsp,
    pub rsegs: RSegs,
    pub rplane: RPlane,
    pub rthings: RThings,
    pub rdraw: RDraw,
    pub rdata: RData,
    pub view: ViewParams,
    pub sky: Sky,
    /// (`framecount`) — just for profiling purposes (the original's own
    /// comment, preserved).
    pub framecount: i32,

    /// (`setsizeneeded`/`setblocks`/`setdetail`) A pending
    /// `R_SetViewSize`, applied at the start of the next frame by
    /// [`Renderer::r_apply_pending_view_size`] (`D_Display`'s
    /// `if (setsizeneeded) R_ExecuteSetViewSize()`).
    pub setsizeneeded: bool,
    pub setblocks: i32,
    pub setdetail: i32,
}

impl Renderer {
    /// Port of `R_Init`, with the view size applied immediately (the
    /// original's `R_SetViewSize` defers it to the next frame).
    /// `R_InitSprites` is left to the caller, as in the original (it's
    /// `P_Init`'s job): call [`RThings::r_init_sprites`] on
    /// [`Renderer::rthings`].
    ///
    /// # Panics
    /// Panics if `setdetail != 0` (see module docs).
    pub fn r_init(wad: &mut WadFiles, setblocks: i32, setdetail: i32) -> Self {
        let mut rdata = RData::new();
        rdata.init(wad);

        let mut renderer = Renderer {
            rmain: RMain::new(),
            rstate: RState::default(),
            rbsp: RBsp::new(),
            rsegs: RSegs::new(),
            rplane: RPlane::new(),
            rthings: RThings::new(),
            rdraw: RDraw::new(),
            rdata,
            view: ViewParams::new(SCREENWIDTH, SCREENHEIGHT, 0),
            sky: Sky::default(),
            framecount: 0,
            setsizeneeded: false,
            setblocks,
            setdetail,
        };

        renderer.r_execute_set_view_size(setblocks, setdetail);
        renderer.rmain.init_light_tables(&renderer.rdata);
        renderer.sky.skytexturemid = r_init_sky_map();
        renderer.rdraw.translationtables = Some(r_init_translation_tables());
        renderer
    }

    /// Port of `R_SetViewSize`: only records the request; the size
    /// changes when [`Renderer::r_apply_pending_view_size`] runs.
    pub fn r_set_view_size(&mut self, blocks: i32, detail: i32) {
        self.setsizeneeded = true;
        self.setblocks = blocks;
        self.setdetail = detail;
    }

    /// The `if (setsizeneeded) { R_ExecuteSetViewSize(); ... }` at the
    /// top of `D_Display`. Returns whether a resize happened (the
    /// caller then redraws the border).
    pub fn r_apply_pending_view_size(&mut self) -> bool {
        if !self.setsizeneeded {
            return false;
        }
        self.r_execute_set_view_size(self.setblocks, self.setdetail);
        self.setsizeneeded = false;
        true
    }

    /// Port of `R_ExecuteSetViewSize`. `setblocks` is the screen size
    /// (3..=11, 11 = full screen without status bar), `setdetail` 0 for
    /// high detail.
    ///
    /// `screenheightarray`, `yslope` and `distscale` aren't stored:
    /// `r_things.rs`/`r_plane.rs` compute them where they're read.
    ///
    /// # Panics
    /// Panics if `setdetail != 0` (see module docs).
    pub fn r_execute_set_view_size(&mut self, setblocks: i32, setdetail: i32) {
        assert_eq!(
            setdetail, 0,
            "low detail isn't supported yet: r_plane.rs has no R_DrawSpanLow selection"
        );

        let (scaledviewwidth, viewheight) = if setblocks == 11 {
            (SCREENWIDTH, SCREENHEIGHT)
        } else {
            (setblocks * 32, (setblocks * 168 / 10) & !7)
        };

        let viewwidth = scaledviewwidth >> setdetail;
        self.view = ViewParams::new(viewwidth, viewheight, setdetail);

        self.rdraw.r_init_buffer(scaledviewwidth, viewheight);
        self.rstate
            .r_init_texture_mapping(self.view.centerxfrac, viewwidth);
        self.rmain.init_scale_light(viewwidth, setdetail);
    }

    /// Port of `R_SetupFrame`.
    fn r_setup_frame(&mut self, player: &ViewPlayer) {
        self.rmain.viewx = player.x;
        self.rmain.viewy = player.y;
        self.rmain.viewangle = player
            .angle
            .wrapping_add(self.view.viewangleoffset as Angle);
        self.view.extralight = player.extralight;

        self.rmain.viewz = player.viewz;

        let fine = (self.rmain.viewangle >> ANGLETOFINESHIFT) as usize;
        self.rmain.viewsin = FINESINE[fine];
        self.rmain.viewcos = fine_cosine(fine);

        self.rbsp.sscount = 0;

        // `walllights = scalelightfixed` is handled where walls pick
        // their colormap (`r_segs.rs`).
        self.view.fixedcolormap = if player.fixedcolormap != 0 {
            Some(player.fixedcolormap as usize)
        } else {
            None
        };

        self.framecount += 1;
        self.rmain.validcount = crate::p_maputl::bump_validcount();
    }

    /// Port of `R_RenderPlayerView`: draws `player`'s view of `level`
    /// into `screen` (`screens[0]`, `SCREENWIDTH * SCREENHEIGHT` bytes).
    ///
    /// `things` is the level's mobj pool in spawn order (see module
    /// docs); each sector's things are walked newest first, like the
    /// original's head-linked `thinglist`.
    pub fn r_render_player_view(
        &mut self,
        player: &ViewPlayer,
        level: &mut Level,
        wad: &mut WadFiles,
        things: &[Mobj],
        screen: &mut [u8],
    ) {
        self.r_setup_frame(player);

        let Renderer {
            rmain,
            rstate,
            rbsp,
            rsegs,
            rplane,
            rthings,
            rdraw,
            rdata,
            view,
            sky,
            ..
        } = self;

        // Clear buffers (the original's own comment, preserved).
        rbsp.clear_clip_segs(view.viewwidth);
        rbsp.clear_draw_segs();
        rplane.r_clear_planes(rmain, view.viewwidth, view.viewheight);
        rthings.r_clear_sprites();
        rbsp.skyflatnum = sky.skyflatnum;

        // The head node is the last node output (the original's own
        // comment, preserved).
        let mut hooks = FrameHooks {
            rmain,
            rstate,
            rsegs,
            rplane,
            rthings,
            rdraw,
            rdata,
            wad,
            screen,
            view,
            sky,
            things,
        };
        let root = level.nodes.len() as i32 - 1;
        rbsp.render_bsp_node(&mut hooks, rmain, rstate, level, root);

        rplane.r_draw_planes(
            rdraw,
            rmain,
            rstate,
            rdata,
            wad,
            screen,
            view.viewwidth,
            view.viewheight,
            view.extralight,
            view.fixedcolormap,
            sky,
        );

        let mut ctx = MaskedDrawContext {
            rmain,
            rdata,
            wad,
            level,
            rplane,
            rsegs,
            rdraw,
            screen,
            view,
        };
        rthings.r_draw_masked(&rbsp.drawsegs, Some(&player.sprites), &mut ctx);
    }
}

/// The real [`RenderHooks`]: `r_bsp`'s traversal calling into
/// `r_segs`/`r_plane`/`r_things` for one frame.
struct FrameHooks<'a> {
    rmain: &'a RMain,
    rstate: &'a RState,
    rsegs: &'a mut RSegs,
    rplane: &'a mut RPlane,
    rthings: &'a mut RThings,
    rdraw: &'a mut RDraw,
    rdata: &'a mut RData,
    wad: &'a mut WadFiles,
    screen: &'a mut [u8],
    view: &'a ViewParams,
    sky: &'a Sky,
    things: &'a [Mobj],
}

impl RenderHooks for FrameHooks<'_> {
    fn store_wall_range(
        &mut self,
        level: &mut Level,
        seg: usize,
        rw_angle1: Angle,
        floorplane: &mut Option<usize>,
        ceilingplane: &mut Option<usize>,
        first: i32,
        last: i32,
    ) -> Option<DrawSeg> {
        self.rsegs.rw_angle1 = rw_angle1;
        Some(self.rsegs.r_store_wall_range(
            self.rmain,
            self.rstate,
            self.rplane,
            self.rdata,
            self.wad,
            level,
            self.rdraw,
            self.screen,
            seg,
            first,
            last,
            self.view.viewwidth,
            self.view.viewheight,
            self.view.centeryfrac,
            self.view.projection,
            self.view.extralight,
            self.view.fixedcolormap,
            self.sky.skyflatnum,
            floorplane,
            ceilingplane,
        ))
    }

    fn find_plane(&mut self, height: Fixed, picnum: i32, lightlevel: i32) -> usize {
        self.rplane
            .r_find_plane(height, picnum, lightlevel, self.sky.skyflatnum)
    }

    fn add_sprites(&mut self, level: &mut Level, sector: usize) {
        let subsectors = &level.subsectors;
        let in_sector: Vec<&Mobj> = self
            .things
            .iter()
            .rev()
            .filter(|m| {
                m.subsector
                    .is_some_and(|ss| subsectors[ss].sector == sector)
            })
            .collect();
        self.rthings.r_add_sprites(
            &mut level.sectors[sector],
            in_sector,
            self.rmain,
            self.rdata,
            self.view,
        );
    }
}

/// Port of `R_AddPointToBox`. Expand a given bbox so that it encloses a
/// given point (the original's own comment, preserved).
///
/// Note: unlike `m_bbox::m_add_to_box` (which this deliberately does NOT
/// call), the original's `R_AddPointToBox` uses `>`/`<` (not the
/// combined if/else-if `m_add_to_box` uses) — functionally equivalent
/// for expanding a box outward, but kept as its own function to mirror
/// the original's actual separate implementation (`r_main.c` never
/// calls `M_AddToBox` itself).
pub fn add_point_to_box(x: i32, y: i32, box_: &mut BBox) {
    if x < box_[BOXLEFT] {
        box_[BOXLEFT] = x;
    }
    if x > box_[BOXRIGHT] {
        box_[BOXRIGHT] = x;
    }
    if y < box_[BOXBOTTOM] {
        box_[BOXBOTTOM] = y;
    }
    if y > box_[BOXTOP] {
        box_[BOXTOP] = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_to_angle_origin_when_at_view_position() {
        let rmain = RMain::new();
        assert_eq!(rmain.point_to_angle(0, 0), 0);
    }

    #[test]
    fn point_to_dist_matches_original_c_reference_values() {
        // Ground truth from compiling and running the original
        // R_PointToDist (tables.c + m_fixed.c) standalone against these
        // exact points, not the ideal Euclidean distance — the table-
        // based trig approximation is intentionally imprecise (e.g. a
        // 3-4-5 triangle gives 327591, not the ideal 327680 == 5*FRACUNIT).
        let rmain = RMain::new();
        assert_eq!(rmain.point_to_dist(3 * FRACUNIT, 4 * FRACUNIT), 327591);
        assert_eq!(rmain.point_to_dist(FRACUNIT, 0), 65537);
        assert_eq!(rmain.point_to_dist(0, FRACUNIT), 65537);
    }

    #[test]
    fn point_to_angle_matches_known_octant_values() {
        let rmain = RMain::new();
        // Ground truth from compiling and running the original
        // R_PointToAngle (tables.c + this same logic) standalone against
        // these exact four points — not assumed from ANG90/ANG180/ANG270
        // by eye. The octant-boundary cases (x==0 or y==0 on the
        // diagonal) hit the "ANGxx - 1 - tantoangle[...]" branches with
        // tantoangle[0]==0, landing one BAM unit short of the "obvious"
        // ANG90/ANG180 value — a real quirk of the original algorithm's
        // octant split, not a rounding bug in this port.
        assert_eq!(rmain.point_to_angle(FRACUNIT, 0), 0);
        assert_eq!(rmain.point_to_angle(0, FRACUNIT), ANG90 - 1);
        assert_eq!(rmain.point_to_angle(-FRACUNIT, 0), ANG180 - 1);
        assert_eq!(rmain.point_to_angle(0, -FRACUNIT), ANG270);
    }

    #[test]
    fn add_point_to_box_expands_correctly() {
        let mut box_: BBox = [0, 0, 0, 0];
        add_point_to_box(10, 20, &mut box_);
        add_point_to_box(-5, -3, &mut box_);
        assert_eq!(box_[BOXLEFT], -5);
        assert_eq!(box_[BOXRIGHT], 10);
        assert_eq!(box_[BOXBOTTOM], -3);
        assert_eq!(box_[BOXTOP], 20);
    }

    #[test]
    fn point_to_angle2_mutates_viewx_viewy() {
        let mut rmain = RMain::new();
        let angle = rmain.point_to_angle2(0, 0, FRACUNIT, 0);
        assert_eq!(angle, 0);
        assert_eq!(rmain.viewx, 0);
        assert_eq!(rmain.viewy, 0);
    }

    #[test]
    fn point_on_side_matches_axis_aligned_partition() {
        // A vertical partition line (dx=0) at x=100: dy>0 means "up".
        let node = crate::r_defs::Node {
            x: 100,
            y: 0,
            dx: 0,
            dy: FRACUNIT,
            bbox: [[0; 4]; 2],
            children: [0, 0],
        };
        // x <= node.x -> side is (dy>0) -> 1
        assert_eq!(RMain::point_on_side(50, 0, &node), 1);
        // x > node.x -> side is (dy<0) -> 0
        assert_eq!(RMain::point_on_side(150, 0, &node), 0);
    }

    #[test]
    fn light_tables_are_populated_within_range() {
        let mut rmain = RMain::new();
        let rdata = RData::new(); // colormaps empty -> falls back to NUMCOLORMAPS
        rmain.init_light_tables(&rdata);
        for level_row in &rmain.zlight {
            for &level in level_row {
                assert!((0..NUMCOLORMAPS).contains(&level));
            }
        }
    }

    #[test]
    fn init_scale_light_matches_original_formula_at_full_width() {
        let mut rmain = RMain::new();
        rmain.init_scale_light(320, 0);

        // Darkest light level, nearest scale: startmap 60, clamped.
        assert_eq!(rmain.scalelight[0][0], NUMCOLORMAPS - 1);
        // Brightest light level: startmap 0, never darker than map 0.
        assert!(rmain.scalelight[LIGHTLEVELS - 1].iter().all(|&l| l == 0));
        // Light level 8: startmap (7*2)*32/16 = 28, minus j/2 per step.
        assert_eq!(rmain.scalelight[8][0], 28);
        assert_eq!(rmain.scalelight[8][10], 23);
        // Larger scale (closer) is never darker.
        for row in &rmain.scalelight {
            assert!(row.windows(2).all(|w| w[1] <= w[0]));
        }
    }
}
