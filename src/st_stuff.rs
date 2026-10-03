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
//	Status bar code.
//	Does the face/direction indicator animatin.
//	Does palette indicators as well (red pain/berserk, bright pickup)
//
//-----------------------------------------------------------------------------

//! Rust port of `st_stuff.h` / `st_stuff.c`. The status bar: ammo, health,
//! armor, weapons owned, keys, the face, the damage/pickup/radsuit
//! palette shifts — and the player-facing cheat codes
//! (`ST_Responder`).
//!
//! # State
//!
//! The original's file-scope statics (`plyr`, the widgets, `st_*`
//! flags, the face counters, the loaded patches, ...) are one
//! [`StState`] in a `thread_local!` behind free functions named like
//! the C ones ([`st_init`], [`st_start`], [`st_ticker`], [`st_drawer`],
//! [`st_responder`], [`st_stop`]) — the same convention as
//! `s_sound.rs`. `plyr` (`&players[consoleplayer]`) is passed in by the
//! callers instead of being stored.
//!
//! # Divergences
//!
//! * The widgets hold no pointers into the player (see `st_lib`'s
//!   docs); [`draw_widgets`] reads the current values, which is what
//!   the pointers gave at the same moment (`ST_drawWidgets` runs right
//!   after `ST_Ticker` in the same tic).
//! * `I_SetPalette` isn't called here: [`st_drawer`] and [`st_stop`]
//!   *return* the palette (768 bytes) the caller must apply, so this
//!   module needs no `IVideo`.
//! * `STlib_init` (loading `STTMINUS`) happens in [`st_init`] rather
//!   than in every `ST_Start`; the patch is the same.
//! * `ST_updateFaceWidget` reads `plyr->attacker->x/y`; if that mobj
//!   has since been freed (a dangling pointer in C) the "turn toward
//!   the attacker" branch is skipped.
//! * `idmypos`'s message is built in a `static char buf[]` in C; here
//!   the string is leaked (`&'static str`, what `Player::message`
//!   holds) — a debugging cheat, used a handful of times per session.
//! * `ST_unloadGraphics`/`ST_unloadData`/`ST_Stop`'s cache-tag
//!   changes have no equivalent (patches are owned `Rc`s).
//! * The chat state (`st_chatstate`/`st_chat`/`st_cursoron`, driven
//!   from `hu_stuff`) is kept for parity though only the netgame chat
//!   of a later phase will change it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::am_map::{AM_MSGENTERED, AM_MSGEXITED, AM_MSGHEADER};
use crate::d_englsh as msg;
use crate::d_event::{EvType, Event};
use crate::d_items::WEAPONINFO;
use crate::d_player::{Cheat, Player};
use crate::doomdef::{
    AmmoType, GameMode, PowerType, WeaponType, NUMAMMO, NUMCARDS, NUMWEAPONS, TICRATE,
};
use crate::doomstat;
use crate::g_game::g_defered_init_new;
use crate::m_cheat::CheatSeq;
use crate::m_random::m_random;
use crate::p_tick::Thinkers;
use crate::r_main::angle_from_delta;
use crate::s_sound::s_change_music;
use crate::sounds::MusicEnum;
use crate::st_lib::{
    st_updatebinicon, st_updatemulticon, PatchList, StBinIcon, StLib, StMultIcon, StNumber,
    StPercent, BG, FG, ST_HEIGHT, ST_WIDTH, ST_Y,
};
use crate::tables::{ANG180, ANG45};
use crate::v_video::{PatchData, VVideo};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

// Palette indices (`STARTREDPALS`..): the PLAYPAL has 14 palettes.
pub const STARTREDPALS: i32 = 1;
pub const STARTBONUSPALS: i32 = 9;
pub const NUMREDPALS: i32 = 8;
pub const NUMBONUSPALS: i32 = 4;
/// Radiation suit, green shift.
pub const RADIATIONPAL: i32 = 13;

/// N/A ammo (the "1994" sentinel `STlib_drawNum` doesn't draw).
const LARGEAMMO: i32 = 1994;

// Location of the status bar and its widgets (all in 320x200 pixels).
pub const ST_X: i32 = 0;
pub const ST_X2: i32 = 104;
pub const ST_FX: i32 = 143;
pub const ST_FY: i32 = 169;

// Number of face graphics and how they are laid out in `faces[]`.
pub const ST_NUMPAINFACES: usize = 5;
pub const ST_NUMSTRAIGHTFACES: usize = 3;
pub const ST_NUMTURNFACES: usize = 2;
pub const ST_NUMSPECIALFACES: usize = 3;
pub const ST_FACESTRIDE: usize = ST_NUMSTRAIGHTFACES + ST_NUMTURNFACES + ST_NUMSPECIALFACES;
pub const ST_NUMEXTRAFACES: usize = 2;
pub const ST_NUMFACES: usize = ST_FACESTRIDE * ST_NUMPAINFACES + ST_NUMEXTRAFACES;
pub const ST_TURNOFFSET: usize = ST_NUMSTRAIGHTFACES;
pub const ST_OUCHOFFSET: usize = ST_TURNOFFSET + ST_NUMTURNFACES;
pub const ST_EVILGRINOFFSET: usize = ST_OUCHOFFSET + 1;
pub const ST_RAMPAGEOFFSET: usize = ST_EVILGRINOFFSET + 1;
pub const ST_GODFACE: usize = ST_NUMPAINFACES * ST_FACESTRIDE;
pub const ST_DEADFACE: usize = ST_GODFACE + 1;

pub const ST_FACESX: i32 = 143;
pub const ST_FACESY: i32 = 168;

pub const ST_EVILGRINCOUNT: i32 = 2 * TICRATE;
pub const ST_STRAIGHTFACECOUNT: i32 = TICRATE / 2;
pub const ST_TURNCOUNT: i32 = TICRATE;
pub const ST_OUCHCOUNT: i32 = TICRATE;
pub const ST_RAMPAGEDELAY: i32 = 2 * TICRATE;
pub const ST_MUCHPAIN: i32 = 20;

// Widget positions.
pub const ST_AMMOWIDTH: i32 = 3;
pub const ST_AMMOX: i32 = 44;
pub const ST_AMMOY: i32 = 171;
pub const ST_HEALTHWIDTH: i32 = 3;
pub const ST_HEALTHX: i32 = 90;
pub const ST_HEALTHY: i32 = 171;
pub const ST_ARMSX: i32 = 111;
pub const ST_ARMSY: i32 = 172;
pub const ST_ARMSBGX: i32 = 104;
pub const ST_ARMSBGY: i32 = 168;
pub const ST_ARMSXSPACE: i32 = 12;
pub const ST_ARMSYSPACE: i32 = 10;
pub const ST_FRAGSX: i32 = 138;
pub const ST_FRAGSY: i32 = 171;
pub const ST_FRAGSWIDTH: i32 = 2;
pub const ST_ARMORWIDTH: i32 = 3;
pub const ST_ARMORX: i32 = 221;
pub const ST_ARMORY: i32 = 171;
pub const ST_KEY_X: i32 = 239;
pub const ST_KEY_Y: [i32; 3] = [171, 181, 191];
pub const ST_AMMOSMALL_WIDTH: i32 = 3;
pub const ST_AMMOSMALL_X: i32 = 288;
pub const ST_AMMOSMALL_Y: [i32; 4] = [173, 179, 191, 185];
pub const ST_MAXAMMO_X: i32 = 314;

