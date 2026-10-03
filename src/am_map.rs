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
// DESCRIPTION:  the automap code
//
//-----------------------------------------------------------------------------

//! Rust port of `am_map.h` / `am_map.c`. The automap: the level's lines
//! drawn in map colours over a cleared `screens[0]`, the player arrow
//! (and, with the `iddt` cheat, every thing), a grid, numbered marks,
//! panning/zooming and "follow the player".
//!
//! # State
//!
//! The file-scope statics are one [`AmState`] in a `thread_local!`
//! behind C-named free functions ([`am_start`], [`am_stop`],
//! [`am_responder`], [`am_ticker`], [`am_drawer`]) — same convention as
//! the other UI modules. What the automap reads and writes outside
//! itself comes in an [`AmCtx`]. `automapactive` is `doomstat`'s.
//!
//! # Divergences
//!
//! * `viewactive` isn't a variable here: `d_display` derives it from
//!   `automapactive` (the two are always opposites in a level).
//! * `AM_Stop`'s "exited" notification to the status bar is built in
//!   the original with its initializer fields in the wrong order
//!   (`{ 0, ev_keyup, AM_MSGEXITED }` for `{ type, data1, data2, ... }`),
//!   so `ST_Responder` receives a key-*down* of key 1 and never learns
//!   the automap closed. Reproduced as is (`st_gamestate` stays
//!   `AutomapState`; nothing reads it).
//! * `AM_updateLightLev` is never called in the original, so it isn't
//!   ported (`lightlev` stays 0, as there). `AM_drawFline`'s debug
//!   `fuck` message for an out-of-range line is kept (the line is
//!   skipped).
//! * The `iddt` cheat is checked inside [`am_responder`] with its own
//!   [`CheatSeq`], like the original's `cheat_amap`.

use std::cell::RefCell;

use crate::d_englsh as msg;
use crate::d_event::{EvType, Event};
use crate::d_player::Player;
use crate::doomdata::{ML_DONTDRAW, ML_MAPPED, ML_SECRET};
use crate::doomdef::{
    PowerType, KEY_DOWNARROW, KEY_LEFTARROW, KEY_RIGHTARROW, KEY_TAB, KEY_UPARROW, MAXPLAYERS,
    SCREENHEIGHT, SCREENWIDTH,
};
use crate::doomstat;
use crate::m_cheat::CheatSeq;
use crate::m_fixed::{fixed_div, fixed_mul, Fixed, FRACBITS, FRACUNIT};
use crate::p_setup::{Level, MAPBLOCKUNITS};
use crate::p_tick::Thinkers;
use crate::st_stuff::st_responder;
use crate::tables::{Angle, ANGLETOFINESHIFT, FINESINE};
use crate::v_video::{PatchData, VVideo};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// (`AM_MSGHEADER`) `(('a'<<24)+('m'<<16))`.
pub const AM_MSGHEADER: i32 = ((b'a' as i32) << 24) + ((b'm' as i32) << 16);
/// (`AM_MSGENTERED`) `(AM_MSGHEADER | ('e'<<8))`.
pub const AM_MSGENTERED: i32 = AM_MSGHEADER | ((b'e' as i32) << 8);
/// (`AM_MSGEXITED`) `(AM_MSGHEADER | ('x'<<8))`.
pub const AM_MSGEXITED: i32 = AM_MSGHEADER | ((b'x' as i32) << 8);

/// (`PLAYERRADIUS`, `p_local.h`)
const PLAYERRADIUS: Fixed = 16 * FRACUNIT;

// Palette indices of the automap colours.
const REDS: i32 = 256 - 5 * 16;
const REDRANGE: i32 = 16;
const GREENS: i32 = 7 * 16;
const GRAYS: i32 = 6 * 16;
const GRAYSRANGE: i32 = 16;
const BROWNS: i32 = 4 * 16;
const YELLOWS: i32 = 256 - 32 + 7;
const BLACK: i32 = 0;
const WHITE: i32 = 256 - 47;

/// Automap colors.
const BACKGROUND: i32 = BLACK;
const WALLCOLORS: i32 = REDS;
const WALLRANGE: i32 = REDRANGE;
const TSWALLCOLORS: i32 = GRAYS;
const FDWALLCOLORS: i32 = BROWNS;
const CDWALLCOLORS: i32 = YELLOWS;
const THINGCOLORS: i32 = GREENS;
const SECRETWALLCOLORS: i32 = WALLCOLORS;
const GRIDCOLORS: i32 = GRAYS + GRAYSRANGE / 2;
const XHAIRCOLORS: i32 = GRAYS;

/// The keys (`AM_*KEY`).
pub const AM_PANDOWNKEY: i32 = KEY_DOWNARROW;
pub const AM_PANUPKEY: i32 = KEY_UPARROW;
pub const AM_PANRIGHTKEY: i32 = KEY_RIGHTARROW;
pub const AM_PANLEFTKEY: i32 = KEY_LEFTARROW;
pub const AM_ZOOMINKEY: i32 = b'=' as i32;
pub const AM_ZOOMOUTKEY: i32 = b'-' as i32;
pub const AM_STARTKEY: i32 = KEY_TAB;
pub const AM_ENDKEY: i32 = KEY_TAB;
pub const AM_GOBIGKEY: i32 = b'0' as i32;
pub const AM_FOLLOWKEY: i32 = b'f' as i32;
pub const AM_GRIDKEY: i32 = b'g' as i32;
pub const AM_MARKKEY: i32 = b'm' as i32;
pub const AM_CLEARMARKKEY: i32 = b'c' as i32;

/// (`AM_NUMMARKPOINTS`)
pub const AM_NUMMARKPOINTS: usize = 10;
/// scale on entry (`INITSCALEMTOF`, `.2*FRACUNIT`)
const INITSCALEMTOF: Fixed = (0.2 * FRACUNIT as f64) as Fixed;
/// how much the automap moves window per tic in frame-buffer coordinates
/// (`F_PANINC`): moves 140 pixels in 1 second
const F_PANINC: i32 = 4;
/// how much zoom-in per tic: goes to 2x in 1 second (`M_ZOOMIN`)
const M_ZOOMIN: Fixed = (1.02 * FRACUNIT as f64) as Fixed;
/// how much zoom-out per tic: pulls out to 0.5x in 1 second
/// (`M_ZOOMOUT`)
const M_ZOOMOUT: Fixed = (FRACUNIT as f64 / 1.02) as Fixed;

/// (`fpoint_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct FPoint {
    x: i32,
    y: i32,
}

/// (`fline_t`)
#[derive(Debug, Clone, Copy, Default)]
struct FLine {
    a: FPoint,
    b: FPoint,
}

/// (`mpoint_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct MPoint {
    x: Fixed,
    y: Fixed,
}

/// (`mline_t`)
#[derive(Debug, Clone, Copy)]
struct MLine {
    a: MPoint,
    b: MPoint,
}

