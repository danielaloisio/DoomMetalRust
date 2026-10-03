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
//	Intermission screens.
//
//-----------------------------------------------------------------------------

//! Rust port of `wi_stuff.h` / `wi_stuff.c`. The intermission screen shown
//! between levels: the "finished" title, the kills/items/secrets/time
//! counters (single player), the per-player table (co-op) or frag
//! matrix (deathmatch), and the "you are here" map with the next
//! level marked.
//!
//! # State and flow
//!
//! The file-scope statics are one [`WiState`] in a `thread_local!`
//! behind C-named free functions ([`wi_start`], [`wi_ticker`],
//! [`wi_drawer`], [`wi_end`], [`wi_responder`]). `wbs` (a pointer into
//! `doomstat::wminfo` in the original) is a copy taken by
//! [`wi_start`].
//!
//! `WI_updateNoState` ends with `WI_End(); G_WorldDone();`. `G_WorldDone`
//! belongs to the game flow (Phase 9h), so [`wi_ticker`] *returns*
//! [`WiEvent::WorldDone`] once the screen is finished and the caller
//! makes that call.
//!
//! # Divergences and preserved quirks
//!
//! * `WI_drawAnimatedBack` starts with `if (commercial) return;` — the
//!   enum constant, always true — so the original **never draws** the
//!   map animations (they are still ticked, drawing the same random
//!   numbers). Reproduced ([`draw_animated_back`] returns at once).
//! * `dofrags` isn't reset by `WI_initNetgameStats` (it accumulates
//!   `+=` on top of last intermission's 0/1); reproduced.
//! * The animation patches of episode 2's map 9 share episode 2's map
//!   5's, like the original (`anims[1][4].p[i]`).
//! * `WI_Responder` is a stub returning false, as in the original.
//! * `Z_ChangeTag` unloading has no equivalent (patches are owned
//!   `Rc`s dropped by [`wi_end`]).

use std::cell::RefCell;
use std::rc::Rc;

use crate::d_event::Event;
use crate::d_event::{BT_ATTACK, BT_USE};
use crate::d_player::{Player, WbPlayerStruct, WbStartStruct};
use crate::doomdef::{GameMode, MAXPLAYERS, SCREENHEIGHT, SCREENWIDTH, TICRATE};
use crate::doomstat;
use crate::m_random::m_random;
use crate::s_sound::{s_change_music, s_start_sound};
use crate::sounds::{MusicEnum, Sfx};
use crate::v_video::{
    patch_height, patch_leftoffset, patch_topoffset, patch_width, PatchData, VVideo,
};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

const NUMEPISODES: usize = 4;
const NUMMAPS: usize = 9;

// GLOBAL LOCATIONS
const WI_TITLEY: i32 = 2;
const WI_SPACINGY: i32 = 33;

// SINGPLE-PLAYER STUFF
const SP_STATSX: i32 = 50;
const SP_STATSY: i32 = 50;
const SP_TIMEX: i32 = 16;
const SP_TIMEY: i32 = SCREENHEIGHT - 32;

// NET GAME STUFF
const NG_STATSY: i32 = 50;
const NG_SPACINGX: i32 = 64;

// DEATHMATCH STUFF
const DM_MATRIXX: i32 = 42;
const DM_MATRIXY: i32 = 68;
const DM_SPACINGX: i32 = 40;
const DM_TOTALSX: i32 = 269;
const DM_KILLERSX: i32 = 10;
const DM_KILLERSY: i32 = 100;
const DM_VICTIMSX: i32 = 5;
const DM_VICTIMSY: i32 = 50;

/// Foreground screen (`FB`).
const FB: usize = 0;

/// (`SHOWNEXTLOCDELAY`) in seconds.
const SHOWNEXTLOCDELAY: i32 = 4;

/// (`animenum_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnimType {
    Always,
    Random,
    Level,
}

/// The static part of an `anim_t`: `{type, period, nanims, loc, data1}`.
#[derive(Debug, Clone, Copy)]
struct AnimDef {
    kind: AnimType,
    period: i32,
    nanims: usize,
    loc: (i32, i32),
    data1: i32,
    data2: i32,
}

const fn always(period: i32, x: i32, y: i32) -> AnimDef {
    AnimDef {
        kind: AnimType::Always,
        period,
        nanims: 3,
        loc: (x, y),
        data1: 0,
        data2: 0,
    }
}

const fn level(nanims: usize, x: i32, y: i32, data1: i32) -> AnimDef {
    AnimDef {
        kind: AnimType::Level,
        period: TICRATE / 3,
        nanims,
        loc: (x, y),
        data1,
        data2: 0,
    }
}

/// Locations of the level markers per episode (`lnodes`).
const LNODES: [[(i32, i32); NUMMAPS]; 3] = [
    [
        (185, 164),
        (148, 143),
        (69, 122),
        (209, 102),
        (116, 89),
        (166, 55),
        (71, 56),
        (135, 29),
        (71, 24),
    ],
    [
        (254, 25),
        (97, 50),
        (188, 64),
        (128, 78),
        (214, 92),
        (133, 130),
        (208, 136),
        (148, 140),
        (235, 158),
    ],
    [
        (156, 168),
        (48, 154),
        (174, 95),
        (265, 75),
        (130, 48),
        (279, 23),
        (198, 48),
        (140, 25),
        (281, 136),
    ],
];

/// `epsd0animinfo`.
const EPSD0_ANIMS: [AnimDef; 10] = [
    always(TICRATE / 3, 224, 104),
    always(TICRATE / 3, 184, 160),
    always(TICRATE / 3, 112, 136),
    always(TICRATE / 3, 72, 112),
    always(TICRATE / 3, 88, 96),
    always(TICRATE / 3, 64, 48),
    always(TICRATE / 3, 192, 40),
    always(TICRATE / 3, 136, 16),
    always(TICRATE / 3, 80, 16),
    always(TICRATE / 3, 64, 24),
];

/// `epsd1animinfo`.
const EPSD1_ANIMS: [AnimDef; 9] = [
    level(1, 128, 136, 1),
    level(1, 128, 136, 2),
    level(1, 128, 136, 3),
    level(1, 128, 136, 4),
    level(1, 128, 136, 5),
    level(1, 128, 136, 6),
    level(1, 128, 136, 7),
    level(3, 192, 144, 8),
    level(1, 128, 136, 8),
];

/// `epsd2animinfo`.
const EPSD2_ANIMS: [AnimDef; 6] = [
    always(TICRATE / 3, 104, 168),
    always(TICRATE / 3, 40, 136),
    always(TICRATE / 3, 160, 96),
    always(TICRATE / 3, 104, 80),
    always(TICRATE / 3, 120, 32),
    always(TICRATE / 4, 40, 0),
];

fn anim_defs(epsd: i32) -> &'static [AnimDef] {
    match epsd {
        0 => &EPSD0_ANIMS,
        1 => &EPSD1_ANIMS,
        _ => &EPSD2_ANIMS,
    }
}

/// An `anim_t`'s runtime half.
#[derive(Clone, Default)]
struct AnimRun {
    p: Vec<PatchData>,
    nexttic: i32,
    ctr: i32,
}

/// (`stateenum_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WiMode {
    NoState,
    StatCount,
    ShowNextLoc,
}