/// (`st_stateenum_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StStateEnum {
    AutomapState,
    FirstPersonState,
}

/// (`st_chatstateenum_t`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StChatStateEnum {
    StartChatState,
    WaitDestState,
    GetChatCharState,
}

/// The loaded graphics (`sbar`, `tallnum`, ... `faces`).
struct StGfx {
    sbar: PatchData,
    tallnum: PatchList,
    tallpercent: PatchData,
    shortnum: PatchList,
    keys: PatchList,
    faces: PatchList,
    faceback: PatchData,
    armsbg: PatchData,
    /// `arms[i]`: `[gray STGNUM(i+2), yellow shortnum(i+2)]`.
    arms: Vec<PatchList>,
}

/// The cheat sequences (`cheat_*` statics).
struct Cheats {
    mus: CheatSeq,
    god: CheatSeq,
    ammo: CheatSeq,
    ammonokey: CheatSeq,
    noclip: CheatSeq,
    commercial_noclip: CheatSeq,
    powerup: Vec<CheatSeq>,
    choppers: CheatSeq,
    clev: CheatSeq,
    mypos: CheatSeq,
}

impl Cheats {
    fn new() -> Self {
        let beh = [0xb2u8, 0x26, 0x62, 0xa6, 0x32, 0xf6, 0x36, 0x26];
        let with = |last: Option<u8>| {
            let mut v = beh.to_vec();
            if let Some(l) = last {
                v.push(l);
            }
            v.push(0xff);
            CheatSeq::new(&v)
        };
        Cheats {
            mus: CheatSeq::new(&[0xb2, 0x26, 0xb6, 0xae, 0xea, 1, 0, 0, 0xff]),
            god: CheatSeq::new(&[0xb2, 0x26, 0x26, 0xaa, 0x26, 0xff]),
            ammo: CheatSeq::new(&[0xb2, 0x26, 0xf2, 0x66, 0xa2, 0xff]),
            ammonokey: CheatSeq::new(&[0xb2, 0x26, 0x66, 0xa2, 0xff]),
            noclip: CheatSeq::new(&[
                0xb2, 0x26, 0xea, 0x2a, 0xb2, 0xea, 0x2a, 0xf6, 0x2a, 0x26, 0xff,
            ]),
            commercial_noclip: CheatSeq::new(&[0xb2, 0x26, 0xe2, 0x36, 0xb2, 0x2a, 0xff]),
            // beholdv, beholds, beholdi, beholdr, beholda, beholdl, behold
            powerup: vec![
                with(Some(0x6e)),
                with(Some(0xea)),
                with(Some(0xb2)),
                with(Some(0x6a)),
                with(Some(0xa2)),
                with(Some(0x36)),
                with(None),
            ],
            choppers: CheatSeq::new(&[
                0xb2, 0x26, 0xe2, 0x32, 0xf6, 0x2a, 0x2a, 0xa6, 0x6a, 0xea, 0xff,
            ]),
            clev: CheatSeq::new(&[0xb2, 0x26, 0xe2, 0x36, 0xa6, 0x6e, 1, 0, 0, 0xff]),
            mypos: CheatSeq::new(&[0xb2, 0x26, 0xb6, 0xba, 0x2a, 0xf6, 0xea, 0xff]),
        }
    }
}

/// The widgets (`w_*`).
struct Widgets {
    ready: StNumber,
    frags: StNumber,
    health: StPercent,
    armsbg: StBinIcon,
    arms: Vec<StMultIcon>,
    faces: StMultIcon,
    keyboxes: Vec<StMultIcon>,
    armor: StPercent,
    ammo: Vec<StNumber>,
    maxammo: Vec<StNumber>,
}

/// Everything `st_stuff.c` keeps in file scope.
struct StState {
    lib: StLib,
    gfx: StGfx,
    /// The whole PLAYPAL (14 palettes of 768 bytes; `lu_palette`).
    playpal: Vec<u8>,
    widgets: Widgets,
    cheats: Cheats,

    firsttime: bool,
    clock: u32,
    msgcounter: i32,
    chatstate: StChatStateEnum,
    gamestate: StStateEnum,
    statusbaron: bool,
    chat: bool,
    oldchat: bool,
    cursoron: bool,
    notdeathmatch: bool,
    armson: bool,
    fragson: bool,
    fragscount: i32,
    oldhealth: i32,
    oldweaponsowned: [bool; NUMWEAPONS],
    facecount: i32,
    faceindex: usize,
    keyboxes: [i32; 3],
    randomnumber: i32,
    palette: i32,
    stopped: bool,

    // Function-local statics.
    /// `ST_calcPainOffset`'s `lastcalc`/`oldhealth`.
    pain_lastcalc: usize,
    pain_oldhealth: i32,
    /// `ST_updateFaceWidget`'s `lastattackdown`/`priority`.
    face_lastattackdown: i32,
    face_priority: i32,
}

thread_local! {
    static ST: RefCell<Option<StState>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut StState) -> R) -> R {
    ST.with(|s| f(s.borrow_mut().as_mut().expect("ST_Init has not run")))
}

/// Whether [`st_init`] has run on this thread.
pub fn st_is_initialized() -> bool {
    ST.with(|s| s.borrow().is_some())
}

/// Drops the status bar state (tests, and shutdown).
pub fn st_shutdown() {
    ST.with(|s| *s.borrow_mut() = None);
}

fn cache(wad: &mut WadFiles, name: &str) -> PatchData {
    Rc::from(wad.cache_lump_name(name, PurgeTag::Static))
}