const fn ml(ax: i32, ay: i32, bx: i32, by: i32) -> MLine {
    MLine {
        a: MPoint { x: ax, y: ay },
        b: MPoint { x: bx, y: by },
    }
}

/// `R` in the player-arrow tables: `(8*PLAYERRADIUS)/7`.
const R: i32 = (8 * PLAYERRADIUS) / 7;

/// Line-art for the player (`player_arrow`).
const PLAYER_ARROW: [MLine; 7] = [
    ml(-R + R / 8, 0, R, 0),    // -----
    ml(R, 0, R - R / 2, R / 4), // ----->
    ml(R, 0, R - R / 2, -R / 4),
    ml(-R + R / 8, 0, -R - R / 8, R / 4), // >---->
    ml(-R + R / 8, 0, -R - R / 8, -R / 4),
    ml(-R + 3 * R / 8, 0, -R + R / 8, R / 4), // >>--->
    ml(-R + 3 * R / 8, 0, -R + R / 8, -R / 4),
];

/// The player with the `iddt` cheat on (`cheat_player_arrow`).
const CHEAT_PLAYER_ARROW: [MLine; 16] = [
    ml(-R + R / 8, 0, R, 0),    // -----
    ml(R, 0, R - R / 2, R / 6), // ----->
    ml(R, 0, R - R / 2, -R / 6),
    ml(-R + R / 8, 0, -R - R / 8, R / 6), // >----->
    ml(-R + R / 8, 0, -R - R / 8, -R / 6),
    ml(-R + 3 * R / 8, 0, -R + R / 8, R / 6), // >>----->
    ml(-R + 3 * R / 8, 0, -R + R / 8, -R / 6),
    ml(-R / 2, 0, -R / 2, -R / 6), // >>-d--->
    ml(-R / 2, -R / 6, -R / 2 + R / 6, -R / 6),
    ml(-R / 2 + R / 6, -R / 6, -R / 2 + R / 6, R / 4),
    ml(-R / 6, 0, -R / 6, -R / 6), // >>-dd-->
    ml(-R / 6, -R / 6, 0, -R / 6),
    ml(0, -R / 6, 0, R / 4),
    ml(R / 6, R / 4, R / 6, -R / 7), // >>-ddt->
    ml(R / 6, -R / 7, R / 6 + R / 32, -R / 7 - R / 32),
    ml(R / 6 + R / 32, -R / 7 - R / 32, R / 6 + R / 10, -R / 7),
];

/// (`thintriangle_guy`) the things drawn by `iddt`. `R` here is
/// `FRACUNIT`, and the constants are C `double`s truncated to `int`.
const THINTRIANGLE_GUY: [MLine; 3] = [
    ml(-32768, -45875, FRACUNIT, 0), // (-.5*R, -.7*R) -> (R, 0)
    ml(FRACUNIT, 0, -32768, 45875),
    ml(-32768, 45875, -32768, -45875),
];

/// The file-scope statics of `am_map.c`.
struct AmState {
    cheating: i32,
    grid: bool,
    finit_width: i32,
    finit_height: i32,
    // Location of window on screen
    f_x: i32,
    f_y: i32,
    // size of window on screen
    f_w: i32,
    f_h: i32,
    /// used for funky strobing effect
    lightlev: i32,
    #[allow(dead_code)]
    amclock: i32,
    /// how far the window pans each tic (map coords)
    m_paninc: MPoint,
    /// how far the window zooms in each tic (map coords)
    mtof_zoommul: Fixed,
    /// how far the window zooms in each tic (fb coords)
    ftom_zoommul: Fixed,
    /// LL x,y where the window is on the map (map coords)
    m_x: Fixed,
    m_y: Fixed,
    /// UR x,y where the window is on the map (map coords)
    m_x2: Fixed,
    m_y2: Fixed,
    // width/height of window on map (map coords)
    m_w: Fixed,
    m_h: Fixed,
    // based on level size
    min_x: Fixed,
    min_y: Fixed,
    max_x: Fixed,
    max_y: Fixed,
    max_w: Fixed,
    max_h: Fixed,
    min_w: Fixed,
    min_h: Fixed,
    /// used to tell when to stop zooming out
    min_scale_mtof: Fixed,
    /// used to tell when to stop zooming in
    max_scale_mtof: Fixed,
    // old stuff for recovery later
    old_m_w: Fixed,
    old_m_h: Fixed,
    old_m_x: Fixed,
    old_m_y: Fixed,
    /// old location used by the Follower routine
    f_oldloc: MPoint,
    // used by MTOF to scale from map-to-frame-buffer coords
    scale_mtof: Fixed,
    // used by FTOM to scale from frame-buffer-to-map coords
    // (=1/scale_mtof)
    scale_ftom: Fixed,
    /// numbers used for marking by the automap
    marknums: Vec<PatchData>,
    /// where the points are
    markpoints: [MPoint; AM_NUMMARKPOINTS],
    /// next point to be assigned
    markpointnum: usize,
    /// specifies whether to follow the player around
    followplayer: bool,
    cheat_amap: CheatSeq,
    stopped: bool,
    /// `AM_Responder`'s `static int bigstate`.
    bigstate: bool,
    /// `AM_Start`'s `static int lastlevel/lastepisode`.
    lastlevel: i32,
    lastepisode: i32,
}

thread_local! {
    static AM: RefCell<AmState> = RefCell::new(AmState::new());
}

fn with<R>(f: impl FnOnce(&mut AmState) -> R) -> R {
    AM.with(|s| f(&mut s.borrow_mut()))
}

/// Resets the automap to its power-on state (tests, shutdown).
pub fn am_shutdown() {
    AM.with(|s| *s.borrow_mut() = AmState::new());
    doomstat::state_mut().automapactive = false;
}

impl AmState {
    fn new() -> Self {
        AmState {
            cheating: 0,
            grid: false,
            finit_width: SCREENWIDTH,
            finit_height: SCREENHEIGHT - 32,
            f_x: 0,
            f_y: 0,
            f_w: 0,
            f_h: 0,
            lightlev: 0,
            amclock: 0,
            m_paninc: MPoint::default(),
            mtof_zoommul: FRACUNIT,
            ftom_zoommul: FRACUNIT,
            m_x: 0,
            m_y: 0,
            m_x2: 0,
            m_y2: 0,
            m_w: 0,
            m_h: 0,
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            max_w: 0,
            max_h: 0,
            min_w: 0,
            min_h: 0,
            min_scale_mtof: 0,
            max_scale_mtof: 0,
            old_m_w: 0,
            old_m_h: 0,
            old_m_x: 0,
            old_m_y: 0,
            f_oldloc: MPoint::default(),
            scale_mtof: INITSCALEMTOF,
            scale_ftom: 0,
            marknums: Vec::new(),
            markpoints: [MPoint { x: -1, y: 0 }; AM_NUMMARKPOINTS],
            markpointnum: 0,
            followplayer: true,
            cheat_amap: CheatSeq::new(&[0xb2, 0x26, 0x26, 0x2e, 0xff]),
            stopped: true,
            bigstate: false,
            lastlevel: -1,
            lastepisode: -1,
        }
    }