/// What [`wi_ticker`] asks of the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WiEvent {
    None,
    /// The intermission is over: call `G_WorldDone`.
    WorldDone,
}

/// The loaded graphics.
struct WiGfx {
    bg: PatchData,
    yah: Vec<PatchData>,
    splat: Option<PatchData>,
    percent: PatchData,
    colon: PatchData,
    num: Vec<PatchData>,
    wiminus: PatchData,
    finished: PatchData,
    entering: PatchData,
    sp_secret: PatchData,
    kills: PatchData,
    secret: PatchData,
    items: PatchData,
    frags: PatchData,
    time: PatchData,
    par: PatchData,
    sucks: PatchData,
    killers: PatchData,
    victims: PatchData,
    total: PatchData,
    star: PatchData,
    bstar: PatchData,
    p: Vec<PatchData>,
    #[allow(dead_code)]
    bp: Vec<PatchData>,
    lnames: Vec<PatchData>,
}

/// The file-scope statics of `wi_stuff.c`.
struct WiState {
    acceleratestage: bool,
    /// The player whose stats are shown (`me`).
    me: usize,
    state: WiMode,
    wbs: WbStartStruct,
    plrs: [WbPlayerStruct; MAXPLAYERS as usize],
    /// Used for general timing.
    cnt: i32,
    /// Used for timing of background animation.
    bcnt: i32,
    cnt_kills: [i32; MAXPLAYERS as usize],
    cnt_items: [i32; MAXPLAYERS as usize],
    cnt_secret: [i32; MAXPLAYERS as usize],
    cnt_time: i32,
    cnt_par: i32,
    cnt_pause: i32,
    gfx: WiGfx,
    anims: Vec<AnimRun>,
    snl_pointeron: bool,
    dm_state: i32,
    dm_frags: [[i32; MAXPLAYERS as usize]; MAXPLAYERS as usize],
    dm_totals: [i32; MAXPLAYERS as usize],
    cnt_frags: [i32; MAXPLAYERS as usize],
    dofrags: i32,
    ng_state: i32,
    sp_state: i32,
    playeringame: [bool; MAXPLAYERS as usize],
    gamemode: GameMode,
    deathmatch: bool,
    netgame: bool,
}

thread_local! {
    static WI: RefCell<Option<WiState>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut WiState) -> R) -> R {
    WI.with(|s| f(s.borrow_mut().as_mut().expect("WI_Start has not run")))
}

/// Whether an intermission is in progress on this thread.
pub fn wi_is_active() -> bool {
    WI.with(|s| s.borrow().is_some())
}

/// The current sub-state (diagnostics and tests).
pub fn wi_mode() -> WiMode {
    with(|s| s.state)
}

fn sound(sfx: Sfx) {
    s_start_sound(None, sfx);
}

/// Port of `WI_Responder`. Always false.
pub fn wi_responder(_ev: &Event) -> bool {
    false
}

impl WiState {
    fn pw(&self, patch: &PatchData) -> i32 {
        patch_width(patch)
    }

    /// Port of `WI_slamBackground`.
    fn slam_background(&self, v: &mut VVideo) {
        let (screen0, rest) = v.screens.split_at_mut(1);
        screen0[0][..(SCREENWIDTH * SCREENHEIGHT) as usize]
            .copy_from_slice(&rest[0][..(SCREENWIDTH * SCREENHEIGHT) as usize]);
        v.v_mark_rect(0, 0, SCREENWIDTH, SCREENHEIGHT);
    }

    /// Port of `WI_drawLF`. Draws "<Levelname> Finished!".
    fn draw_lf(&self, v: &mut VVideo) {
        let lname = &self.gfx.lnames[self.wbs.last as usize];
        let mut y = WI_TITLEY;

        // draw <LevelName>
        v.v_draw_patch((SCREENWIDTH - patch_width(lname)) / 2, y, FB, lname);

        // draw "Finished!"
        y += (5 * patch_height(lname)) / 4;
        v.v_draw_patch(
            (SCREENWIDTH - patch_width(&self.gfx.finished)) / 2,
            y,
            FB,
            &self.gfx.finished,
        );
    }

    /// Port of `WI_drawEL`. Draws "Entering <LevelName>".
    fn draw_el(&self, v: &mut VVideo) {
        let lname = &self.gfx.lnames[self.wbs.next as usize];
        let mut y = WI_TITLEY;

        // draw "Entering"
        v.v_draw_patch(
            (SCREENWIDTH - patch_width(&self.gfx.entering)) / 2,
            y,
            FB,
            &self.gfx.entering,
        );

        // draw level
        y += (5 * patch_height(lname)) / 4;
        v.v_draw_patch((SCREENWIDTH - patch_width(lname)) / 2, y, FB, lname);
    }

    /// Port of `WI_drawOnLnode`. Draws a patch (the first of `c` that
    /// fits the screen) on a level marker.
    fn draw_on_lnode(&self, v: &mut VVideo, n: usize, c: &[PatchData]) {
        let (nx, ny) = LNODES[self.wbs.epsd as usize][n];
        let mut fits = false;
        let mut i = 0;
        loop {
            let left = nx - patch_leftoffset(&c[i]);
            let top = ny - patch_topoffset(&c[i]);
            let right = left + patch_width(&c[i]);
            let bottom = top + patch_height(&c[i]);

            if left >= 0 && right < SCREENWIDTH && top >= 0 && bottom < SCREENHEIGHT {
                fits = true;
            } else {
                i += 1;
            }
            if fits || i == 2 {
                break;
            }
        }

        if fits && i < 2 {
            v.v_draw_patch(nx, ny, FB, &c[i]);
        } else {
            // DEBUG
            println!("Could not place patch on level {}", n + 1);
        }
    }

    /// Port of `WI_initAnimatedBack`.
    fn init_animated_back(&mut self) {
        if self.gamemode == GameMode::Commercial {
            return;
        }
        if self.wbs.epsd > 2 {
            return;
        }

        for (i, def) in anim_defs(self.wbs.epsd).iter().enumerate() {
            let a = &mut self.anims[i];
            a.ctr = -1;

            // specify the next time to draw it
            match def.kind {
                AnimType::Always => {
                    a.nexttic = self.bcnt + 1 + (m_random() % def.period);
                }
                AnimType::Random => {
                    a.nexttic = self.bcnt + 1 + def.data2 + (m_random() % def.data1);
                }
                AnimType::Level => {
                    a.nexttic = self.bcnt + 1;
                }
            }
        }
    }

    /// Port of `WI_updateAnimatedBack`.
    fn update_animated_back(&mut self) {
        if self.gamemode == GameMode::Commercial {
            return;
        }
        if self.wbs.epsd > 2 {
            return;
        }

        for (i, def) in anim_defs(self.wbs.epsd).iter().enumerate() {
            let a = &mut self.anims[i];
            if self.bcnt == a.nexttic {
                match def.kind {
                    AnimType::Always => {
                        a.ctr += 1;
                        if a.ctr >= def.nanims as i32 {
                            a.ctr = 0;
                        }
                        a.nexttic = self.bcnt + def.period;
                    }
                    AnimType::Random => {
                        a.ctr += 1;
                        if a.ctr == def.nanims as i32 {
                            a.ctr = -1;
                            a.nexttic = self.bcnt + def.data2 + (m_random() % def.data1);
                        } else {
                            a.nexttic = self.bcnt + def.period;
                        }
                    }
                    AnimType::Level => {
                        // gawd-awful hack for level anims
                        if !(self.state == WiMode::StatCount && i == 7)
                            && self.wbs.next == def.data1
                        {
                            a.ctr += 1;
                            if a.ctr == def.nanims as i32 {
                                a.ctr -= 1;
                            }
                            a.nexttic = self.bcnt + def.period;
                        }
                    }
                }
            }
        }
    }