/// Port of `ST_loadGraphics`.
fn load_graphics(wad: &mut WadFiles, consoleplayer: i32) -> StGfx {
    let mut tallnum = Vec::new();
    let mut shortnum = Vec::new();
    for i in 0..10 {
        tallnum.push(cache(wad, &format!("STTNUM{i}")));
        shortnum.push(cache(wad, &format!("STYSNUM{i}")));
    }
    let tallpercent = cache(wad, "STTPRCNT");

    // key cards
    let keys: Vec<PatchData> = (0..NUMCARDS)
        .map(|i| cache(wad, &format!("STKEYS{i}")))
        .collect();

    // arms background
    let armsbg = cache(wad, "STARMS");

    // arms ownership widgets
    let arms: Vec<PatchList> = (0..6)
        .map(|i| {
            // gray
            let gray = cache(wad, &format!("STGNUM{}", i + 2));
            // yellow
            let yellow = shortnum[i + 2].clone();
            let list: Vec<PatchData> = vec![gray, yellow];
            list.into()
        })
        .collect();

    // face backgrounds for different color players
    let faceback = cache(wad, &format!("STFB{consoleplayer}"));

    // status bar background bits
    let sbar = cache(wad, "STBAR");

    // face states
    let mut faces = Vec::new();
    for i in 0..ST_NUMPAINFACES {
        for j in 0..ST_NUMSTRAIGHTFACES {
            faces.push(cache(wad, &format!("STFST{i}{j}")));
        }
        faces.push(cache(wad, &format!("STFTR{i}0"))); // turn right
        faces.push(cache(wad, &format!("STFTL{i}0"))); // turn left
        faces.push(cache(wad, &format!("STFOUCH{i}"))); // ouch!
        faces.push(cache(wad, &format!("STFEVL{i}"))); // evil grin ;)
        faces.push(cache(wad, &format!("STFKILL{i}"))); // pissed off
    }
    faces.push(cache(wad, "STFGOD0"));
    faces.push(cache(wad, "STFDEAD0"));
    assert_eq!(faces.len(), ST_NUMFACES);

    StGfx {
        sbar,
        tallnum: tallnum.into(),
        tallpercent,
        shortnum: shortnum.into(),
        keys: keys.into(),
        faces: faces.into(),
        faceback,
        armsbg,
        arms,
    }
}

/// Port of `ST_createWidgets`.
fn create_widgets(g: &StGfx) -> Widgets {
    let small_num = |x: i32, y: i32| StNumber::new(x, y, g.shortnum.clone(), ST_AMMOSMALL_WIDTH);
    Widgets {
        // ready weapon ammo
        ready: StNumber::new(ST_AMMOX, ST_AMMOY, g.tallnum.clone(), ST_AMMOWIDTH),
        // the last weapon type is not needed here (`w_ready.data`)

        // health percentage
        health: StPercent::new(
            ST_HEALTHX,
            ST_HEALTHY,
            g.tallnum.clone(),
            g.tallpercent.clone(),
        ),
        // arms background
        armsbg: StBinIcon::new(ST_ARMSBGX, ST_ARMSBGY, g.armsbg.clone()),
        // weapons owned
        arms: (0..6)
            .map(|i| {
                StMultIcon::new(
                    ST_ARMSX + (i % 3) * ST_ARMSXSPACE,
                    ST_ARMSY + (i / 3) * ST_ARMSYSPACE,
                    g.arms[i as usize].clone(),
                )
            })
            .collect(),
        // frags sum
        frags: StNumber::new(ST_FRAGSX, ST_FRAGSY, g.tallnum.clone(), ST_FRAGSWIDTH),
        // faces
        faces: StMultIcon::new(ST_FACESX, ST_FACESY, g.faces.clone()),
        // armor percentage - should be colored later
        armor: StPercent::new(
            ST_ARMORX,
            ST_ARMORY,
            g.tallnum.clone(),
            g.tallpercent.clone(),
        ),
        // keyboxes 0-2
        keyboxes: (0..3)
            .map(|i| StMultIcon::new(ST_KEY_X, ST_KEY_Y[i], g.keys.clone()))
            .collect(),
        // ammo count (all four kinds)
        ammo: (0..4)
            .map(|i| small_num(ST_AMMOSMALL_X, ST_AMMOSMALL_Y[i]))
            .collect(),
        // max ammo count (all four kinds)
        maxammo: (0..4)
            .map(|i| small_num(ST_MAXAMMO_X, ST_AMMOSMALL_Y[i]))
            .collect(),
    }
}

/// Port of `ST_Init`: loads the graphics and allocates `screens[4]` —
/// the status bar's pristine background, `ST_WIDTH*ST_HEIGHT` bytes,
/// rows relative to `ST_Y`.
pub fn st_init(wad: &mut WadFiles, v: &mut VVideo) {
    let lu_palette = wad.get_num_for_name("PLAYPAL");
    let playpal = wad.cache_lump_num(lu_palette, PurgeTag::Cache).to_vec();
    let consoleplayer = doomstat::state().consoleplayer;
    let gfx = load_graphics(wad, consoleplayer);
    let lib = StLib::init(wad);
    v.screens[BG] = vec![0; (ST_WIDTH * ST_HEIGHT) as usize];

    let widgets = create_widgets(&gfx);
    ST.with(|s| {
        *s.borrow_mut() = Some(StState {
            lib,
            gfx,
            playpal,
            widgets,
            cheats: Cheats::new(),
            firsttime: true,
            clock: 0,
            msgcounter: 0,
            chatstate: StChatStateEnum::StartChatState,
            gamestate: StStateEnum::FirstPersonState,
            statusbaron: true,
            chat: false,
            oldchat: false,
            cursoron: false,
            notdeathmatch: true,
            armson: true,
            fragson: false,
            fragscount: 0,
            oldhealth: -1,
            oldweaponsowned: [false; NUMWEAPONS],
            facecount: 0,
            faceindex: 0,
            keyboxes: [-1; 3],
            randomnumber: 0,
            palette: 0,
            stopped: true,
            pain_lastcalc: 0,
            pain_oldhealth: -1,
            face_lastattackdown: -1,
            face_priority: 0,
        });
    });
}

/// Port of `ST_Start` (with `ST_initData`/`ST_createWidgets`): wakes up
/// the status bar for a level. `plyr` is `&players[consoleplayer]`.
pub fn st_start(plyr: &Player) -> Option<Vec<u8>> {
    let stop = st_stop_if_running();
    with(|s| {
        // ST_initData
        s.firsttime = true;
        s.clock = 0;
        s.chatstate = StChatStateEnum::StartChatState;
        s.gamestate = StStateEnum::FirstPersonState;
        s.statusbaron = true;
        s.oldchat = false;
        s.chat = false;
        s.cursoron = false;
        s.faceindex = 0;
        s.palette = -1;
        s.oldhealth = -1;
        s.oldweaponsowned = plyr.weaponowned[..NUMWEAPONS].try_into().unwrap();
        s.keyboxes = [-1; 3];
        // ST_createWidgets
        s.widgets = create_widgets(&s.gfx);
        s.stopped = false;
    });
    stop
}

fn st_stop_if_running() -> Option<Vec<u8>> {
    if with(|s| !s.stopped) {
        st_stop()
    } else {
        None
    }
}

/// Port of `ST_Stop`. Returns the default palette to `I_SetPalette`
/// (`None` if it was already stopped).
pub fn st_stop() -> Option<Vec<u8>> {
    with(|s| {
        if s.stopped {
            return None;
        }
        s.stopped = true;
        Some(s.playpal[..768].to_vec())
    })
}

/// Port of `ST_refreshBackground`.
fn refresh_background(s: &StState, v: &mut VVideo) {
    if s.statusbaron {
        v.v_draw_patch(ST_X, 0, BG, &s.gfx.sbar);

        if doomstat::state().netgame {
            v.v_draw_patch(ST_FX, 0, BG, &s.gfx.faceback);
        }

        v.v_copy_rect(ST_X, 0, BG, ST_WIDTH, ST_HEIGHT, ST_X, ST_Y, FG);
    }
}