    /// `FTOM(x)`: `FixedMul(((x)<<16), scale_ftom)`.
    fn ftom(&self, x: i32) -> Fixed {
        fixed_mul(x.wrapping_shl(16), self.scale_ftom)
    }

    /// `MTOF(x)`: `FixedMul((x), scale_mtof) >> 16`.
    fn mtof(&self, x: Fixed) -> i32 {
        fixed_mul(x, self.scale_mtof) >> 16
    }

    /// `CXMTOF(x)`.
    fn cxmtof(&self, x: Fixed) -> i32 {
        self.f_x + self.mtof(x.wrapping_sub(self.m_x))
    }

    /// `CYMTOF(y)`.
    fn cymtof(&self, y: Fixed) -> i32 {
        self.f_y + (self.f_h - self.mtof(y.wrapping_sub(self.m_y)))
    }

    /// Port of `AM_activateNewScale`. Changes the scale of the map
    /// window while keeping its centre.
    fn activate_new_scale(&mut self) {
        self.m_x += self.m_w / 2;
        self.m_y += self.m_h / 2;
        self.m_w = self.ftom(self.f_w);
        self.m_h = self.ftom(self.f_h);
        self.m_x -= self.m_w / 2;
        self.m_y -= self.m_h / 2;
        self.m_x2 = self.m_x + self.m_w;
        self.m_y2 = self.m_y + self.m_h;
    }

    /// Port of `AM_saveScaleAndLoc`.
    fn save_scale_and_loc(&mut self) {
        self.old_m_x = self.m_x;
        self.old_m_y = self.m_y;
        self.old_m_w = self.m_w;
        self.old_m_h = self.m_h;
    }

    /// Port of `AM_restoreScaleAndLoc`.
    fn restore_scale_and_loc(&mut self, player_pos: (Fixed, Fixed)) {
        self.m_w = self.old_m_w;
        self.m_h = self.old_m_h;
        if !self.followplayer {
            self.m_x = self.old_m_x;
            self.m_y = self.old_m_y;
        } else {
            self.m_x = player_pos.0 - self.m_w / 2;
            self.m_y = player_pos.1 - self.m_h / 2;
        }
        self.m_x2 = self.m_x + self.m_w;
        self.m_y2 = self.m_y + self.m_h;

        // Change the scaling multipliers
        self.scale_mtof = fixed_div(self.f_w << FRACBITS, self.m_w);
        self.scale_ftom = fixed_div(FRACUNIT, self.scale_mtof);
    }

    /// Port of `AM_addMark`. Adds a marker at the current location.
    fn add_mark(&mut self) {
        self.markpoints[self.markpointnum].x = self.m_x + self.m_w / 2;
        self.markpoints[self.markpointnum].y = self.m_y + self.m_h / 2;
        self.markpointnum = (self.markpointnum + 1) % AM_NUMMARKPOINTS;
    }

    /// Port of `AM_clearMarks`.
    fn clear_marks(&mut self) {
        for p in &mut self.markpoints {
            p.x = -1; // means empty
        }
        self.markpointnum = 0;
    }

    /// Port of `AM_findMinMaxBoundaries`. Determines bounding box of
    /// all vertices, sets global variables controlling zoom range.
    fn find_min_max_boundaries(&mut self, level: &Level) {
        self.min_x = i32::MAX;
        self.min_y = i32::MAX;
        self.max_x = -i32::MAX;
        self.max_y = -i32::MAX;

        for v in &level.vertexes {
            if v.x < self.min_x {
                self.min_x = v.x;
            } else if v.x > self.max_x {
                self.max_x = v.x;
            }

            if v.y < self.min_y {
                self.min_y = v.y;
            } else if v.y > self.max_y {
                self.max_y = v.y;
            }
        }

        self.max_w = self.max_x - self.min_x;
        self.max_h = self.max_y - self.min_y;

        self.min_w = 2 * PLAYERRADIUS; // const? never changed?
        self.min_h = 2 * PLAYERRADIUS;

        let a = fixed_div(self.f_w << FRACBITS, self.max_w);
        let b = fixed_div(self.f_h << FRACBITS, self.max_h);

        self.min_scale_mtof = if a < b { a } else { b };
        self.max_scale_mtof = fixed_div(self.f_h << FRACBITS, 2 * PLAYERRADIUS);
    }

    /// Port of `AM_changeWindowLoc`.
    fn change_window_loc(&mut self) {
        if self.m_paninc.x != 0 || self.m_paninc.y != 0 {
            self.followplayer = false;
            self.f_oldloc.x = i32::MAX;
        }

        self.m_x += self.m_paninc.x;
        self.m_y += self.m_paninc.y;

        if self.m_x + self.m_w / 2 > self.max_x {
            self.m_x = self.max_x - self.m_w / 2;
        } else if self.m_x + self.m_w / 2 < self.min_x {
            self.m_x = self.min_x - self.m_w / 2;
        }

        if self.m_y + self.m_h / 2 > self.max_y {
            self.m_y = self.max_y - self.m_h / 2;
        } else if self.m_y + self.m_h / 2 < self.min_y {
            self.m_y = self.min_y - self.m_h / 2;
        }

        self.m_x2 = self.m_x + self.m_w;
        self.m_y2 = self.m_y + self.m_h;
    }

    /// Port of `AM_minOutWindowScale`.
    fn min_out_window_scale(&mut self) {
        self.scale_mtof = self.min_scale_mtof;
        self.scale_ftom = fixed_div(FRACUNIT, self.scale_mtof);
        self.activate_new_scale();
    }

    /// Port of `AM_maxOutWindowScale`.
    fn max_out_window_scale(&mut self) {
        self.scale_mtof = self.max_scale_mtof;
        self.scale_ftom = fixed_div(FRACUNIT, self.scale_mtof);
        self.activate_new_scale();
    }

    /// Port of `AM_changeWindowScale`. Zooming.
    fn change_window_scale(&mut self) {
        // Change the scaling multipliers
        self.scale_mtof = fixed_mul(self.scale_mtof, self.mtof_zoommul);
        self.scale_ftom = fixed_div(FRACUNIT, self.scale_mtof);

        if self.scale_mtof < self.min_scale_mtof {
            self.min_out_window_scale();
        } else if self.scale_mtof > self.max_scale_mtof {
            self.max_out_window_scale();
        } else {
            self.activate_new_scale();
        }
    }

    /// Port of `AM_doFollowPlayer`.
    fn do_follow_player(&mut self, player_pos: (Fixed, Fixed)) {
        if self.f_oldloc.x != player_pos.0 || self.f_oldloc.y != player_pos.1 {
            self.m_x = self.ftom(self.mtof(player_pos.0)) - self.m_w / 2;
            self.m_y = self.ftom(self.mtof(player_pos.1)) - self.m_h / 2;
            self.m_x2 = self.m_x + self.m_w;
            self.m_y2 = self.m_y + self.m_h;
            self.f_oldloc.x = player_pos.0;
            self.f_oldloc.y = player_pos.1;
        }
    }