    /// Port of `WI_drawAnimatedBack`. See the module docs: the
    /// original's `if (commercial) return;` is always true, so this
    /// never draws anything.
    fn draw_animated_back(&self, _v: &mut VVideo) {}

    /// Port of `WI_drawNum`. Draws a number; returns the x of its
    /// left edge. `digits < 0` means "as many as needed".
    fn draw_num(&self, v: &mut VVideo, mut x: i32, y: i32, mut n: i32, mut digits: i32) -> i32 {
        let fontwidth = patch_width(&self.gfx.num[0]);

        if digits < 0 {
            if n == 0 {
                // make variable-length zeros 1 digit long
                digits = 1;
            } else {
                // figure out # of digits in #
                digits = 0;
                let mut temp = n;
                while temp != 0 {
                    temp /= 10;
                    digits += 1;
                }
            }
        }

        let neg = n < 0;
        if neg {
            n = -n;
        }

        // if non-number, do not draw it
        if n == 1994 {
            return 0;
        }

        // draw the new number
        while digits > 0 {
            digits -= 1;
            x -= fontwidth;
            v.v_draw_patch(x, y, FB, &self.gfx.num[(n % 10) as usize]);
            n /= 10;
        }

        // draw a minus sign if necessary
        if neg {
            x -= 8;
            v.v_draw_patch(x, y, FB, &self.gfx.wiminus);
        }

        x
    }

    /// Port of `WI_drawPercent`.
    fn draw_percent(&self, v: &mut VVideo, x: i32, y: i32, p: i32) {
        if p < 0 {
            return;
        }
        v.v_draw_patch(x, y, FB, &self.gfx.percent);
        self.draw_num(v, x, y, p, -1);
    }

    /// Port of `WI_drawTime`. Display level completion time and par,
    /// or "sucks" if over 1 hour.
    fn draw_time(&self, v: &mut VVideo, mut x: i32, y: i32, t: i32) {
        if t < 0 {
            return;
        }

        if t <= 61 * 59 {
            let mut div = 1;
            loop {
                let n = (t / div) % 60;
                x = self.draw_num(v, x, y, n, 2) - patch_width(&self.gfx.colon);
                div *= 60;

                // draw
                if div == 60 || t / div != 0 {
                    v.v_draw_patch(x, y, FB, &self.gfx.colon);
                }
                if t / div == 0 {
                    break;
                }
            }
        } else {
            // "sucks"
            v.v_draw_patch(x - patch_width(&self.gfx.sucks), y, FB, &self.gfx.sucks);
        }
    }

    /// Port of `WI_initNoState`.
    fn init_no_state(&mut self) {
        self.state = WiMode::NoState;
        self.acceleratestage = false;
        self.cnt = 10;
    }

    /// Port of `WI_updateNoState`.
    fn update_no_state(&mut self) -> WiEvent {
        self.update_animated_back();

        self.cnt -= 1;
        if self.cnt == 0 {
            return WiEvent::WorldDone;
        }
        WiEvent::None
    }

    /// Port of `WI_initShowNextLoc`.
    fn init_show_next_loc(&mut self) {
        self.state = WiMode::ShowNextLoc;
        self.acceleratestage = false;
        self.cnt = SHOWNEXTLOCDELAY * TICRATE;

        self.init_animated_back();
    }

    /// Port of `WI_updateShowNextLoc`.
    fn update_show_next_loc(&mut self) {
        self.update_animated_back();

        self.cnt -= 1;
        if self.cnt == 0 || self.acceleratestage {
            self.init_no_state();
        } else {
            self.snl_pointeron = (self.cnt & 31) < 20;
        }
    }

    /// Port of `WI_drawShowNextLoc`.
    fn draw_show_next_loc(&self, v: &mut VVideo) {
        self.slam_background(v);

        // draw animated background
        self.draw_animated_back(v);

        if self.gamemode != GameMode::Commercial {
            if self.wbs.epsd > 2 {
                self.draw_el(v);
                return;
            }

            let last = if self.wbs.last == 8 {
                self.wbs.next - 1
            } else {
                self.wbs.last
            };

            // draw a splat on taken cities.
            if let Some(splat) = &self.gfx.splat {
                for i in 0..=last {
                    self.draw_on_lnode(v, i as usize, std::slice::from_ref(splat));
                }

                // splat the secret level?
                if self.wbs.didsecret {
                    self.draw_on_lnode(v, 8, std::slice::from_ref(splat));
                }
            }

            // draw a flashing "you are here"
            if self.snl_pointeron {
                self.draw_on_lnode(v, self.wbs.next as usize, &self.gfx.yah);
            }
        }

        // draws which level you are entering..
        if self.gamemode != GameMode::Commercial || self.wbs.next != 30 {
            self.draw_el(v);
        }
    }

    /// Port of `WI_drawNoState`.
    fn draw_no_state(&mut self, v: &mut VVideo) {
        self.snl_pointeron = true;
        self.draw_show_next_loc(v);
    }

    /// Port of `WI_fragSum`.
    fn frag_sum(&self, playernum: usize) -> i32 {
        let mut frags = 0;
        for i in 0..MAXPLAYERS as usize {
            if self.playeringame[i] && i != playernum {
                frags += self.plrs[playernum].frags[i];
            }
        }

        // JDC hack - negative frags.
        frags -= self.plrs[playernum].frags[playernum];
        frags
    }

    /// Port of `WI_initDeathmatchStats`.
    fn init_deathmatch_stats(&mut self) {
        self.state = WiMode::StatCount;
        self.acceleratestage = false;
        self.dm_state = 1;

        self.cnt_pause = TICRATE;

        for i in 0..MAXPLAYERS as usize {
            if self.playeringame[i] {
                for j in 0..MAXPLAYERS as usize {
                    if self.playeringame[j] {
                        self.dm_frags[i][j] = 0;
                    }
                }
                self.dm_totals[i] = 0;
            }
        }

        self.init_animated_back();
    }