/// `power` index → [`PowerType`] (the cheats index `powers[]` by `i`).
fn power_from_index(i: usize) -> PowerType {
    match i {
        0 => PowerType::PwInvulnerability,
        1 => PowerType::PwStrength,
        2 => PowerType::PwInvisibility,
        3 => PowerType::PwIronfeet,
        4 => PowerType::PwAllmap,
        _ => PowerType::PwInfrared,
    }
}

/// Port of `ST_Responder`. Respond to keyboard input events, intercept
/// cheats (the original's own comment, preserved). `plyr` is
/// `&players[consoleplayer]`.
pub fn st_responder(
    ev: &Event,
    plyr: &mut Player,
    thinkers: &mut Thinkers,
    wad: &mut WadFiles,
) -> bool {
    // Filter automap on/off.
    if ev.event_type == EvType::KeyUp && (ev.data1 & 0xffff0000u32 as i32) == AM_MSGHEADER {
        match ev.data1 {
            AM_MSGENTERED => {
                with(|s| {
                    s.gamestate = StStateEnum::AutomapState;
                    s.firsttime = true;
                });
            }
            AM_MSGEXITED => {
                with(|s| s.gamestate = StStateEnum::FirstPersonState);
            }
            _ => {}
        }
        return false;
    }

    // if a user keypress...
    if ev.event_type != EvType::KeyDown {
        return false;
    }
    let key = ev.data1 as u8;
    let gamemode = doomstat::state().gamemode;

    // b. - enabled for more debug fun.
    // if (gamemode != commercial)
    if !doomstat::state().netgame {
        // 'dqd' cheat for toggleable god mode
        if with(|s| s.cheats.god.check(key)) {
            plyr.cheats ^= Cheat::GodMode as i32;
            if plyr.cheats & Cheat::GodMode as i32 != 0 {
                if let Some(mo) = thinkers.mobj_mut(plyr.mo) {
                    mo.health = 100;
                }
                plyr.health = 100;
                plyr.message = Some(msg::STSTR_DQDON);
            } else {
                plyr.message = Some(msg::STSTR_DQDOFF);
            }
        }
        // 'fa' cheat for killer fucking arsenal
        else if with(|s| s.cheats.ammonokey.check(key)) {
            plyr.armorpoints = 200;
            plyr.armortype = 2;
            for i in 0..NUMWEAPONS {
                plyr.weaponowned[i] = true;
            }
            for i in 0..NUMAMMO {
                plyr.ammo[i] = plyr.maxammo[i];
            }
            plyr.message = Some(msg::STSTR_FAADDED);
        }
        // 'kfa' cheat for key full ammo
        else if with(|s| s.cheats.ammo.check(key)) {
            plyr.armorpoints = 200;
            plyr.armortype = 2;
            for i in 0..NUMWEAPONS {
                plyr.weaponowned[i] = true;
            }
            for i in 0..NUMAMMO {
                plyr.ammo[i] = plyr.maxammo[i];
            }
            for i in 0..NUMCARDS {
                plyr.cards[i] = true;
            }
            plyr.message = Some(msg::STSTR_KFAADDED);
        }
        // 'mus' cheat for changing music
        else if with(|s| s.cheats.mus.check(key)) {
            plyr.message = Some(msg::STSTR_MUS);
            let buf = with(|s| s.cheats.mus.get_param());
            if gamemode == GameMode::Commercial {
                let n = (buf[0] as i32 - b'0' as i32) * 10 + buf[1] as i32 - b'0' as i32;
                let musnum = MusicEnum::MusRunnin as i32 + n - 1;
                if n > 35 {
                    plyr.message = Some(msg::STSTR_NOMUS);
                } else {
                    s_change_music(wad, musnum, true);
                }
            } else {
                let n = (buf[0] as i32 - b'1' as i32) * 9 + (buf[1] as i32 - b'1' as i32);
                let musnum = MusicEnum::MusE1m1 as i32 + n;
                if n > 31 {
                    plyr.message = Some(msg::STSTR_NOMUS);
                } else {
                    s_change_music(wad, musnum, true);
                }
            }
        }
        // Simplified, accepting both "noclip" and "idspispopd".
        // no clipping mode cheat
        else if with(|s| s.cheats.noclip.check(key) || s.cheats.commercial_noclip.check(key)) {
            plyr.cheats ^= Cheat::NoClip as i32;
            plyr.message = Some(if plyr.cheats & Cheat::NoClip as i32 != 0 {
                msg::STSTR_NCON
            } else {
                msg::STSTR_NCOFF
            });
        }

        // 'behold?' power-up cheats
        for i in 0..6 {
            if with(|s| s.cheats.powerup[i].check(key)) {
                if plyr.powers[i] == 0 {
                    crate::p_inter::p_give_power(thinkers, plyr, power_from_index(i));
                } else if i != PowerType::PwStrength as usize {
                    plyr.powers[i] = 1;
                } else {
                    plyr.powers[i] = 0;
                }
                plyr.message = Some(msg::STSTR_BEHOLDX);
            }
        }

        // 'behold' power-up menu
        if with(|s| s.cheats.powerup[6].check(key)) {
            plyr.message = Some(msg::STSTR_BEHOLD);
        }
        // 'choppers' invulnerability & chainsaw
        else if with(|s| s.cheats.choppers.check(key)) {
            plyr.weaponowned[WeaponType::WpChainsaw as usize] = true;
            plyr.powers[PowerType::PwInvulnerability as usize] = 1;
            plyr.message = Some(msg::STSTR_CHOPPERS);
        }
        // 'mypos' for player position
        else if with(|s| s.cheats.mypos.check(key)) {
            if let Some(mo) = thinkers.mobj(plyr.mo) {
                let text = format!(
                    "ang=0x{:x};x,y=(0x{:x},0x{:x})",
                    mo.angle, mo.x as u32, mo.y as u32
                );
                // See the module docs on why this is leaked.
                plyr.message = Some(Box::leak(text.into_boxed_str()));
            }
        }
    }

    // 'clev' change-level cheat
    if with(|s| s.cheats.clev.check(key)) {
        let buf = with(|s| s.cheats.clev.get_param());

        let (epsd, map) = if gamemode == GameMode::Commercial {
            (
                0,
                (buf[0] as i32 - b'0' as i32) * 10 + buf[1] as i32 - b'0' as i32,
            )
        } else {
            (buf[0] as i32 - b'0' as i32, buf[1] as i32 - b'0' as i32)
        };

        // Catch invalid maps.
        if epsd < 1 {
            return false;
        }
        if map < 1 {
            return false;
        }
        // Ohmygod - this is not going to work.
        if (gamemode == GameMode::Retail && (epsd > 4 || map > 9))
            || (gamemode == GameMode::Registered && (epsd > 3 || map > 9))
            || (gamemode == GameMode::Shareware && (epsd > 1 || map > 9))
            || (gamemode == GameMode::Commercial && (epsd > 1 || map > 34))
        {
            return false;
        }

        // So be it.
        plyr.message = Some(msg::STSTR_CLEV);
        g_defered_init_new(doomstat::state().gameskill, epsd, map);
    }
    false
}