    /// Port of `AM_LevelInit`. Should be called at the start of a level
    /// (the original's own comment).
    fn level_init(&mut self, level: &Level) {
        self.f_x = 0;
        self.f_y = 0;
        self.f_w = self.finit_width;
        self.f_h = self.finit_height;

        self.clear_marks();

        self.find_min_max_boundaries(level);
        self.scale_mtof = fixed_div(self.min_scale_mtof, (0.7 * FRACUNIT as f64) as i32);
        if self.scale_mtof > self.max_scale_mtof {
            self.scale_mtof = self.min_scale_mtof;
        }
        self.scale_ftom = fixed_div(FRACUNIT, self.scale_mtof);
    }

    /// Port of `AM_clipMline`. Clip lines, draw visible parts of lines
    /// (Cohen-Sutherland): returns the clipped frame-buffer line, or
    /// `None` if trivially outside.
    fn clip_mline(&self, ml: &MLine) -> Option<FLine> {
        const LEFT: i32 = 1;
        const RIGHT: i32 = 2;
        const BOTTOM: i32 = 4;
        const TOP: i32 = 8;

        let mut outcode1 = 0;
        let mut outcode2 = 0;

        let doutcode = |mx: i32, my: i32, f_w: i32, f_h: i32| -> i32 {
            let mut oc = 0;
            if my < 0 {
                oc |= TOP;
            } else if my >= f_h {
                oc |= BOTTOM;
            }
            if mx < 0 {
                oc |= LEFT;
            } else if mx >= f_w {
                oc |= RIGHT;
            }
            oc
        };

        // do trivial rejects and pickoffs
        if ml.a.y > self.m_y2 {
            outcode1 = TOP;
        } else if ml.a.y < self.m_y {
            outcode1 = BOTTOM;
        }

        if ml.b.y > self.m_y2 {
            outcode2 = TOP;
        } else if ml.b.y < self.m_y {
            outcode2 = BOTTOM;
        }

        if outcode1 & outcode2 != 0 {
            return None; // trivially outside
        }

        if ml.a.x < self.m_x {
            outcode1 |= LEFT;
        } else if ml.a.x > self.m_x2 {
            outcode1 |= RIGHT;
        }

        if ml.b.x < self.m_x {
            outcode2 |= LEFT;
        } else if ml.b.x > self.m_x2 {
            outcode2 |= RIGHT;
        }

        if outcode1 & outcode2 != 0 {
            return None; // trivially outside
        }

        // transform to frame-buffer coordinates.
        let mut fl = FLine {
            a: FPoint {
                x: self.cxmtof(ml.a.x),
                y: self.cymtof(ml.a.y),
            },
            b: FPoint {
                x: self.cxmtof(ml.b.x),
                y: self.cymtof(ml.b.y),
            },
        };

        outcode1 = doutcode(fl.a.x, fl.a.y, self.f_w, self.f_h);
        outcode2 = doutcode(fl.b.x, fl.b.y, self.f_w, self.f_h);

        if outcode1 & outcode2 != 0 {
            return None;
        }

        while outcode1 | outcode2 != 0 {
            // may be partially inside box, find an outside point
            let outside = if outcode1 != 0 { outcode1 } else { outcode2 };

            // clip to each side
            let tmp = if outside & TOP != 0 {
                let dy = fl.a.y - fl.b.y;
                let dx = fl.b.x - fl.a.x;
                FPoint {
                    x: fl.a.x + (dx * fl.a.y) / dy,
                    y: 0,
                }
            } else if outside & BOTTOM != 0 {
                let dy = fl.a.y - fl.b.y;
                let dx = fl.b.x - fl.a.x;
                FPoint {
                    x: fl.a.x + (dx * (fl.a.y - self.f_h)) / dy,
                    y: self.f_h - 1,
                }
            } else if outside & RIGHT != 0 {
                let dy = fl.b.y - fl.a.y;
                let dx = fl.b.x - fl.a.x;
                FPoint {
                    x: self.f_w - 1,
                    y: fl.a.y + (dy * (self.f_w - 1 - fl.a.x)) / dx,
                }
            } else {
                // LEFT
                let dy = fl.b.y - fl.a.y;
                let dx = fl.b.x - fl.a.x;
                FPoint {
                    x: 0,
                    y: fl.a.y + (dy * (-fl.a.x)) / dx,
                }
            };

            if outside == outcode1 {
                fl.a = tmp;
                outcode1 = doutcode(fl.a.x, fl.a.y, self.f_w, self.f_h);
            } else {
                fl.b = tmp;
                outcode2 = doutcode(fl.b.x, fl.b.y, self.f_w, self.f_h);
            }

            if outcode1 & outcode2 != 0 {
                return None; // trivially outside
            }
        }

        Some(fl)
    }

    /// Port of `AM_drawFline`. Classic Bresenham w/ whatever
    /// optimizations needed for speed (the original's own comment).
    fn draw_fline(&self, fb: &mut [u8], fl: &FLine, color: i32) {
        if fl.a.x < 0
            || fl.a.x >= self.f_w
            || fl.a.y < 0
            || fl.a.y >= self.f_h
            || fl.b.x < 0
            || fl.b.x >= self.f_w
            || fl.b.y < 0
            || fl.b.y >= self.f_h
        {
            eprintln!("fuck \\r");
            return;
        }

        let mut put = |x: i32, y: i32| fb[(y * self.f_w + x) as usize] = color as u8;

        let dx = fl.b.x - fl.a.x;
        let ax = 2 * dx.abs();
        let sx = if dx < 0 { -1 } else { 1 };

        let dy = fl.b.y - fl.a.y;
        let ay = 2 * dy.abs();
        let sy = if dy < 0 { -1 } else { 1 };

        let mut x = fl.a.x;
        let mut y = fl.a.y;

        if ax > ay {
            let mut d = ay - ax / 2;
            loop {
                put(x, y);
                if x == fl.b.x {
                    return;
                }
                if d >= 0 {
                    y += sy;
                    d -= ax;
                }
                x += sx;
                d += ay;
            }
        } else {
            let mut d = ax - ay / 2;
            loop {
                put(x, y);
                if y == fl.b.y {
                    return;
                }
                if d >= 0 {
                    x += sx;
                    d -= ay;
                }
                y += sy;
                d += ax;
            }
        }
    }

    /// Port of `AM_drawMline`. Clip lines, draw visible part sof lines.
    fn draw_mline(&self, fb: &mut [u8], ml: &MLine, color: i32) {
        if let Some(fl) = self.clip_mline(ml) {
            self.draw_fline(fb, &fl, color); // draws it on frame buffer using fb coords
        }
    }