    /// Port of `WI_updateDeathmatchStats`.
    fn update_deathmatch_stats(&mut self) {
        self.update_animated_back();

        if self.acceleratestage && self.dm_state != 4 {
            self.acceleratestage = false;

            for i in 0..MAXPLAYERS as usize {
                if self.playeringame[i] {
                    for j in 0..MAXPLAYERS as usize {
                        if self.playeringame[j] {
                            self.dm_frags[i][j] = self.plrs[i].frags[j];
                        }
                    }
                    self.dm_totals[i] = self.frag_sum(i);
                }
            }

            sound(Sfx::SfxBarexp);
            self.dm_state = 4;
        }

        if self.dm_state == 2 {
            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            let mut stillticking = false;

            for i in 0..MAXPLAYERS as usize {
                if self.playeringame[i] {
                    for j in 0..MAXPLAYERS as usize {
                        if self.playeringame[j] && self.dm_frags[i][j] != self.plrs[i].frags[j] {
                            if self.plrs[i].frags[j] < 0 {
                                self.dm_frags[i][j] -= 1;
                            } else {
                                self.dm_frags[i][j] += 1;
                            }

                            self.dm_frags[i][j] = self.dm_frags[i][j].clamp(-99, 99);
                            stillticking = true;
                        }
                    }
                    self.dm_totals[i] = self.frag_sum(i).clamp(-99, 99);
                }
            }

            if !stillticking {
                sound(Sfx::SfxBarexp);
                self.dm_state += 1;
            }
        } else if self.dm_state == 4 {
            if self.acceleratestage {
                sound(Sfx::SfxSlop);

                if self.gamemode == GameMode::Commercial {
                    self.init_no_state();
                } else {
                    self.init_show_next_loc();
                }
            }
        } else if self.dm_state & 1 != 0 {
            self.cnt_pause -= 1;
            if self.cnt_pause == 0 {
                self.dm_state += 1;
                self.cnt_pause = TICRATE;
            }
        }
    }

    /// Port of `WI_drawDeathmatchStats`.
    fn draw_deathmatch_stats(&self, v: &mut VVideo) {
        let lh = WI_SPACINGY; // line height

        self.slam_background(v);

        // draw animated background
        self.draw_animated_back(v);
        self.draw_lf(v);

        // draw stat titles (top line)
        v.v_draw_patch(
            DM_TOTALSX - patch_width(&self.gfx.total) / 2,
            DM_MATRIXY - WI_SPACINGY + 10,
            FB,
            &self.gfx.total,
        );

        v.v_draw_patch(DM_KILLERSX, DM_KILLERSY, FB, &self.gfx.killers);
        v.v_draw_patch(DM_VICTIMSX, DM_VICTIMSY, FB, &self.gfx.victims);

        // draw P?
        let mut x = DM_MATRIXX + DM_SPACINGX;
        let mut y = DM_MATRIXY;

        for i in 0..MAXPLAYERS as usize {
            if self.playeringame[i] {
                let p = &self.gfx.p[i];
                v.v_draw_patch(x - patch_width(p) / 2, DM_MATRIXY - WI_SPACINGY, FB, p);
                v.v_draw_patch(DM_MATRIXX - patch_width(p) / 2, y, FB, p);

                if i == self.me {
                    v.v_draw_patch(
                        x - patch_width(p) / 2,
                        DM_MATRIXY - WI_SPACINGY,
                        FB,
                        &self.gfx.bstar,
                    );
                    v.v_draw_patch(DM_MATRIXX - patch_width(p) / 2, y, FB, &self.gfx.star);
                }
            }
            x += DM_SPACINGX;
            y += lh;
        }

        // draw stats
        let mut y = DM_MATRIXY + 10;
        let w = patch_width(&self.gfx.num[0]);

        for i in 0..MAXPLAYERS as usize {
            let mut x = DM_MATRIXX + DM_SPACINGX;

            if self.playeringame[i] {
                for j in 0..MAXPLAYERS as usize {
                    if self.playeringame[j] {
                        self.draw_num(v, x + w, y, self.dm_frags[i][j], 2);
                    }
                    x += DM_SPACINGX;
                }
                self.draw_num(v, DM_TOTALSX + w, y, self.dm_totals[i], 2);
            }
            y += WI_SPACINGY;
        }
    }

    /// `NG_STATSX`.
    fn ng_statsx(&self) -> i32 {
        32 + patch_width(&self.gfx.star) / 2 + 32 * (self.dofrags == 0) as i32
    }

    /// Port of `WI_initNetgameStats`.
    fn init_netgame_stats(&mut self) {
        self.state = WiMode::StatCount;
        self.acceleratestage = false;
        self.ng_state = 1;

        self.cnt_pause = TICRATE;

        for i in 0..MAXPLAYERS as usize {
            if !self.playeringame[i] {
                continue;
            }

            self.cnt_kills[i] = 0;
            self.cnt_items[i] = 0;
            self.cnt_secret[i] = 0;
            self.cnt_frags[i] = 0;

            self.dofrags += self.frag_sum(i);
        }

        self.dofrags = (self.dofrags != 0) as i32;

        self.init_animated_back();
    }

    /// `(plrs[i].<field> * 100) / wbs->max<field>`.
    fn pct(count: i32, max: i32) -> i32 {
        (count * 100) / max
    }

    /// Port of `WI_updateNetgameStats`.
    fn update_netgame_stats(&mut self) {
        self.update_animated_back();

        if self.acceleratestage && self.ng_state != 10 {
            self.acceleratestage = false;

            for i in 0..MAXPLAYERS as usize {
                if !self.playeringame[i] {
                    continue;
                }

                self.cnt_kills[i] = Self::pct(self.plrs[i].skills, self.wbs.maxkills);
                self.cnt_items[i] = Self::pct(self.plrs[i].sitems, self.wbs.maxitems);
                self.cnt_secret[i] = Self::pct(self.plrs[i].ssecret, self.wbs.maxsecret);

                if self.dofrags != 0 {
                    self.cnt_frags[i] = self.frag_sum(i);
                }
            }
            sound(Sfx::SfxBarexp);
            self.ng_state = 10;
        }

        if self.ng_state == 2 {
            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            let mut stillticking = false;
            for i in 0..MAXPLAYERS as usize {
                if !self.playeringame[i] {
                    continue;
                }

                self.cnt_kills[i] += 2;

                let target = Self::pct(self.plrs[i].skills, self.wbs.maxkills);
                if self.cnt_kills[i] >= target {
                    self.cnt_kills[i] = target;
                } else {
                    stillticking = true;
                }
            }

            if !stillticking {
                sound(Sfx::SfxBarexp);
                self.ng_state += 1;
            }
        } else if self.ng_state == 4 {
            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            let mut stillticking = false;
            for i in 0..MAXPLAYERS as usize {
                if !self.playeringame[i] {
                    continue;
                }

                self.cnt_items[i] += 2;
                let target = Self::pct(self.plrs[i].sitems, self.wbs.maxitems);
                if self.cnt_items[i] >= target {
                    self.cnt_items[i] = target;
                } else {
                    stillticking = true;
                }
            }
            if !stillticking {
                sound(Sfx::SfxBarexp);
                self.ng_state += 1;
            }
        } else if self.ng_state == 6 {
            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            let mut stillticking = false;
            for i in 0..MAXPLAYERS as usize {
                if !self.playeringame[i] {
                    continue;
                }

                self.cnt_secret[i] += 2;

                let target = Self::pct(self.plrs[i].ssecret, self.wbs.maxsecret);
                if self.cnt_secret[i] >= target {
                    self.cnt_secret[i] = target;
                } else {
                    stillticking = true;
                }
            }

            if !stillticking {
                sound(Sfx::SfxBarexp);
                self.ng_state += 1 + 2 * (self.dofrags == 0) as i32;
            }
        } else if self.ng_state == 8 {
            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            let mut stillticking = false;
            for i in 0..MAXPLAYERS as usize {
                if !self.playeringame[i] {
                    continue;
                }

                self.cnt_frags[i] += 1;

                let fsum = self.frag_sum(i);
                if self.cnt_frags[i] >= fsum {
                    self.cnt_frags[i] = fsum;
                } else {
                    stillticking = true;
                }
            }

            if !stillticking {
                sound(Sfx::SfxPldeth);
                self.ng_state += 1;
            }
        } else if self.ng_state == 10 {
            if self.acceleratestage {
                sound(Sfx::SfxSgcock);
                if self.gamemode == GameMode::Commercial {
                    self.init_no_state();
                } else {
                    self.init_show_next_loc();
                }
            }
        } else if self.ng_state & 1 != 0 {
            self.cnt_pause -= 1;
            if self.cnt_pause == 0 {
                self.ng_state += 1;
                self.cnt_pause = TICRATE;
            }
        }
    }