/// Port of `ST_calcPainOffset`.
fn calc_pain_offset(s: &mut StState, plyr: &Player) -> usize {
    let health = plyr.health.min(100);

    if health != s.pain_oldhealth {
        s.pain_lastcalc =
            ST_FACESTRIDE * (((100 - health) * ST_NUMPAINFACES as i32) / 101) as usize;
        s.pain_oldhealth = health;
    }
    s.pain_lastcalc
}

/// Port of `ST_updateFaceWidget`. This is a not-very-pretty routine
/// which handles the face states and their timing. The precedence of
/// expressions is: dead > evil grin > turned head > straight ahead
/// (the original's own comment, preserved).
fn update_face_widget(s: &mut StState, plyr: &Player, thinkers: &Thinkers) {
    if s.face_priority < 10 {
        // dead
        if plyr.health == 0 {
            s.face_priority = 9;
            s.faceindex = ST_DEADFACE;
            s.facecount = 1;
        }
    }

    if s.face_priority < 9 && plyr.bonuscount != 0 {
        // picking up a bonus
        let mut doevilgrin = false;
        for i in 0..NUMWEAPONS {
            if s.oldweaponsowned[i] != plyr.weaponowned[i] {
                doevilgrin = true;
                s.oldweaponsowned[i] = plyr.weaponowned[i];
            }
        }
        if doevilgrin {
            // evil grin if just picked up weapon
            s.face_priority = 8;
            s.facecount = ST_EVILGRINCOUNT;
            s.faceindex = calc_pain_offset(s, plyr) + ST_EVILGRINOFFSET;
        }
    }

    if s.face_priority < 8 {
        let attacker = plyr
            .attacker
            .filter(|&a| a != plyr.mo)
            .and_then(|a| thinkers.mobj(a));
        if let (true, Some(attacker), Some(mo)) =
            (plyr.damagecount != 0, attacker, thinkers.mobj(plyr.mo))
        {
            // being attacked
            s.face_priority = 7;

            if plyr.health - s.oldhealth > ST_MUCHPAIN {
                s.facecount = ST_TURNCOUNT;
                s.faceindex = calc_pain_offset(s, plyr) + ST_OUCHOFFSET;
            } else {
                let badguyangle =
                    angle_from_delta(attacker.x.wrapping_sub(mo.x), attacker.y.wrapping_sub(mo.y));

                let (diffang, i) = if badguyangle > mo.angle {
                    // whether right or left
                    let d = badguyangle - mo.angle;
                    (d, d > ANG180)
                } else {
                    // whether left or right
                    let d = mo.angle - badguyangle;
                    (d, d <= ANG180)
                }; // confusing, aint it?

                s.facecount = ST_TURNCOUNT;
                s.faceindex = calc_pain_offset(s, plyr);

                if diffang < ANG45 {
                    // head-on
                    s.faceindex += ST_RAMPAGEOFFSET;
                } else if i {
                    // turn face right
                    s.faceindex += ST_TURNOFFSET;
                } else {
                    // turn face left
                    s.faceindex += ST_TURNOFFSET + 1;
                }
            }
        }
    }

    if s.face_priority < 7 {
        // getting hurt because of your own damn stupidity
        if plyr.damagecount != 0 {
            if plyr.health - s.oldhealth > ST_MUCHPAIN {
                s.face_priority = 7;
                s.facecount = ST_TURNCOUNT;
                s.faceindex = calc_pain_offset(s, plyr) + ST_OUCHOFFSET;
            } else {
                s.face_priority = 6;
                s.facecount = ST_TURNCOUNT;
                s.faceindex = calc_pain_offset(s, plyr) + ST_RAMPAGEOFFSET;
            }
        }
    }

    if s.face_priority < 6 {
        // rapid firing
        if plyr.attackdown {
            if s.face_lastattackdown == -1 {
                s.face_lastattackdown = ST_RAMPAGEDELAY;
            } else {
                s.face_lastattackdown -= 1;
                if s.face_lastattackdown == 0 {
                    s.face_priority = 5;
                    s.faceindex = calc_pain_offset(s, plyr) + ST_RAMPAGEOFFSET;
                    s.facecount = 1;
                    s.face_lastattackdown = 1;
                }
            }
        } else {
            s.face_lastattackdown = -1;
        }
    }

    if s.face_priority < 5 {
        // invulnerability
        if plyr.cheats & Cheat::GodMode as i32 != 0
            || plyr.powers[PowerType::PwInvulnerability as usize] != 0
        {
            s.face_priority = 4;

            s.faceindex = ST_GODFACE;
            s.facecount = 1;
        }
    }

    // look left or look right if the facecount has timed out
    if s.facecount == 0 {
        s.faceindex = calc_pain_offset(s, plyr) + (s.randomnumber % 3) as usize;
        s.facecount = ST_STRAIGHTFACECOUNT;
        s.face_priority = 0;
    }

    s.facecount -= 1;
}

/// Port of `ST_updateWidgets`.
fn update_widgets(s: &mut StState, plyr: &Player, thinkers: &Thinkers) {
    // (`w_ready.num`/`w_ready.data` are derived at draw time.)

    // update keycard multiple widgets
    for i in 0..3 {
        s.keyboxes[i] = if plyr.cards[i] { i as i32 } else { -1 };

        if plyr.cards[i + 3] {
            s.keyboxes[i] = i as i32 + 3;
        }
    }

    // refresh everything if this is him coming back to life
    update_face_widget(s, plyr, thinkers);

    // used by the w_armsbg widget
    let deathmatch = doomstat::state().deathmatch;
    s.notdeathmatch = !deathmatch;

    // used by w_arms[] widgets
    s.armson = s.statusbaron && !deathmatch;

    // used by w_frags widget
    s.fragson = deathmatch && s.statusbaron;
    s.fragscount = 0;

    let consoleplayer = doomstat::state().consoleplayer as usize;
    for (i, frags) in plyr.frags.iter().enumerate() {
        if i != consoleplayer {
            s.fragscount += frags;
        } else {
            s.fragscount -= frags;
        }
    }

    // get rid of chat window if up because of message
    s.msgcounter = s.msgcounter.wrapping_sub(1);
    if s.msgcounter == 0 {
        s.chat = s.oldchat;
    }
}

/// Port of `ST_Ticker`.
pub fn st_ticker(plyr: &Player, thinkers: &Thinkers) {
    with(|s| {
        s.clock += 1;
        s.randomnumber = m_random();
        update_widgets(s, plyr, thinkers);
        s.oldhealth = plyr.health;
    });
}