    /// Port of `AM_drawGrid`. Draws flat (floor/ceiling tile) aligned
    /// grid lines.
    fn draw_grid(&self, fb: &mut [u8], bmaporgx: Fixed, bmaporgy: Fixed, color: i32) {
        let unit = MAPBLOCKUNITS << FRACBITS;

        // Figure out start of vertical gridlines
        let mut start = self.m_x;
        if (start - bmaporgx) % unit != 0 {
            start += unit - ((start - bmaporgx) % unit);
        }
        let end = self.m_x + self.m_w;

        // draw vertical gridlines
        let mut x = start;
        while x < end {
            self.draw_mline(
                fb,
                &MLine {
                    a: MPoint { x, y: self.m_y },
                    b: MPoint {
                        x,
                        y: self.m_y + self.m_h,
                    },
                },
                color,
            );
            x += unit;
        }

        // Figure out start of horizontal gridlines
        let mut start = self.m_y;
        if (start - bmaporgy) % unit != 0 {
            start += unit - ((start - bmaporgy) % unit);
        }
        let end = self.m_y + self.m_h;

        // draw horizontal gridlines
        let mut y = start;
        while y < end {
            self.draw_mline(
                fb,
                &MLine {
                    a: MPoint { x: self.m_x, y },
                    b: MPoint {
                        x: self.m_x + self.m_w,
                        y,
                    },
                },
                color,
            );
            y += unit;
        }
    }

    /// Port of `AM_drawWalls`. Determines visible lines, draws them.
    /// This is LineDef based, not LineSeg based (the original's own
    /// comment).
    fn draw_walls(&self, fb: &mut [u8], level: &Level, plr: &Player) {
        for line in &level.lines {
            let (v1, v2) = (&level.vertexes[line.v1], &level.vertexes[line.v2]);
            let l = MLine {
                a: MPoint { x: v1.x, y: v1.y },
                b: MPoint { x: v2.x, y: v2.y },
            };
            let lightlev = self.lightlev;
            let cheating = self.cheating != 0;

            if cheating || line.flags & ML_MAPPED != 0 {
                if line.flags & ML_DONTDRAW != 0 && !cheating {
                    continue;
                }
                match (line.frontsector, line.backsector) {
                    (_, None) => self.draw_mline(fb, &l, WALLCOLORS + lightlev),
                    (Some(front), Some(back)) => {
                        let (front, back) = (&level.sectors[front], &level.sectors[back]);
                        if line.special == 39 {
                            // teleporters
                            self.draw_mline(fb, &l, WALLCOLORS + WALLRANGE / 2);
                        } else if line.flags & ML_SECRET != 0 {
                            // secret door
                            if cheating {
                                self.draw_mline(fb, &l, SECRETWALLCOLORS + lightlev);
                            } else {
                                self.draw_mline(fb, &l, WALLCOLORS + lightlev);
                            }
                        } else if back.floorheight != front.floorheight {
                            self.draw_mline(fb, &l, FDWALLCOLORS + lightlev); // floor level change
                        } else if back.ceilingheight != front.ceilingheight {
                            self.draw_mline(fb, &l, CDWALLCOLORS + lightlev); // ceiling level change
                        } else if cheating {
                            self.draw_mline(fb, &l, TSWALLCOLORS + lightlev);
                        }
                    }
                    _ => {}
                }
            } else if plr.powers[PowerType::PwAllmap as usize] != 0 && line.flags & ML_DONTDRAW == 0
            {
                self.draw_mline(fb, &l, GRAYS + 3);
            }
        }
    }

    /// Port of `AM_drawLineCharacter`. Draws a vector graphic
    /// according to numerous parameters.
    #[allow(clippy::too_many_arguments)]
    fn draw_line_character(
        &self,
        fb: &mut [u8],
        lineguy: &[MLine],
        scale: Fixed,
        angle: Angle,
        color: i32,
        x: Fixed,
        y: Fixed,
    ) {
        let transform = |mut p: MPoint| -> MPoint {
            if scale != 0 {
                p.x = fixed_mul(scale, p.x);
                p.y = fixed_mul(scale, p.y);
            }
            if angle != 0 {
                am_rotate(&mut p.x, &mut p.y, angle);
            }
            p.x += x;
            p.y += y;
            p
        };
        for g in lineguy {
            let l = MLine {
                a: transform(g.a),
                b: transform(g.b),
            };
            self.draw_mline(fb, &l, color);
        }
    }

    /// Port of `AM_drawMarks`. Draws the numbered writings.
    fn draw_marks(&self, v: &mut VVideo) {
        for i in 0..AM_NUMMARKPOINTS {
            if self.markpoints[i].x != -1 {
                let w = 5; // because something's wrong with the wad, i guess
                let h = 6; // because something's wrong with the wad, i guess
                let fx = self.cxmtof(self.markpoints[i].x);
                let fy = self.cymtof(self.markpoints[i].y);
                if fx >= self.f_x && fx <= self.f_w - w && fy >= self.f_y && fy <= self.f_h - h {
                    v.v_draw_patch(fx, fy, 0, &self.marknums[i]);
                }
            }
        }
    }
}

/// Port of `AM_rotate`. Rotation in 2D. Used to rotate player arrow
/// line character.
fn am_rotate(x: &mut Fixed, y: &mut Fixed, a: Angle) {
    let idx = (a >> ANGLETOFINESHIFT) as usize;
    let cos = crate::tables::fine_cosine(idx);
    let sin = FINESINE[idx];
    let tmpx = fixed_mul(*x, cos) - fixed_mul(*y, sin);
    *y = fixed_mul(*x, sin) + fixed_mul(*y, cos);
    *x = tmpx;
}

/// What the automap reads/writes outside itself.
pub struct AmCtx<'a> {
    /// `plr` (`players[consoleplayer]`).
    pub player: &'a mut Player,
    pub thinkers: &'a mut Thinkers,
    pub level: &'a Level,
    pub wad: &'a mut WadFiles,
}

fn player_pos(ctx: &AmCtx) -> (Fixed, Fixed) {
    let mo = ctx.thinkers.mobj(ctx.player.mo).expect("player mobj");
    (mo.x, mo.y)
}

/// Sends `ev` to the status bar (`ST_Responder`).
fn st_notify(ev: Event, ctx: &mut AmCtx) {
    st_responder(&ev, ctx.player, ctx.thinkers, ctx.wad);
}