    /// Port of `WI_drawNetgameStats`.
    fn draw_netgame_stats(&self, v: &mut VVideo) {
        let pwidth = patch_width(&self.gfx.percent);
        let statsx = self.ng_statsx();

        self.slam_background(v);

        // draw animated background
        self.draw_animated_back(v);
        self.draw_lf(v);

        // draw stat titles (top line)
        let g = &self.gfx;
        v.v_draw_patch(
            statsx + NG_SPACINGX - patch_width(&g.kills),
            NG_STATSY,
            FB,
            &g.kills,
        );
        v.v_draw_patch(
            statsx + 2 * NG_SPACINGX - patch_width(&g.items),
            NG_STATSY,
            FB,
            &g.items,
        );
        v.v_draw_patch(
            statsx + 3 * NG_SPACINGX - patch_width(&g.secret),
            NG_STATSY,
            FB,
            &g.secret,
        );

        if self.dofrags != 0 {
            v.v_draw_patch(
                statsx + 4 * NG_SPACINGX - patch_width(&g.frags),
                NG_STATSY,
                FB,
                &g.frags,
            );
        }

        // draw stats
        let mut y = NG_STATSY + patch_height(&g.kills);

        for i in 0..MAXPLAYERS as usize {
            if !self.playeringame[i] {
                continue;
            }

            let mut x = statsx;
            v.v_draw_patch(x - patch_width(&g.p[i]), y, FB, &g.p[i]);

            if i == self.me {
                v.v_draw_patch(x - patch_width(&g.p[i]), y, FB, &g.star);
            }

            x += NG_SPACINGX;
            self.draw_percent(v, x - pwidth, y + 10, self.cnt_kills[i]);
            x += NG_SPACINGX;
            self.draw_percent(v, x - pwidth, y + 10, self.cnt_items[i]);
            x += NG_SPACINGX;
            self.draw_percent(v, x - pwidth, y + 10, self.cnt_secret[i]);
            x += NG_SPACINGX;

            if self.dofrags != 0 {
                self.draw_num(v, x, y + 10, self.cnt_frags[i], -1);
            }

            y += WI_SPACINGY;
        }
    }

    /// Port of `WI_initStats`.
    fn init_stats(&mut self) {
        self.state = WiMode::StatCount;
        self.acceleratestage = false;
        self.sp_state = 1;
        self.cnt_kills[0] = -1;
        self.cnt_items[0] = -1;
        self.cnt_secret[0] = -1;
        self.cnt_time = -1;
        self.cnt_par = -1;
        self.cnt_pause = TICRATE;

        self.init_animated_back();
    }

    /// Port of `WI_updateStats`.
    fn update_stats(&mut self) {
        self.update_animated_back();

        let me = self.me;
        let kills = Self::pct(self.plrs[me].skills, self.wbs.maxkills);
        let items = Self::pct(self.plrs[me].sitems, self.wbs.maxitems);
        let secret = Self::pct(self.plrs[me].ssecret, self.wbs.maxsecret);
        let time = self.plrs[me].stime / TICRATE;
        let par = self.wbs.partime / TICRATE;

        if self.acceleratestage && self.sp_state != 10 {
            self.acceleratestage = false;
            self.cnt_kills[0] = kills;
            self.cnt_items[0] = items;
            self.cnt_secret[0] = secret;
            self.cnt_time = time;
            self.cnt_par = par;
            sound(Sfx::SfxBarexp);
            self.sp_state = 10;
        }

        if self.sp_state == 2 {
            self.cnt_kills[0] += 2;

            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            if self.cnt_kills[0] >= kills {
                self.cnt_kills[0] = kills;
                sound(Sfx::SfxBarexp);
                self.sp_state += 1;
            }
        } else if self.sp_state == 4 {
            self.cnt_items[0] += 2;

            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            if self.cnt_items[0] >= items {
                self.cnt_items[0] = items;
                sound(Sfx::SfxBarexp);
                self.sp_state += 1;
            }
        } else if self.sp_state == 6 {
            self.cnt_secret[0] += 2;

            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            if self.cnt_secret[0] >= secret {
                self.cnt_secret[0] = secret;
                sound(Sfx::SfxBarexp);
                self.sp_state += 1;
            }
        } else if self.sp_state == 8 {
            if self.bcnt & 3 == 0 {
                sound(Sfx::SfxPistol);
            }

            self.cnt_time += 3;

            if self.cnt_time >= time {
                self.cnt_time = time;
            }

            self.cnt_par += 3;

            if self.cnt_par >= par {
                self.cnt_par = par;

                if self.cnt_time >= time {
                    sound(Sfx::SfxBarexp);
                    self.sp_state += 1;
                }
            }
        } else if self.sp_state == 10 {
            if self.acceleratestage {
                sound(Sfx::SfxSgcock);

                if self.gamemode == GameMode::Commercial {
                    self.init_no_state();
                } else {
                    self.init_show_next_loc();
                }
            }
        } else if self.sp_state & 1 != 0 {
            self.cnt_pause -= 1;
            if self.cnt_pause == 0 {
                self.sp_state += 1;
                self.cnt_pause = TICRATE;
            }
        }
    }

    /// Port of `WI_drawStats`.
    fn draw_stats(&self, v: &mut VVideo) {
        // line height
        let lh = (3 * patch_height(&self.gfx.num[0])) / 2;

        self.slam_background(v);

        // draw animated background
        self.draw_animated_back(v);
        self.draw_lf(v);

        let g = &self.gfx;
        v.v_draw_patch(SP_STATSX, SP_STATSY, FB, &g.kills);
        self.draw_percent(v, SCREENWIDTH - SP_STATSX, SP_STATSY, self.cnt_kills[0]);

        v.v_draw_patch(SP_STATSX, SP_STATSY + lh, FB, &g.items);
        self.draw_percent(
            v,
            SCREENWIDTH - SP_STATSX,
            SP_STATSY + lh,
            self.cnt_items[0],
        );

        v.v_draw_patch(SP_STATSX, SP_STATSY + 2 * lh, FB, &g.sp_secret);
        self.draw_percent(
            v,
            SCREENWIDTH - SP_STATSX,
            SP_STATSY + 2 * lh,
            self.cnt_secret[0],
        );

        v.v_draw_patch(SP_TIMEX, SP_TIMEY, FB, &g.time);
        self.draw_time(v, SCREENWIDTH / 2 - SP_TIMEX, SP_TIMEY, self.cnt_time);

        if self.wbs.epsd < 3 {
            v.v_draw_patch(SCREENWIDTH / 2 + SP_TIMEX, SP_TIMEY, FB, &g.par);
            self.draw_time(v, SCREENWIDTH - SP_TIMEX, SP_TIMEY, self.cnt_par);
        }
    }
}