/// Port of `ST_doPaletteStuff`. Returns the palette to `I_SetPalette`
/// when it changed.
fn do_palette_stuff(s: &mut StState, plyr: &Player) -> Option<Vec<u8>> {
    let mut cnt = plyr.damagecount;

    if plyr.powers[PowerType::PwStrength as usize] != 0 {
        // slowly fade the berzerk out
        let bzc = 12 - (plyr.powers[PowerType::PwStrength as usize] >> 6);

        if bzc > cnt {
            cnt = bzc;
        }
    }

    let palette = if cnt != 0 {
        let mut palette = (cnt + 7) >> 3;

        if palette >= NUMREDPALS {
            palette = NUMREDPALS - 1;
        }

        palette + STARTREDPALS
    } else if plyr.bonuscount != 0 {
        let mut palette = (plyr.bonuscount + 7) >> 3;

        if palette >= NUMBONUSPALS {
            palette = NUMBONUSPALS - 1;
        }

        palette + STARTBONUSPALS
    } else if plyr.powers[PowerType::PwIronfeet as usize] > 4 * 32
        || plyr.powers[PowerType::PwIronfeet as usize] & 8 != 0
    {
        RADIATIONPAL
    } else {
        0
    };

    if palette != s.palette {
        s.palette = palette;
        let ofs = palette as usize * 768;
        Some(s.playpal[ofs..ofs + 768].to_vec())
    } else {
        None
    }
}

/// Port of `ST_drawWidgets`. `refresh` redraws everything regardless
/// of change.
fn draw_widgets(s: &mut StState, refresh: bool, plyr: &Player, v: &mut VVideo) {
    // used by w_arms[] widgets
    s.armson = s.statusbaron && !doomstat::state().deathmatch;

    // used by w_frags widget
    s.fragson = doomstat::state().deathmatch && s.statusbaron;

    let on = s.statusbaron;
    let ready_ammo = WEAPONINFO[plyr.readyweapon as usize].ammo;
    let ready_num = if ready_ammo == AmmoType::AmNoammo {
        LARGEAMMO // means "n/a"
    } else {
        plyr.ammo[ready_ammo as usize]
    };

    let lib = &s.lib;
    let w = &mut s.widgets;
    lib.update_num(&mut w.ready, refresh, ready_num, on, v);

    for i in 0..4 {
        lib.update_num(&mut w.ammo[i], refresh, plyr.ammo[i], on, v);
        lib.update_num(&mut w.maxammo[i], refresh, plyr.maxammo[i], on, v);
    }

    lib.update_percent(&mut w.health, refresh, plyr.health, on, v);
    lib.update_percent(&mut w.armor, refresh, plyr.armorpoints, on, v);

    st_updatebinicon(&mut w.armsbg, refresh, s.notdeathmatch, on, v);

    for i in 0..6 {
        st_updatemulticon(
            &mut w.arms[i],
            refresh,
            plyr.weaponowned[i + 1] as i32,
            s.armson,
            v,
        );
    }

    st_updatemulticon(&mut w.faces, refresh, s.faceindex as i32, on, v);

    for i in 0..3 {
        st_updatemulticon(&mut w.keyboxes[i], refresh, s.keyboxes[i], on, v);
    }

    lib.update_num(&mut w.frags, refresh, s.fragscount, s.fragson, v);
}

/// Port of `ST_Drawer`. Returns the palette to `I_SetPalette` when the
/// damage/pickup/radsuit shift changed.
pub fn st_drawer(
    fullscreen: bool,
    refresh: bool,
    plyr: &Player,
    v: &mut VVideo,
) -> Option<Vec<u8>> {
    with(|s| {
        s.statusbaron = !fullscreen || doomstat::state().automapactive;
        s.firsttime = s.firsttime || refresh;

        // Do red-/gold-shifts from damage/items
        let palette = do_palette_stuff(s, plyr);

        // If just after ST_Start(), refresh all
        if s.firsttime {
            // ST_doRefresh
            s.firsttime = false;

            // draw status bar background to off-screen buff
            refresh_background(s, v);

            // and refresh all widgets
            draw_widgets(s, true, plyr, v);
        } else {
            // Otherwise, update as little as possible (ST_diffDraw)
            draw_widgets(s, false, plyr, v);
        }
        palette
    })
}