/// Port of `AM_initVariables`.
fn init_variables(ctx: &mut AmCtx) {
    doomstat::state_mut().automapactive = true;

    with(|s| {
        s.f_oldloc.x = i32::MAX;
        s.amclock = 0;
        s.lightlev = 0;

        s.m_paninc = MPoint::default();
        s.ftom_zoommul = FRACUNIT;
        s.mtof_zoommul = FRACUNIT;

        s.m_w = s.ftom(s.f_w);
        s.m_h = s.ftom(s.f_h);
    });

    // (the original picks the first player in the game if the console
    // player isn't; single player here.)
    let (px, py) = player_pos(ctx);
    with(|s| {
        s.m_x = px - s.m_w / 2;
        s.m_y = py - s.m_h / 2;
        s.change_window_loc();

        // for saving & restoring
        s.old_m_x = s.m_x;
        s.old_m_y = s.m_y;
        s.old_m_w = s.m_w;
        s.old_m_h = s.m_h;
    });

    // inform the status bar of the change
    st_notify(
        Event {
            event_type: EvType::KeyUp,
            data1: AM_MSGENTERED,
            data2: 0,
            data3: 0,
        },
        ctx,
    );
}

/// Port of `AM_loadPics`.
fn load_pics(wad: &mut WadFiles) {
    let pics: Vec<PatchData> = (0..10)
        .map(|i| std::rc::Rc::from(wad.cache_lump_name(&format!("AMMNUM{i}"), PurgeTag::Static)))
        .collect();
    with(|s| s.marknums = pics);
}

/// Port of `AM_Stop`.
pub fn am_stop(ctx: &mut AmCtx) {
    // AM_unloadPics: the patches are owned `Rc`s, nothing to release.
    doomstat::state_mut().automapactive = false;
    // (Built with the fields out of order in the original — see the
    // module docs.)
    st_notify(
        Event {
            event_type: EvType::KeyDown,
            data1: EvType::KeyUp as i32,
            data2: AM_MSGEXITED,
            data3: 0,
        },
        ctx,
    );
    with(|s| s.stopped = true);
}

/// Port of `AM_Start`.
pub fn am_start(ctx: &mut AmCtx) {
    if !with(|s| s.stopped) {
        am_stop(ctx);
    }
    with(|s| s.stopped = false);

    let st = doomstat::state();
    let (gamemap, gameepisode) = (st.gamemap, st.gameepisode);
    with(|s| {
        if s.lastlevel != gamemap || s.lastepisode != gameepisode {
            s.level_init(ctx.level);
            s.lastlevel = gamemap;
            s.lastepisode = gameepisode;
        }
    });
    init_variables(ctx);
    load_pics(ctx.wad);
}

/// Port of `AM_Responder`. Handle events (user inputs) in automap
/// mode; returns true when the event was eaten.
pub fn am_responder(ev: &Event, ctx: &mut AmCtx) -> bool {
    let mut rc = false;

    if !doomstat::state().automapactive {
        if ev.event_type == EvType::KeyDown && ev.data1 == AM_STARTKEY {
            am_start(ctx);
            rc = true;
        }
    } else if ev.event_type == EvType::KeyDown {
        rc = true;
        match ev.data1 {
            AM_PANRIGHTKEY => {
                // pan right
                with(|s| {
                    if !s.followplayer {
                        s.m_paninc.x = s.ftom(F_PANINC);
                    } else {
                        rc = false;
                    }
                });
            }
            AM_PANLEFTKEY => {
                // pan left
                with(|s| {
                    if !s.followplayer {
                        s.m_paninc.x = -s.ftom(F_PANINC);
                    } else {
                        rc = false;
                    }
                });
            }
            AM_PANUPKEY => {
                // pan up
                with(|s| {
                    if !s.followplayer {
                        s.m_paninc.y = s.ftom(F_PANINC);
                    } else {
                        rc = false;
                    }
                });
            }
            AM_PANDOWNKEY => {
                // pan down
                with(|s| {
                    if !s.followplayer {
                        s.m_paninc.y = -s.ftom(F_PANINC);
                    } else {
                        rc = false;
                    }
                });
            }
            AM_ZOOMOUTKEY => {
                // zoom out
                with(|s| {
                    s.mtof_zoommul = M_ZOOMOUT;
                    s.ftom_zoommul = M_ZOOMIN;
                });
            }
            AM_ZOOMINKEY => {
                // zoom in
                with(|s| {
                    s.mtof_zoommul = M_ZOOMIN;
                    s.ftom_zoommul = M_ZOOMOUT;
                });
            }
            AM_ENDKEY => {
                with(|s| s.bigstate = false);
                am_stop(ctx);
            }
            AM_GOBIGKEY => {
                let pos = player_pos(ctx);
                with(|s| {
                    s.bigstate = !s.bigstate;
                    if s.bigstate {
                        s.save_scale_and_loc();
                        s.min_out_window_scale();
                    } else {
                        s.restore_scale_and_loc(pos);
                    }
                });
            }
            AM_FOLLOWKEY => {
                let on = with(|s| {
                    s.followplayer = !s.followplayer;
                    s.f_oldloc.x = i32::MAX;
                    s.followplayer
                });
                ctx.player.message = Some(if on {
                    msg::AMSTR_FOLLOWON
                } else {
                    msg::AMSTR_FOLLOWOFF
                });
            }
            AM_GRIDKEY => {
                let on = with(|s| {
                    s.grid = !s.grid;
                    s.grid
                });
                ctx.player.message = Some(if on {
                    msg::AMSTR_GRIDON
                } else {
                    msg::AMSTR_GRIDOFF
                });
            }
            AM_MARKKEY => {
                let n = with(|s| s.markpointnum);
                // (`static char buffer[20]` in C; leaked here, a handful
                // of times per session at most.)
                let text = format!("{} {}", msg::AMSTR_MARKEDSPOT, n);
                ctx.player.message = Some(Box::leak(text.into_boxed_str()));
                with(|s| s.add_mark());
            }
            AM_CLEARMARKKEY => {
                with(|s| s.clear_marks());
                ctx.player.message = Some(msg::AMSTR_MARKSCLEARED);
            }
            _ => {
                rc = false;
            }
        }

        if !doomstat::state().deathmatch && with(|s| s.cheat_amap.check(ev.data1 as u8)) {
            rc = false;
            with(|s| s.cheating = (s.cheating + 1) % 3);
        }
    } else if ev.event_type == EvType::KeyUp {
        rc = false;
        with(|s| match ev.data1 {
            AM_PANRIGHTKEY | AM_PANLEFTKEY => {
                if !s.followplayer {
                    s.m_paninc.x = 0;
                }
            }
            AM_PANUPKEY | AM_PANDOWNKEY => {
                if !s.followplayer {
                    s.m_paninc.y = 0;
                }
            }
            AM_ZOOMOUTKEY | AM_ZOOMINKEY => {
                s.mtof_zoommul = FRACUNIT;
                s.ftom_zoommul = FRACUNIT;
            }
            _ => {}
        });
    }

    rc
}

/// Port of `AM_Ticker`. Called by the main loop; `pos` is
/// `(plr->mo->x, plr->mo->y)`.
pub fn am_ticker(plr_pos: (Fixed, Fixed)) {
    if !doomstat::state().automapactive {
        return;
    }

    with(|s| {
        s.amclock += 1;

        if s.followplayer {
            s.do_follow_player(plr_pos);
        }

        // Change the zoom if necessary
        if s.ftom_zoommul != FRACUNIT {
            s.change_window_scale();
        }

        // Change x,y location
        if s.m_paninc.x != 0 || s.m_paninc.y != 0 {
            s.change_window_loc();
        }
    });
}