/// Port of `WI_checkForAccelerate`: any player pressing fire or use
/// (fresh press) speeds the screen up.
fn check_for_accelerate(s: &mut WiState, players: &mut [Player]) {
    for (i, player) in players.iter_mut().enumerate().take(MAXPLAYERS as usize) {
        if s.playeringame[i] {
            if player.cmd.buttons & (BT_ATTACK as u8) != 0 {
                if !player.attackdown {
                    s.acceleratestage = true;
                }
                player.attackdown = true;
            } else {
                player.attackdown = false;
            }
            if player.cmd.buttons & (BT_USE as u8) != 0 {
                if !player.usedown {
                    s.acceleratestage = true;
                }
                player.usedown = true;
            } else {
                player.usedown = false;
            }
        }
    }
}

/// Port of `WI_Ticker`. Updates stuff each tic (the original's own
/// comment); returns [`WiEvent::WorldDone`] when it is time for
/// `G_WorldDone`. `players` is the game's player array.
pub fn wi_ticker(players: &mut [Player], wad: &mut WadFiles) -> WiEvent {
    // counter for general background animation
    let (bcnt, gamemode) = with(|s| {
        s.bcnt += 1;
        (s.bcnt, s.gamemode)
    });

    if bcnt == 1 {
        // intermission music
        if gamemode == GameMode::Commercial {
            s_change_music(wad, MusicEnum::MusDm2int as i32, true);
        } else {
            s_change_music(wad, MusicEnum::MusInter as i32, true);
        }
    }

    with(|s| {
        check_for_accelerate(s, players);

        match s.state {
            WiMode::StatCount => {
                if s.deathmatch {
                    s.update_deathmatch_stats();
                } else if s.netgame {
                    s.update_netgame_stats();
                } else {
                    s.update_stats();
                }
                WiEvent::None
            }
            WiMode::ShowNextLoc => {
                s.update_show_next_loc();
                WiEvent::None
            }
            WiMode::NoState => s.update_no_state(),
        }
    })
}

/// Port of `WI_Drawer`.
pub fn wi_drawer(v: &mut VVideo) {
    with(|s| match s.state {
        WiMode::StatCount => {
            if s.deathmatch {
                s.draw_deathmatch_stats(v);
            } else if s.netgame {
                s.draw_netgame_stats(v);
            } else {
                s.draw_stats(v);
            }
        }
        WiMode::ShowNextLoc => s.draw_show_next_loc(v),
        WiMode::NoState => s.draw_no_state(v),
    });
}

/// Port of `WI_End` (`WI_unloadData`): drops the intermission state.
pub fn wi_end() {
    WI.with(|s| *s.borrow_mut() = None);
}

/// Port of `WI_loadData`. Loads the intermission graphics and draws
/// the background onto `screens[1]`.
fn load_data(
    wad: &mut WadFiles,
    v: &mut VVideo,
    wbs: &WbStartStruct,
    gamemode: GameMode,
    french: bool,
    netgame: bool,
    deathmatch: bool,
) -> (WiGfx, Vec<AnimRun>) {
    let mut get =
        |name: &str| -> PatchData { Rc::from(wad.cache_lump_name(name, PurgeTag::Static)) };

    let mut name = if gamemode == GameMode::Commercial {
        "INTERPIC".to_string()
    } else {
        format!("WIMAP{}", wbs.epsd)
    };
    if gamemode == GameMode::Retail && wbs.epsd == 3 {
        name = "INTERPIC".to_string();
    }

    let bg = get(&name);
    // (the original draws the background onto screens[1] right here)
    v.v_draw_patch(0, 0, 1, &bg);

    let mut yah = Vec::new();
    let mut splat = None;
    let mut anims: Vec<AnimRun> = Vec::new();
    let lnames: Vec<PatchData>;

    if gamemode == GameMode::Commercial {
        // NUMCMAPS = 32
        lnames = (0..32).map(|i| get(&format!("CWILV{:02}", i))).collect();
    } else {
        lnames = (0..NUMMAPS)
            .map(|i| get(&format!("WILV{}{}", wbs.epsd, i)))
            .collect();

        // you are here
        yah.push(get("WIURH0"));
        yah.push(get("WIURH1"));

        // splat
        splat = Some(get("WISPLAT"));

        if wbs.epsd < 3 {
            let defs = anim_defs(wbs.epsd);
            for (j, def) in defs.iter().enumerate() {
                let mut run = AnimRun::default();
                for i in 0..def.nanims {
                    // MONDO HACK!
                    if wbs.epsd != 1 || j != 8 {
                        // animations
                        run.p.push(get(&format!("WIA{}{:02}{:02}", wbs.epsd, j, i)));
                    } else {
                        // HACK ALERT!
                        run.p.push(anims[4].p[i].clone());
                    }
                }
                anims.push(run);
            }
        }
    }

    // More hacks on minus sign.
    let wiminus = get("WIMINUS");
    let num: Vec<PatchData> = (0..10).map(|i| get(&format!("WINUM{i}"))).collect();
    let percent = get("WIPCNT");

    // "finished"
    let finished = get("WIF");

    // "entering"
    let entering = get("WIENTER");

    // "kills"
    let kills = get("WIOSTK");

    // "scrt"
    let secret = get("WIOSTS");

    // "secret"
    let sp_secret = get("WISCRT2");

    // Yuck.
    let items = if french && netgame && !deathmatch {
        // "items"
        get("WIOBJ")
    } else {
        get("WIOSTI")
    };

    // "frgs"
    let frags = get("WIFRGS");

    // ":"
    let colon = get("WICOLON");

    // "time"
    let time = get("WITIME");

    // "sucks"
    let sucks = get("WISUCKS");

    // "par"
    let par = get("WIPAR");

    // "killers" (vertical)
    let killers = get("WIKILRS");

    // "victims" (horiz)
    let victims = get("WIVCTMS");

    // "total"
    let total = get("WIMSTT");

    // your face
    let star = get("STFST01");

    // dead face
    let bstar = get("STFDEAD0");

    let mut p = Vec::new();
    let mut bp = Vec::new();
    for i in 0..MAXPLAYERS {
        // "1,2,3,4"
        p.push(get(&format!("STPB{i}")));
        // "1,2,3,4"
        bp.push(get(&format!("WIBP{}", i + 1)));
    }

    (
        WiGfx {
            bg,
            yah,
            splat,
            percent,
            colon,
            num,
            wiminus,
            finished,
            entering,
            sp_secret,
            kills,
            secret,
            items,
            frags,
            time,
            par,
            sucks,
            killers,
            victims,
            total,
            star,
            bstar,
            p,
            bp,
            lnames,
        },
        anims,
    )
}

/// Port of `WI_Start` (with `WI_initVariables`): begins the
/// intermission for `wbstartstruct`. `playeringame` is the game's
/// global of the same name.
pub fn wi_start(
    wbstartstruct: &WbStartStruct,
    playeringame: &[bool],
    wad: &mut WadFiles,
    v: &mut VVideo,
) {
    let st = doomstat::state();
    wi_start_for(
        wbstartstruct,
        playeringame,
        wad,
        v,
        (st.gamemode, st.french, st.netgame, st.deathmatch),
    );
}