/// The current face index (`st_faceindex`), for tests and diagnostics.
pub fn st_faceindex() -> usize {
    with(|s| s.faceindex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::MobjType;
    use crate::p_tick::{ThinkFn, ThinkerData};
    use crate::r_defs::Mobj;

    /// A ready status bar around a real WAD, plus a player whose mobj
    /// sits at the origin facing east.
    struct Fixture {
        v: VVideo,
        thinkers: Thinkers,
        plyr: Player,
    }

    fn add_mobj(t: &mut Thinkers, x: i32, y: i32) -> crate::p_tick::ThinkerId {
        let mut m = Mobj::blank(MobjType::MtPlayer);
        m.x = x;
        m.y = y;
        t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(m))
    }

    fn fixture() -> Option<Fixture> {
        let path = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut wad = WadFiles::new();
        wad.init_file(path);
        let mut v = VVideo::new();
        st_shutdown();
        st_init(&mut wad, &mut v);
        let mut thinkers = Thinkers::new();
        let mo = add_mobj(&mut thinkers, 0, 0);
        let plyr = Player::for_test(mo);
        st_start(&plyr);
        Some(Fixture { v, thinkers, plyr })
    }

    #[test]
    fn face_table_layout_matches_the_original() {
        assert_eq!(ST_FACESTRIDE, 8);
        assert_eq!(ST_NUMFACES, 42);
        assert_eq!(ST_GODFACE, 40);
        assert_eq!(ST_DEADFACE, 41);
        assert_eq!(ST_RAMPAGEOFFSET, 7);
    }

    #[test]
    fn init_allocates_the_status_bar_background_screen() {
        let Some(f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        assert_eq!(f.v.screens[BG].len(), (ST_WIDTH * ST_HEIGHT) as usize);
    }

    #[test]
    fn pain_offset_follows_health() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut p = f.plyr.clone();
        with(|s| {
            p.health = 100;
            assert_eq!(calc_pain_offset(s, &p), 0);
            p.health = 200; // clamped to 100
            assert_eq!(calc_pain_offset(s, &p), 0);
            p.health = 50;
            assert_eq!(calc_pain_offset(s, &p), ST_FACESTRIDE * 2);
            p.health = 1;
            assert_eq!(calc_pain_offset(s, &p), ST_FACESTRIDE * 4);
            p.health = 0;
            assert_eq!(calc_pain_offset(s, &p), ST_FACESTRIDE * 4);
        });
        f.plyr.health = 100;
    }

    /// One tick of the face logic, with `M_Random` pinned to 0 by
    /// setting the state's random number directly.
    fn face_after(f: &mut Fixture, tune: impl FnOnce(&mut Player)) -> usize {
        with(|s| {
            s.face_priority = 0;
            s.facecount = 0;
            s.randomnumber = 0;
            s.oldhealth = 100;
            s.face_lastattackdown = -1;
            s.pain_oldhealth = -1;
        });
        let mut p = f.plyr.clone();
        p.health = 100;
        tune(&mut p);
        with(|s| update_face_widget(s, &p, &f.thinkers));
        st_faceindex()
    }

    #[test]
    fn face_priorities() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        // healthy and idle: straight ahead (random 0 -> straight face 0)
        assert_eq!(face_after(&mut f, |_| {}), 0);
        // dead beats everything
        assert_eq!(
            face_after(&mut f, |p| {
                p.health = 0;
                p.cheats = Cheat::GodMode as i32;
            }),
            ST_DEADFACE
        );
        // god mode
        assert_eq!(
            face_after(&mut f, |p| p.cheats = Cheat::GodMode as i32),
            ST_GODFACE
        );
        // invulnerability power-up too
        assert_eq!(
            face_after(&mut f, |p| p.powers
                [PowerType::PwInvulnerability as usize] =
                100),
            ST_GODFACE
        );
        // hurt by yourself: rampage (damage w/o attacker) at 100% health
        assert_eq!(face_after(&mut f, |p| p.damagecount = 5), ST_RAMPAGEOFFSET);
        // a big health jump while hurt: ouch
        assert_eq!(
            {
                with(|s| s.oldhealth = 0);
                with(|s| {
                    s.face_priority = 0;
                    s.facecount = 0;
                    s.pain_oldhealth = -1;
                });
                let mut p = f.plyr.clone();
                p.damagecount = 5;
                p.health = 100;
                with(|s| update_face_widget(s, &p, &f.thinkers));
                st_faceindex()
            },
            ST_OUCHOFFSET
        );
        // picking up a weapon: evil grin
        assert_eq!(
            face_after(&mut f, |p| {
                p.bonuscount = 5;
                p.weaponowned[WeaponType::WpShotgun as usize] = true;
            }),
            ST_EVILGRINOFFSET
        );
    }

    #[test]
    fn face_turns_toward_the_attacker() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        // Player faces east (angle 0). An attacker to the north is 90
        // degrees to the left: turn-left face; to the south: right.
        let north = add_mobj(&mut f.thinkers, 0, 100 << 16);
        let south = add_mobj(&mut f.thinkers, 0, -(100 << 16));
        let east = add_mobj(&mut f.thinkers, 100 << 16, 0);
        assert_eq!(
            face_after(&mut f, |p| {
                p.damagecount = 5;
                p.attacker = Some(north);
            }),
            ST_TURNOFFSET + 1
        );
        assert_eq!(
            face_after(&mut f, |p| {
                p.damagecount = 5;
                p.attacker = Some(south);
            }),
            ST_TURNOFFSET
        );
        // head-on: within 45 degrees
        assert_eq!(
            face_after(&mut f, |p| {
                p.damagecount = 5;
                p.attacker = Some(east);
            }),
            ST_RAMPAGEOFFSET
        );
        // the player's own mobj as "attacker" doesn't count
        let own = f.plyr.mo;
        assert_eq!(
            face_after(&mut f, |p| {
                p.damagecount = 5;
                p.attacker = Some(own);
            }),
            ST_RAMPAGEOFFSET,
            "falls through to the self-inflicted-damage branch"
        );
    }

    #[test]
    fn palette_shifts_and_change_detection() {
        let Some(f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut p = f.plyr.clone();
        with(|s| {
            s.palette = 0;
            assert!(
                do_palette_stuff(s, &p).is_none(),
                "no change: nothing to set"
            );

            p.damagecount = 20; // (20+7)>>3 = 3, +1
            let pal = do_palette_stuff(s, &p).expect("changed");
            assert_eq!(s.palette, 4);
            assert_eq!(pal, s.playpal[4 * 768..5 * 768]);
            assert!(do_palette_stuff(s, &p).is_none(), "same again: no re-set");

            p.damagecount = 255; // clamps at NUMREDPALS-1, +1
            do_palette_stuff(s, &p);
            assert_eq!(s.palette, 8);

            p.damagecount = 0;
            p.bonuscount = 20; // (20+7)>>3 = 3 -> +9
            do_palette_stuff(s, &p);
            assert_eq!(s.palette, 12);
            p.bonuscount = 1000; // clamps at NUMBONUSPALS-1
            do_palette_stuff(s, &p);
            assert_eq!(s.palette, 12);

            p.bonuscount = 0;
            p.powers[PowerType::PwIronfeet as usize] = 200; // > 4*32
            do_palette_stuff(s, &p);
            assert_eq!(s.palette, RADIATIONPAL);
            p.powers[PowerType::PwIronfeet as usize] = 8; // blinking phase
            do_palette_stuff(s, &p);
            assert_eq!(s.palette, RADIATIONPAL);
            p.powers[PowerType::PwIronfeet as usize] = 16;
            do_palette_stuff(s, &p);
            assert_eq!(s.palette, 0);

            // berzerk red fades: 12 - (strength>>6)
            p.powers[PowerType::PwStrength as usize] = 1;
            do_palette_stuff(s, &p);
            assert_eq!(s.palette, ((12 + 7) >> 3) + STARTREDPALS);
        });
    }

    #[test]
    fn keyboxes_show_skulls_over_cards() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.plyr.cards[0] = true; // blue card
        f.plyr.cards[1] = true; // yellow card
        f.plyr.cards[4] = true; // yellow skull wins
        st_ticker(&f.plyr, &f.thinkers);
        with(|s| assert_eq!(s.keyboxes, [0, 4, -1]));
    }

    #[test]
    fn frags_count_others_minus_self() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.plyr.frags = [5, 2, 1, 0]; // consoleplayer 0: self counts negative
        st_ticker(&f.plyr, &f.thinkers);
        with(|s| assert_eq!(s.fragscount, 2 + 1 - 5));
    }

    #[test]
    fn first_draw_paints_the_bar_and_later_draws_only_update_changes() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.plyr.health = 100;
        st_ticker(&f.plyr, &f.thinkers);
        let pal = st_drawer(false, false, &f.plyr, &mut f.v);
        assert!(pal.is_none() || pal.is_some());

        // The bar's background is on screen: the STBAR patch, drawn at
        // (0, ST_Y).
        let mut reference = VVideo::new();
        reference.screens[BG] = vec![0; (ST_WIDTH * ST_HEIGHT) as usize];
        with(|s| reference.v_draw_patch(ST_X, 0, BG, &s.gfx.sbar));
        let sw = crate::doomdef::SCREENWIDTH as usize;
        // a column the widgets never touch (the far left edge)
        for row in 0..ST_HEIGHT as usize {
            assert_eq!(
                f.v.screens[FG][(ST_Y as usize + row) * sw],
                reference.screens[BG][row * sw],
                "row {row}"
            );
        }
        // nothing above the bar was touched
        assert!(f.v.screens[FG][..(ST_Y as usize) * sw]
            .iter()
            .all(|&b| b == 0));

        // second draw, nothing changed: a scribble inside the bar stays
        let probe = (ST_Y as usize + 1) * sw + 1;
        f.v.screens[FG][probe] = 77;
        st_drawer(false, false, &f.plyr, &mut f.v);
        assert_eq!(f.v.screens[FG][probe], 77);

        // refresh redraws it
        st_drawer(false, true, &f.plyr, &mut f.v);
        assert_ne!(f.v.screens[FG][probe], 77);
    }

    #[test]
    fn health_number_redraws_when_health_changes() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.plyr.health = 100;
        st_ticker(&f.plyr, &f.thinkers);
        st_drawer(false, false, &f.plyr, &mut f.v);
        let sw = crate::doomdef::SCREENWIDTH as usize;
        let region = |v: &VVideo| -> Vec<u8> {
            let mut out = Vec::new();
            for y in ST_HEALTHY..ST_HEALTHY + 16 {
                let o = y as usize * sw;
                out.extend_from_slice(&v.screens[FG][o + 50..o + 90]);
            }
            out
        };
        let before = region(&f.v);
        f.plyr.health = 67;
        st_ticker(&f.plyr, &f.thinkers);
        st_drawer(false, false, &f.plyr, &mut f.v);
        assert_ne!(before, region(&f.v));
    }

    #[test]
    fn fullscreen_hides_the_bar_unless_the_automap_is_up() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        st_drawer(true, false, &f.plyr, &mut f.v);
        let sw = crate::doomdef::SCREENWIDTH as usize;
        assert!(
            f.v.screens[FG][ST_Y as usize * sw..]
                .iter()
                .all(|&b| b == 0),
            "fullscreen: no status bar"
        );
    }

    fn key(c: char) -> Event {
        Event {
            event_type: EvType::KeyDown,
            data1: c as i32,
            data2: 0,
            data3: 0,
        }
    }

    fn type_str(f: &mut Fixture, wad: &mut WadFiles, text: &str) {
        for c in text.chars() {
            st_responder(&key(c), &mut f.plyr, &mut f.thinkers, wad);
        }
    }

    #[test]
    fn cheats_god_kfa_noclip_behold_choppers() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut wad = WadFiles::new(); // unused by these cheats
        f.plyr.health = 30;

        type_str(&mut f, &mut wad, "iddqd");
        assert_ne!(f.plyr.cheats & Cheat::GodMode as i32, 0);
        assert_eq!(f.plyr.health, 100);
        assert_eq!(f.thinkers.mobj(f.plyr.mo).unwrap().health, 100);
        assert_eq!(f.plyr.message, Some(msg::STSTR_DQDON));
        type_str(&mut f, &mut wad, "iddqd");
        assert_eq!(f.plyr.cheats & Cheat::GodMode as i32, 0);
        assert_eq!(f.plyr.message, Some(msg::STSTR_DQDOFF));

        type_str(&mut f, &mut wad, "idfa");
        assert_eq!(f.plyr.armorpoints, 200);
        assert!(f.plyr.weaponowned[..NUMWEAPONS].iter().all(|&w| w));
        assert_eq!(f.plyr.ammo, f.plyr.maxammo);
        assert!(!f.plyr.cards[0], "idfa gives no keys");
        type_str(&mut f, &mut wad, "idkfa");
        assert!(f.plyr.cards.iter().all(|&c| c));
        assert_eq!(f.plyr.message, Some(msg::STSTR_KFAADDED));

        type_str(&mut f, &mut wad, "idspispopd");
        assert_ne!(f.plyr.cheats & Cheat::NoClip as i32, 0);
        assert_eq!(f.plyr.message, Some(msg::STSTR_NCON));
        type_str(&mut f, &mut wad, "idclip");
        assert_eq!(f.plyr.cheats & Cheat::NoClip as i32, 0);

        type_str(&mut f, &mut wad, "idchoppers");
        assert!(f.plyr.weaponowned[WeaponType::WpChainsaw as usize]);
        assert_ne!(f.plyr.powers[PowerType::PwInvulnerability as usize], 0);
        assert_eq!(f.plyr.message, Some(msg::STSTR_CHOPPERS));

        type_str(&mut f, &mut wad, "idbehold");
        assert_eq!(f.plyr.message, Some(msg::STSTR_BEHOLD));
        f.plyr.message = None;
        // The original doesn't re-examine the mismatching key as a new
        // first letter, so the half-matched "idbehold?" sequences need a
        // stray key to reset before the next cheat can register.
        type_str(&mut f, &mut wad, "x");
        type_str(&mut f, &mut wad, "idbeholds"); // strength toggles on...
        assert_ne!(f.plyr.powers[PowerType::PwStrength as usize], 0);
        assert_eq!(f.plyr.message, Some(msg::STSTR_BEHOLDX));
        type_str(&mut f, &mut wad, "idbeholds"); // ...and off
        assert_eq!(f.plyr.powers[PowerType::PwStrength as usize], 0);
        type_str(&mut f, &mut wad, "idbeholdi");
        assert_ne!(f.plyr.powers[PowerType::PwInvisibility as usize], 0);
        type_str(&mut f, &mut wad, "idbeholdi"); // others drop to 1 tic
        assert_eq!(f.plyr.powers[PowerType::PwInvisibility as usize], 1);
    }

    #[test]
    fn mypos_reports_angle_and_position_in_hex() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut wad = WadFiles::new();
        {
            let m = f.thinkers.mobj_mut(f.plyr.mo).unwrap();
            m.x = 0x10000;
            m.y = -0x10000;
            m.angle = 0x4000_0000;
        }
        type_str(&mut f, &mut wad, "idmypos");
        assert_eq!(
            f.plyr.message,
            Some("ang=0x40000000;x,y=(0x10000,0xffff0000)")
        );
    }

    #[test]
    fn clev_rejects_maps_the_game_mode_lacks() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut wad = WadFiles::new();
        // gamemode is Indetermined in the default state: the range
        // checks for the named modes don't apply, but map/episode < 1
        // still bail out before anything happens.
        f.plyr.message = None;
        type_str(&mut f, &mut wad, "idclev00");
        assert_eq!(f.plyr.message, None);
        type_str(&mut f, &mut wad, "idclev10");
        assert_eq!(f.plyr.message, None);
    }

    #[test]
    fn automap_notifications_switch_the_status_bar_state() {
        let Some(mut f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut wad = WadFiles::new();
        let up = |code| Event {
            event_type: EvType::KeyUp,
            data1: code,
            data2: 0,
            data3: 0,
        };
        st_responder(&up(AM_MSGENTERED), &mut f.plyr, &mut f.thinkers, &mut wad);
        with(|s| {
            assert_eq!(s.gamestate, StStateEnum::AutomapState);
            assert!(s.firsttime);
        });
        st_responder(&up(AM_MSGEXITED), &mut f.plyr, &mut f.thinkers, &mut wad);
        with(|s| assert_eq!(s.gamestate, StStateEnum::FirstPersonState));
    }

    #[test]
    fn stop_returns_the_default_palette_once() {
        let Some(_f) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let pal = st_stop().expect("was running");
        assert_eq!(pal.len(), 768);
        assert!(st_stop().is_none(), "already stopped");
    }
}