/// Port of `AM_Drawer`. Draws the whole automap over `screens[0]`.
pub fn am_drawer(
    v: &mut VVideo,
    level: &Level,
    thinkers: &Thinkers,
    plr: &Player,
    players: &[Player],
) {
    if !doomstat::state().automapactive {
        return;
    }
    let netgame = doomstat::state().netgame;
    let deathmatch = doomstat::state().deathmatch;
    let displayplayer = doomstat::state().displayplayer as usize;
    let playeringame = doomstat::state().playeringame;
    let _ = MAXPLAYERS;

    with(|s| {
        // AM_clearFB(BACKGROUND)
        let n = (s.f_w * s.f_h) as usize;
        v.screens[0][..n].fill(BACKGROUND as u8);

        let bmaporgx = level.bmaporgx;
        let bmaporgy = level.bmaporgy;
        {
            let fb = &mut v.screens[0];
            if s.grid {
                s.draw_grid(fb, bmaporgx, bmaporgy, GRIDCOLORS);
            }
            s.draw_walls(fb, level, plr);

            // AM_drawPlayers
            if !netgame {
                let mo = thinkers.mobj(plr.mo).expect("player mobj");
                if s.cheating != 0 {
                    s.draw_line_character(fb, &CHEAT_PLAYER_ARROW, 0, mo.angle, WHITE, mo.x, mo.y);
                } else {
                    s.draw_line_character(fb, &PLAYER_ARROW, 0, mo.angle, WHITE, mo.x, mo.y);
                }
            } else {
                let their_colors = [GREENS, GRAYS, BROWNS, REDS];
                for (i, p) in players.iter().enumerate().take(MAXPLAYERS as usize) {
                    // in deathmatch only your own arrow (the original's
                    // `(deathmatch && !singledemo) && p != plr`)
                    if deathmatch && i != displayplayer {
                        continue;
                    }
                    if !playeringame[i] {
                        continue;
                    }
                    let Some(mo) = thinkers.mobj(p.mo) else {
                        continue;
                    };
                    let color = if p.power(crate::doomdef::PowerType::PwInvisibility) != 0 {
                        246 // *close* to black (the original's own comment)
                    } else {
                        their_colors[i]
                    };
                    s.draw_line_character(fb, &PLAYER_ARROW, 0, mo.angle, color, mo.x, mo.y);
                }
            }

            if s.cheating == 2 {
                // AM_drawThings(THINGCOLORS, THINGRANGE)
                for sector in &level.sectors {
                    let mut t = sector.thinglist;
                    while let Some(id) = t {
                        let Some(m) = thinkers.mobj(id) else { break };
                        s.draw_line_character(
                            fb,
                            &THINTRIANGLE_GUY,
                            16 << FRACBITS,
                            m.angle,
                            THINGCOLORS + s.lightlev,
                            m.x,
                            m.y,
                        );
                        t = m.snext;
                    }
                }
            }

            // AM_drawCrosshair(XHAIRCOLORS): single point for now
            let idx = ((s.f_w * (s.f_h + 1)) / 2) as usize;
            fb[idx] = XHAIRCOLORS as u8;
        }

        s.draw_marks(v);
        v.v_mark_rect(s.f_x, s.f_y, s.f_w, s.f_h);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r_defs::Vertex;

    /// A state with a 100x50 window showing the map rectangle
    /// (0,0)..(800,400) map units at 1/8 px per unit (exactly
    /// representable); map coordinates are `n << 16`.
    fn state() -> AmState {
        let mut s = AmState::new();
        s.f_w = 100;
        s.f_h = 50;
        s.scale_mtof = FRACUNIT / 8;
        s.scale_ftom = fixed_div(FRACUNIT, s.scale_mtof);
        s.m_x = 0;
        s.m_y = 0;
        s.m_w = s.ftom(s.f_w);
        s.m_h = s.ftom(s.f_h);
        s.m_x2 = s.m_x + s.m_w;
        s.m_y2 = s.m_y + s.m_h;
        s
    }

    fn mp(x: i32, y: i32) -> MPoint {
        MPoint {
            x: x << 16,
            y: y << 16,
        }
    }

    #[test]
    fn constants_match_the_original() {
        assert_eq!(R, 1_198_372);
        assert_eq!(REDS, 176);
        assert_eq!(WHITE, 209);
        assert_eq!(GRIDCOLORS, 104);
        assert_eq!(INITSCALEMTOF, 13107);
        assert_eq!(M_ZOOMIN, 66846);
        assert_eq!(M_ZOOMOUT, 64250);
        assert_eq!(AM_MSGENTERED, 0x616d6500u32 as i32);
        assert_eq!(AM_MSGEXITED, 0x616d7800u32 as i32);
    }

    #[test]
    fn map_to_frame_conversion_flips_y_and_scales() {
        let s = state();
        // 800 map units -> 100 px; y grows upward on the map, downward
        // on the screen.
        assert_eq!(s.cxmtof(800 << 16), 100);
        assert_eq!(s.cxmtof(0), 0);
        assert_eq!(s.cymtof(0), 50);
        assert_eq!(s.cymtof(400 << 16), 0);
        assert_eq!(s.ftom(10), 80 << 16);
        // fractions truncate (100 units = 12.5 px -> 12)
        assert_eq!(s.cxmtof(100 << 16), 12);
    }

    #[test]
    fn clipping_rejects_trims_and_passes() {
        let s = state();
        let line = |ax, ay, bx, by| MLine {
            a: mp(ax, ay),
            b: mp(bx, by),
        };
        // inside: unchanged
        let fl = s.clip_mline(&line(100, 100, 400, 300)).unwrap();
        assert_eq!((fl.a.x, fl.a.y, fl.b.x, fl.b.y), (12, 38, 50, 13));
        // trivially outside (both above / both right)
        assert!(s.clip_mline(&line(0, 500, 300, 600)).is_none());
        assert!(s.clip_mline(&line(900, 0, 1000, 300)).is_none());
        // crossing the right edge: b is pulled back inside
        let fl = s.clip_mline(&line(400, 200, 2000, 200)).unwrap();
        assert_eq!(fl.b.x, s.f_w - 1);
        assert_eq!(fl.a.y, fl.b.y);
        // every returned endpoint lies in the window
        for l in [
            line(-500, -500, 1500, 800),
            line(-100, 200, 1000, 200),
            line(400, -100, 400, 700),
        ] {
            let fl = s.clip_mline(&l).unwrap();
            for p in [fl.a, fl.b] {
                assert!(p.x >= 0 && p.x < s.f_w && p.y >= 0 && p.y < s.f_h, "{p:?}");
            }
        }
    }

    #[test]
    fn bresenham_hits_both_endpoints_in_every_octant() {
        let s = state();
        for (a, b) in [
            ((5, 5), (60, 5)),
            ((5, 5), (5, 40)),
            ((5, 5), (60, 40)),
            ((60, 40), (5, 5)),
            ((5, 40), (30, 6)),
            ((30, 6), (5, 40)),
            ((10, 10), (10, 10)),
        ] {
            let mut fb = vec![0u8; (s.f_w * s.f_h) as usize];
            let fl = FLine {
                a: FPoint { x: a.0, y: a.1 },
                b: FPoint { x: b.0, y: b.1 },
            };
            s.draw_fline(&mut fb, &fl, 7);
            assert_eq!(fb[(a.1 * s.f_w + a.0) as usize], 7, "{a:?}->{b:?} start");
            assert_eq!(fb[(b.1 * s.f_w + b.0) as usize], 7, "{a:?}->{b:?} end");
            // connected: no gaps wider than one pixel between rows/cols
            let lit = fb.iter().filter(|&&p| p == 7).count() as i32;
            let span = (b.0 - a.0).abs().max((b.1 - a.1).abs()) + 1;
            assert_eq!(lit, span, "{a:?}->{b:?}: one pixel per major-axis step");
        }
    }

    #[test]
    fn out_of_range_lines_are_skipped() {
        let s = state();
        let mut fb = vec![0u8; (s.f_w * s.f_h) as usize];
        s.draw_fline(
            &mut fb,
            &FLine {
                a: FPoint { x: -1, y: 0 },
                b: FPoint { x: 5, y: 5 },
            },
            9,
        );
        assert!(fb.iter().all(|&p| p == 0));
    }

    #[test]
    fn rotation_by_ninety_degrees() {
        let (mut x, mut y) = (FRACUNIT, 0);
        am_rotate(&mut x, &mut y, 0x4000_0000); // ANG90
        assert!(x.abs() < 100, "cos 90 ~ 0, got {x}");
        assert!((y - FRACUNIT).abs() < 100, "sin 90 ~ 1, got {y}");
    }

    #[test]
    fn bounds_and_initial_scale_come_from_the_vertexes() {
        let mut s = AmState::new();
        s.f_w = 320;
        s.f_h = 168;
        let mut level = Level::default();
        level.vertexes = vec![
            Vertex {
                x: -1000 << 16,
                y: -500 << 16,
            },
            Vertex {
                x: 3000 << 16,
                y: 700 << 16,
            },
            Vertex { x: 0, y: 0 },
        ];
        s.find_min_max_boundaries(&level);
        assert_eq!((s.min_x, s.max_x), (-1000 << 16, 3000 << 16));
        assert_eq!((s.min_y, s.max_y), (-500 << 16, 700 << 16));
        // fits the whole level: limited by the wider side (4000 units
        // into 320 px -> 0.08 px/unit; 1200 into 168 -> 0.14)
        assert_eq!(s.min_scale_mtof, fixed_div(320 << 16, 4000 << 16));
        assert_eq!(s.max_scale_mtof, fixed_div(168 << 16, 2 * PLAYERRADIUS));

        s.level_init(&level);
        // starts at 70% of the way out
        assert_eq!(
            s.scale_mtof,
            fixed_div(s.min_scale_mtof, (0.7 * FRACUNIT as f64) as i32)
        );
        assert!(s.markpoints.iter().all(|p| p.x == -1));
    }

    #[test]
    fn marks_wrap_after_ten() {
        let mut s = state();
        s.clear_marks();
        for _ in 0..12 {
            s.add_mark();
        }
        assert_eq!(s.markpointnum, 2);
        assert!(s.markpoints.iter().all(|p| p.x != -1));
        s.clear_marks();
        assert_eq!(s.markpointnum, 0);
    }

    #[test]
    fn panning_stops_at_the_level_edges_and_turns_follow_off() {
        let mut s = state();
        s.min_x = 0;
        s.max_x = 1000 << 16;
        s.min_y = 0;
        s.max_y = 500 << 16;
        s.followplayer = true;
        s.m_paninc = MPoint {
            x: 5000 << 16,
            y: 0,
        };
        s.change_window_loc();
        assert!(!s.followplayer, "panning by hand stops following");
        assert_eq!(s.f_oldloc.x, i32::MAX);
        // the window's centre is clamped to the level's right edge
        assert_eq!(s.m_x + s.m_w / 2, s.max_x);
        assert_eq!(s.m_x2, s.m_x + s.m_w);
    }

    #[test]
    fn zooming_is_clamped_to_the_scale_limits() {
        let mut s = state();
        s.min_scale_mtof = FRACUNIT / 20;
        s.max_scale_mtof = FRACUNIT / 5;
        s.mtof_zoommul = M_ZOOMIN;
        for _ in 0..200 {
            s.change_window_scale();
        }
        assert_eq!(s.scale_mtof, s.max_scale_mtof, "zoom in stops at the max");
        s.mtof_zoommul = M_ZOOMOUT;
        for _ in 0..400 {
            s.change_window_scale();
        }
        assert_eq!(s.scale_mtof, s.min_scale_mtof, "zoom out stops at the min");
        // the window keeps its centre while the scale changes
        let (cx, cy) = (s.m_x + s.m_w / 2, s.m_y + s.m_h / 2);
        s.mtof_zoommul = M_ZOOMIN;
        s.change_window_scale();
        assert!(((s.m_x + s.m_w / 2) - cx).abs() <= 2 && ((s.m_y + s.m_h / 2) - cy).abs() <= 2);
    }

    #[test]
    fn save_and_restore_bring_back_scale_and_location() {
        let mut s = state();
        s.followplayer = false;
        s.save_scale_and_loc();
        let (x, y, w) = (s.m_x, s.m_y, s.m_w);
        s.min_scale_mtof = FRACUNIT / 50;
        s.min_out_window_scale();
        assert_ne!(s.m_w, w);
        s.restore_scale_and_loc((0, 0));
        assert_eq!((s.m_x, s.m_y, s.m_w), (x, y, w));
        // following: recentres on the player instead
        s.followplayer = true;
        s.restore_scale_and_loc((500 << 16, 250 << 16));
        assert_eq!(s.m_x, (500 << 16) - s.m_w / 2);
    }

    #[test]
    fn player_arrow_points_along_its_angle() {
        let s = state();
        let mut fb = vec![0u8; (s.f_w * s.f_h) as usize];
        // an arrow at map (300,200) facing east draws a horizontal shaft
        s.draw_line_character(&mut fb, &PLAYER_ARROW, 0, 0, WHITE, 300 << 16, 200 << 16);
        let (cx, cy) = (s.cxmtof(300 << 16), s.cymtof(200 << 16));
        assert_eq!(
            fb[(cy * s.f_w + cx) as usize],
            WHITE as u8,
            "shaft passes the centre"
        );
        assert!(fb.iter().filter(|&&p| p == WHITE as u8).count() >= 3);
    }
}