/// [`wi_start`] with the game mode, `french`, `netgame` and
/// `deathmatch` given instead of read from `doomstat`.
pub fn wi_start_for(
    wbstartstruct: &WbStartStruct,
    playeringame: &[bool],
    wad: &mut WadFiles,
    v: &mut VVideo,
    (gamemode, french, netgame, deathmatch): (GameMode, bool, bool, bool),
) {
    // WI_initVariables
    let mut wbs = *wbstartstruct;
    if wbs.maxkills == 0 {
        wbs.maxkills = 1;
    }
    if wbs.maxitems == 0 {
        wbs.maxitems = 1;
    }
    if wbs.maxsecret == 0 {
        wbs.maxsecret = 1;
    }
    if gamemode != GameMode::Retail && wbs.epsd > 2 {
        wbs.epsd -= 3;
    }

    // WI_loadData
    let (gfx, anims) = load_data(wad, v, &wbs, gamemode, french, netgame, deathmatch);

    let mut ingame = [false; MAXPLAYERS as usize];
    for (i, g) in playeringame.iter().enumerate().take(MAXPLAYERS as usize) {
        ingame[i] = *g;
    }

    let mut state = WiState {
        acceleratestage: false,
        me: wbs.pnum as usize,
        state: WiMode::StatCount,
        plrs: wbs.plyr,
        wbs,
        cnt: 0,
        bcnt: 0,
        cnt_kills: [0; MAXPLAYERS as usize],
        cnt_items: [0; MAXPLAYERS as usize],
        cnt_secret: [0; MAXPLAYERS as usize],
        cnt_time: 0,
        cnt_par: 0,
        cnt_pause: 0,
        gfx,
        anims,
        snl_pointeron: false,
        dm_state: 0,
        dm_frags: [[0; MAXPLAYERS as usize]; MAXPLAYERS as usize],
        dm_totals: [0; MAXPLAYERS as usize],
        cnt_frags: [0; MAXPLAYERS as usize],
        dofrags: 0,
        ng_state: 0,
        sp_state: 0,
        playeringame: ingame,
        gamemode,
        deathmatch,
        netgame,
    };

    if deathmatch {
        state.init_deathmatch_stats();
    } else if netgame {
        state.init_netgame_stats();
    } else {
        state.init_stats();
    }

    WI.with(|s| *s.borrow_mut() = Some(state));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::MobjType;

    fn wad() -> Option<WadFiles> {
        let path = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut w = WadFiles::new();
        w.init_file(path);
        Some(w)
    }

    fn wbs() -> WbStartStruct {
        let mut w = WbStartStruct {
            epsd: 0,
            last: 0,
            next: 1,
            maxkills: 10,
            maxitems: 5,
            maxsecret: 2,
            partime: 30 * TICRATE,
            pnum: 0,
            ..Default::default()
        };
        w.plyr[0] = WbPlayerStruct {
            in_game: true,
            skills: 8,
            sitems: 5,
            ssecret: 1,
            stime: 100 * TICRATE,
            ..Default::default()
        };
        w
    }

    fn player() -> Player {
        let mut thinkers = crate::p_tick::Thinkers::new();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj(crate::r_defs::Mobj::blank(MobjType::MtPlayer)),
        );
        Player::for_test(mo)
    }

    struct Fixture {
        wad: WadFiles,
        v: VVideo,
        players: Vec<Player>,
    }

    fn start(wbs: &WbStartStruct, modes: (GameMode, bool, bool, bool)) -> Option<Fixture> {
        let mut wad = wad()?;
        let mut v = VVideo::new();
        wi_end();
        wi_start_for(wbs, &[true, false, false, false], &mut wad, &mut v, modes);
        Some(Fixture {
            wad,
            v,
            players: vec![player()],
        })
    }

    const SINGLE: (GameMode, bool, bool, bool) = (GameMode::Registered, false, false, false);

    impl Fixture {
        fn tick(&mut self) -> WiEvent {
            wi_ticker(&mut self.players, &mut self.wad)
        }

        fn press(&mut self, buttons: u8) {
            self.players[0].cmd.buttons = buttons;
        }
    }

    #[test]
    fn start_normalizes_the_totals_and_draws_the_background_on_screen_1() {
        let mut w = wbs();
        w.maxkills = 0;
        w.maxitems = 0;
        w.maxsecret = 0;
        let Some(f) = start(&w, SINGLE) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        with(|s| {
            assert_eq!((s.wbs.maxkills, s.wbs.maxitems, s.wbs.maxsecret), (1, 1, 1));
            assert_eq!(s.state, WiMode::StatCount);
            assert_eq!(s.sp_state, 1);
            assert_eq!(s.cnt_kills[0], -1);
        });
        assert!(
            f.v.screens[1].iter().any(|&b| b != 0),
            "WIMAP0 on screens[1]"
        );
        wi_end();
    }

    #[test]
    fn single_player_counters_run_up_to_the_real_stats() {
        let Some(mut f) = start(&wbs(), SINGLE) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut ticks = 0;
        while with(|s| s.sp_state) != 10 {
            f.tick();
            ticks += 1;
            assert!(ticks < 2000, "counters never finished");
        }
        with(|s| {
            assert_eq!(s.cnt_kills[0], 80);
            assert_eq!(s.cnt_items[0], 100);
            assert_eq!(s.cnt_secret[0], 50);
            assert_eq!(s.cnt_time, 100);
            assert_eq!(s.cnt_par, 30);
        });
        // waits for a keypress; nothing more happens
        for _ in 0..50 {
            assert_eq!(f.tick(), WiEvent::None);
        }
        assert_eq!(wi_mode(), WiMode::StatCount);
        wi_end();
    }

    #[test]
    fn pressing_fire_skips_the_counting_then_moves_on_to_the_map() {
        let Some(mut f) = start(&wbs(), SINGLE) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.tick();
        f.press(BT_ATTACK as u8);
        f.tick();
        with(|s| {
            assert_eq!(s.sp_state, 10);
            assert_eq!(s.cnt_kills[0], 80, "everything jumps to its final value");
        });
        // held: no new press, so no advance
        f.tick();
        assert_eq!(wi_mode(), WiMode::StatCount);
        // release, press again: next screen
        f.press(0);
        f.tick();
        f.press(BT_USE as u8);
        f.tick();
        assert_eq!(wi_mode(), WiMode::ShowNextLoc);
        wi_end();
    }

    #[test]
    fn next_location_screen_times_out_into_no_state_then_world_done() {
        let Some(mut f) = start(&wbs(), SINGLE) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        with(|s| s.init_show_next_loc());
        assert_eq!(wi_mode(), WiMode::ShowNextLoc);
        for _ in 0..SHOWNEXTLOCDELAY * TICRATE - 1 {
            assert_eq!(f.tick(), WiEvent::None);
            assert_eq!(wi_mode(), WiMode::ShowNextLoc);
        }
        f.tick();
        assert_eq!(wi_mode(), WiMode::NoState);
        for _ in 0..9 {
            assert_eq!(f.tick(), WiEvent::None);
        }
        assert_eq!(f.tick(), WiEvent::WorldDone, "10 tics later");
        wi_end();
    }

    #[test]
    fn pointer_flashes_on_the_you_are_here_marker() {
        let Some(mut f) = start(&wbs(), SINGLE) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        with(|s| s.init_show_next_loc());
        // cnt = 140: pointer on while (cnt & 31) < 20
        f.tick(); // cnt 139: 139 & 31 = 11
        assert!(with(|s| s.snl_pointeron));
        for _ in 0..12 {
            f.tick();
        }
        // cnt 127: 127 & 31 = 31 -> off (and stays off until 115)
        assert!(!with(|s| s.snl_pointeron));

        // the drawn screen shows "entering" + the next level's name
        let mut v = VVideo::new();
        v.screens[1] = f.v.screens[1].clone();
        wi_drawer(&mut v);
        assert!(v.screens[0].iter().any(|&b| b != 0));
        wi_end();
    }

    #[test]
    fn number_drawing_matches_the_font_metrics() {
        let Some(f) = start(&wbs(), SINGLE) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut v = VVideo::new();
        with(|s| {
            let w = patch_width(&s.gfx.num[0]);
            // "as many digits as needed"
            assert_eq!(s.draw_num(&mut v, 100, 20, 0, -1), 100 - w);
            assert_eq!(s.draw_num(&mut v, 100, 20, 123, -1), 100 - 3 * w);
            // fixed width pads with zeros
            assert_eq!(s.draw_num(&mut v, 100, 20, 7, 3), 100 - 3 * w);
            // negative: minus sign 8 px further left
            assert_eq!(s.draw_num(&mut v, 100, 20, -5, -1), 100 - w - 8);
            // 1994 means "n/a": nothing drawn, x = 0
            let mut blank = VVideo::new();
            assert_eq!(s.draw_num(&mut blank, 100, 20, 1994, -1), 0);
            assert!(blank.screens[0].iter().all(|&b| b == 0));
            // negative percent isn't drawn at all
            s.draw_percent(&mut blank, 100, 20, -1);
            assert!(blank.screens[0].iter().all(|&b| b == 0));
        });
        drop(f);
        wi_end();
    }

    #[test]
    fn times_over_an_hour_draw_sucks() {
        let Some(f) = start(&wbs(), SINGLE) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let (mut a, mut b, mut c) = (VVideo::new(), VVideo::new(), VVideo::new());
        with(|s| {
            s.draw_time(&mut a, 200, 20, 61 * 59); // 59:59, still numbers
            s.draw_time(&mut b, 200, 20, 61 * 59 + 1); // sucks
            s.draw_time(&mut c, 200, 20, -1); // nothing
        });
        assert!(a.screens[0].iter().any(|&p| p != 0));
        assert!(b.screens[0].iter().any(|&p| p != 0));
        assert_ne!(a.screens[0], b.screens[0]);
        assert!(c.screens[0].iter().all(|&p| p == 0));
        drop(f);
        wi_end();
    }

    #[test]
    fn coop_stats_step_through_the_columns_and_use_frags_only_when_present() {
        let mut w = wbs();
        w.plyr[1] = WbPlayerStruct {
            in_game: true,
            skills: 4,
            sitems: 1,
            ssecret: 0,
            stime: 100 * TICRATE,
            frags: [3, 0, 0, 0],
            ..Default::default()
        };
        let Some(mut f) = start(&w, (GameMode::Registered, false, true, false)) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        // (start() marks only player 0 in game; add player 1)
        with(|s| s.playeringame[1] = true);
        // frags: player 1 fragged player 0 three times
        with(|s| s.init_netgame_stats());
        assert_eq!(with(|s| s.dofrags), 1, "there are frags to show");
        let mut ticks = 0;
        while with(|s| s.ng_state) != 10 {
            f.tick();
            ticks += 1;
            assert!(ticks < 4000);
        }
        with(|s| {
            assert_eq!((s.cnt_kills[0], s.cnt_kills[1]), (80, 40));
            assert_eq!((s.cnt_items[0], s.cnt_items[1]), (100, 20));
            assert_eq!((s.cnt_secret[0], s.cnt_secret[1]), (50, 0));
            // frag sum: others' frags on you minus your own (JDC hack)
            assert_eq!(s.cnt_frags[0], s.frag_sum(0));
        });
        // the table draws
        let mut v = VVideo::new();
        v.screens[1] = f.v.screens[1].clone();
        wi_drawer(&mut v);
        assert!(v.screens[0].iter().any(|&b| b != 0));
        wi_end();
    }

    #[test]
    fn deathmatch_matrix_counts_up_and_clamps() {
        let mut w = wbs();
        w.plyr[0].frags = [0, 120, 0, 0]; // 120 frags on player 1: clamps at 99
        w.plyr[1] = WbPlayerStruct {
            in_game: true,
            frags: [2, 0, 0, 0],
            ..Default::default()
        };
        let Some(mut f) = start(&w, (GameMode::Registered, false, true, true)) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        with(|s| {
            s.playeringame[1] = true;
            s.init_deathmatch_stats();
        });
        // The counting never reaches 120: each step is clamped to 99, so
        // `dm_frags != frags` stays true forever (the original sticks
        // there too until a key is pressed).
        for _ in 0..600 {
            f.tick();
        }
        with(|s| {
            assert_eq!(s.dm_state, 2);
            assert_eq!(s.dm_frags[0][1], 99, "clamped while counting");
            assert_eq!(s.dm_frags[1][0], 2);
            assert_eq!(s.dm_totals[0], 99);
        });
        // A keypress jumps straight to the real (unclamped) numbers.
        f.press(BT_USE as u8);
        f.tick();
        with(|s| {
            assert_eq!(s.dm_state, 4);
            assert_eq!(s.dm_frags[0][1], 120);
            assert_eq!(s.dm_totals[0], s.frag_sum(0));
        });
        let mut v = VVideo::new();
        v.screens[1] = f.v.screens[1].clone();
        wi_drawer(&mut v);
        assert!(v.screens[0].iter().any(|&b| b != 0));
        wi_end();
    }

    #[test]
    fn the_background_animation_ticks_but_is_never_drawn() {
        let Some(mut f) = start(&wbs(), SINGLE) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        with(|s| assert!(s.anims.iter().all(|a| a.ctr == -1)));
        for _ in 0..40 {
            f.tick();
        }
        with(|s| assert!(s.anims.iter().any(|a| a.ctr >= 0), "animations advanced"));
        // ...yet the picture is just the slammed background plus text:
        // drawing with animations on/off can't differ (they never draw)
        let mut a = VVideo::new();
        a.screens[1] = f.v.screens[1].clone();
        wi_drawer(&mut a);
        with(|s| s.anims.iter_mut().for_each(|a| a.ctr = -1));
        let mut b = VVideo::new();
        b.screens[1] = f.v.screens[1].clone();
        wi_drawer(&mut b);
        assert_eq!(a.screens[0], b.screens[0]);
        wi_end();
    }

    #[test]
    fn commercial_uses_interpic_and_cwilv_names() {
        let mut w = wbs();
        w.last = 0;
        w.next = 1;
        let mut wad = match wad() {
            Some(w) => w,
            None => {
                eprintln!("skipping: no doom.wad found");
                return;
            }
        };
        // The registered IWAD has no CWILV*/INTERPIC lumps: loading for
        // DOOM II must fail loudly (W_GetNumForName), like the original.
        let mut v = VVideo::new();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wi_start_for(
                &w,
                &[true],
                &mut wad,
                &mut v,
                (GameMode::Commercial, false, false, false),
            )
        }));
        assert!(r.is_err());
        wi_end();
    }
}
