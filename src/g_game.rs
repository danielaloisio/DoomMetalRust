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
// DESCRIPTION:  none
//
//-----------------------------------------------------------------------------

//! Rust port of `g_game.h` / `g_game.c` (partial, see note below).
//!
//! # Scope
//!
//! Ported: [`g_build_ticcmd`] (`G_BuildTiccmd`, keyboard/mouse only —
//! see below), [`g_responder`] (`G_Responder`, trimmed to what
//! [`Controls`] tracks — see below), [`g_ticker`] (`G_Ticker`, trimmed
//! to the `GS_LEVEL` single-player path), [`g_init_new`] (`G_InitNew`),
//! [`g_do_load_level`] (`G_DoLoadLevel`), [`g_exit_level`]/
//! [`g_secret_exit_level`] (`G_ExitLevel`/`G_SecretExitLevel`, Phase
//! 7b4 — `p_spec.c`'s level-exit specials need these).
//!
//! Not ported (deferred, `TODO(Phase N)` at each call site, same
//! convention as every other not-yet-ported call in this port):
//! - Joystick input — the original reads `joyxmove`/`joybuttons` in
//!   `G_BuildTiccmd`; this port has no joystick backend
//!   ([`crate::i_video`] is keyboard/mouse only), so those branches are
//!   dropped rather than stubbed with dead fields.
//! - `gameaction`'s non-level actions (`ga_loadgame`/`ga_savegame`/
//!   `ga_playdemo`/`ga_completed`/`ga_victory`/`ga_worlddone`/
//!   `ga_screenshot`) — save/load (Phase 10), demos (Phase 9/11),
//!   intermission/finale (Phase 9). [`g_ticker`] only handles
//!   `GameAction::Nothing`/`NewGame`/`LoadLevel`.
//! - `ST_Ticker`/`AM_Ticker`/`HU_Ticker` (status bar, automap, message
//!   HUD — Phase 9) and their `G_Responder` counterparts
//!   (`ST_Responder`/`AM_Responder`/`HU_Responder`) — none of those
//!   subsystems exist yet, so nothing intercepts input ahead of the
//!   movement/button handling [`g_responder`] does port.
//! - Turbo-cheat message, consistency checking, demo ticcmd read/write
//!   (all netgame/demo bookkeeping — Phase 9/11); [`g_ticker`] applies
//!   `players[i].cmd` directly instead of routing it through
//!   `netcmds`/`consistancy`, since there's no netcode generating those
//!   yet.
//! - `G_PlayerReborn`/`G_DoReborn`/death handling beyond what
//!   [`crate::p_user::p_player_think`] already covers — full respawn
//!   flow needs `P_SpawnPlayer` (still partial, see `p_setup`'s module
//!   docs on the `player_t` linkage gap).
//!
//! # `Controls`: this port's input-state block
//!
//! The original tracks `gamekeydown[NUMKEYS]`/`mousebuttons[]`/
//! `mousex`/`mousey`/`dclick*` as file-level globals, written by
//! `G_Responder` and consumed (and partly reset) by `G_BuildTiccmd` —
//! textbook same-tic producer/consumer state, not persisted meaning
//! beyond "what's held right now". [`Controls`] bundles exactly that
//! into one struct instead of adding more `GlobalCell` statics,
//! following the same "explicit state instead of a new global" call
//! this port has made since `p_maputl::PathTraverse` (Phase 6c). Double-
//! click detection (`dclick*`) for the mouse forward/strafe buttons is
//! ported as-is (same counters/thresholds), since it's cheap and
//! genuinely part of `G_BuildTiccmd`'s behavior, not a stub-worthy
//! subsystem.

use crate::d_event::{
    EvType, Event, BTS_PAUSE, BT_ATTACK, BT_CHANGE, BT_SPECIAL, BT_USE, BT_WEAPONSHIFT,
};
use crate::d_player::{Player, PlayerState};
use crate::d_ticcmd::TicCmd;
use crate::doomdef::{
    GameMode, GameState as GameStateEnum, Skill, KEY_DOWNARROW, KEY_LEFTARROW, KEY_PAUSE, KEY_RALT,
    KEY_RCTRL, KEY_RIGHTARROW, KEY_RSHIFT, KEY_STRAFELEFT, KEY_STRAFERIGHT, KEY_UPARROW,
    NUMWEAPONS,
};
use crate::doomstat;
use crate::m_fixed::Fixed;
use crate::m_random::m_clear_random;
use crate::p_setup::{self, Level};
use crate::p_tick::{p_ticker, Thinkers};
use crate::r_main::Renderer;
use crate::w_wad::WadFiles;

/// (`NUMKEYS`, `g_game.c`).
const NUMKEYS: usize = 256;
/// (`SLOWTURNTICS`, `g_game.c`).
const SLOWTURNTICS: i32 = 6;
/// (`TURBOTHRESHOLD`, `g_game.c`) — unused (see module docs, the turbo
/// message is HUD text, Phase 9), kept for reference alongside the
/// other tuning constants this file ports.
#[allow(dead_code)]
const TURBOTHRESHOLD: i8 = 0x32;

/// (`forwardmove[2]`, `g_game.c`).
const FORWARDMOVE: [Fixed; 2] = [0x19, 0x32];
/// (`sidemove[2]`, `g_game.c`).
const SIDEMOVE: [Fixed; 2] = [0x18, 0x28];
/// (`angleturn[3]`, `g_game.c`) — `+ slow turn` (the original's own
/// comment, preserved).
const ANGLETURN: [i16; 3] = [640, 1280, 320];
/// (`MAXPLMOVE`, `g_game.c`'s own `#define (forwardmove[1])`).
const MAXPLMOVE: Fixed = FORWARDMOVE[1];

/// Default key bindings (`m_misc.c`'s `defaults[]` — this port has no
/// config file loader yet, Phase 9, so these are the fixed values that
/// file would otherwise load).
const KEY_RIGHT: i32 = KEY_RIGHTARROW;
const KEY_LEFT: i32 = KEY_LEFTARROW;
const KEY_UP: i32 = KEY_UPARROW;
const KEY_DOWN: i32 = KEY_DOWNARROW;
const KEY_STRAFELEFT_: i32 = KEY_STRAFELEFT;
const KEY_STRAFERIGHT_: i32 = KEY_STRAFERIGHT;
const KEY_FIRE: i32 = KEY_RCTRL;
const KEY_USE: i32 = b' ' as i32;
const KEY_STRAFE: i32 = KEY_RALT;
const KEY_SPEED: i32 = KEY_RSHIFT;

/// Standing in for the original's `gamekeydown[NUMKEYS]`/
/// `mousebuttons[]`/`mousex`/`mousey`/`turnheld`/`dclick*` globals — see
/// module docs.
#[derive(Debug, Clone)]
pub struct Controls {
    pub gamekeydown: [bool; NUMKEYS],
    /// `mousebuttons[0..3]` (the original allows `[-1]` too, for a
    /// fourth unused slot this port has no need to reproduce).
    pub mousebuttons: [bool; 3],
    pub mousex: i32,
    pub mousey: i32,
    pub turnheld: i32,
    pub dclicktime: i32,
    pub dclickstate: bool,
    pub dclicks: i32,
    pub dclicktime2: i32,
    pub dclickstate2: bool,
    pub dclicks2: i32,
    pub sendpause: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Controls {
            gamekeydown: [false; NUMKEYS],
            mousebuttons: [false; 3],
            mousex: 0,
            mousey: 0,
            turnheld: 0,
            dclicktime: 0,
            dclickstate: false,
            dclicks: 0,
            dclicktime2: 0,
            dclickstate2: false,
            dclicks2: 0,
            sendpause: false,
        }
    }
}

/// Mouse sensitivity divisor, standing in for `doomstat`'s
/// `mouse_sensitivity` (already ported) — kept local since
/// [`g_responder`] only reads it, matching the original's
/// `(mouseSensitivity+5)/10` inline scaling.
fn mouse_scale(sensitivity: i32) -> i32 {
    (sensitivity + 5) / 10
}

/// Port of `G_Responder`. Get info needed to make ticcmd_ts for the
/// players (the original's own comment, preserved). Trimmed to the
/// keyboard/mouse cases `Controls` tracks — see module docs.
pub fn g_responder(controls: &mut Controls, ev: &Event) -> bool {
    match ev.event_type {
        EvType::KeyDown => {
            if ev.data1 == KEY_PAUSE {
                controls.sendpause = true;
                return true;
            }
            if (ev.data1 as usize) < NUMKEYS {
                controls.gamekeydown[ev.data1 as usize] = true;
            }
            true // eat key down events (the original's own comment, preserved)
        }
        EvType::KeyUp => {
            if (ev.data1 as usize) < NUMKEYS {
                controls.gamekeydown[ev.data1 as usize] = false;
            }
            false // always let key up events filter down (the original's own comment, preserved)
        }
        EvType::Mouse => {
            controls.mousebuttons[0] = ev.data1 & 1 != 0;
            controls.mousebuttons[1] = ev.data1 & 2 != 0;
            controls.mousebuttons[2] = ev.data1 & 4 != 0;
            let scale = mouse_scale(doomstat::state().mouse_sensitivity);
            controls.mousex = ev.data2 * scale;
            controls.mousey = ev.data3 * scale;
            true // eat events (the original's own comment, preserved)
        }
        EvType::Joystick => false, // no joystick backend — see module docs
    }
}

/// Port of `G_BuildTiccmd`. Builds a ticcmd from all of the available
/// inputs (the original's own comment, preserved) — keyboard/mouse
/// only, see module docs. `mousebstrafe`/`mousebforward` are fixed at
/// button index 1/2 (right-click/middle-click), matching the original's
/// own hardcoded `m_misc.c` defaults (`mousebstrafe = 1`,
/// `mousebforward = 2`; `mousebfire = 0` is `controls.mousebuttons[0]`,
/// read directly rather than through a named constant since nothing
/// else in this function needs to name it).
pub fn g_build_ticcmd(controls: &mut Controls, ticdup: i32) -> TicCmd {
    let mut cmd = TicCmd::default();

    let strafe = controls.gamekeydown[KEY_STRAFE as usize] || controls.mousebuttons[1];
    let speed = controls.gamekeydown[KEY_SPEED as usize];

    let mut forward: i32 = 0;
    let mut side: i32 = 0;

    // use two stage accelerative turning on the keyboard (the
    // original's own comment, preserved; joystick half dropped, see
    // module docs).
    if controls.gamekeydown[KEY_RIGHT as usize] || controls.gamekeydown[KEY_LEFT as usize] {
        controls.turnheld += ticdup;
    } else {
        controls.turnheld = 0;
    }

    let tspeed = if controls.turnheld < SLOWTURNTICS {
        2 // slow turn
    } else {
        speed as i32
    };

    let speed_idx = speed as usize;
    let tspeed_idx = tspeed as usize;

    // let movement keys cancel each other out (the original's own
    // comment, preserved).
    if strafe {
        if controls.gamekeydown[KEY_RIGHT as usize] {
            side += SIDEMOVE[speed_idx];
        }
        if controls.gamekeydown[KEY_LEFT as usize] {
            side -= SIDEMOVE[speed_idx];
        }
    } else {
        if controls.gamekeydown[KEY_RIGHT as usize] {
            cmd.angleturn -= ANGLETURN[tspeed_idx];
        }
        if controls.gamekeydown[KEY_LEFT as usize] {
            cmd.angleturn += ANGLETURN[tspeed_idx];
        }
    }

    if controls.gamekeydown[KEY_UP as usize] {
        forward += FORWARDMOVE[speed_idx];
    }
    if controls.gamekeydown[KEY_DOWN as usize] {
        forward -= FORWARDMOVE[speed_idx];
    }
    if controls.gamekeydown[KEY_STRAFERIGHT_ as usize] {
        side += SIDEMOVE[speed_idx];
    }
    if controls.gamekeydown[KEY_STRAFELEFT_ as usize] {
        side -= SIDEMOVE[speed_idx];
    }

    // chat character queued by the heads-up code (multiplayer chat).
    cmd.chatchar = crate::hu_stuff::hu_dequeue_chat_char();

    // buttons (the original's own comment, preserved).
    if controls.gamekeydown[KEY_FIRE as usize] || controls.mousebuttons[0] {
        cmd.buttons |= BT_ATTACK as u8;
    }
    if controls.gamekeydown[KEY_USE as usize] {
        cmd.buttons |= BT_USE as u8;
        // clear double clicks if hit use button (the original's own
        // comment, preserved).
        controls.dclicks = 0;
    }

    // chainsaw overrides (the original's own comment, preserved):
    // number keys 1..NUMWEAPONS-1 select a weapon directly.
    for i in 0..(NUMWEAPONS - 1) {
        let key = b'1' as usize + i;
        if key < NUMKEYS && controls.gamekeydown[key] {
            cmd.buttons |= BT_CHANGE as u8;
            cmd.buttons |= (i as u8) << BT_WEAPONSHIFT;
            break;
        }
    }

    // mouse (the original's own comment, preserved).
    if controls.mousebuttons[2] {
        forward += FORWARDMOVE[speed_idx];
    }

    // forward double click (the original's own comment, preserved).
    if controls.mousebuttons[2] != controls.dclickstate && controls.dclicktime > 1 {
        controls.dclickstate = controls.mousebuttons[2];
        if controls.dclickstate {
            controls.dclicks += 1;
        }
        if controls.dclicks == 2 {
            cmd.buttons |= BT_USE as u8;
            controls.dclicks = 0;
        } else {
            controls.dclicktime = 0;
        }
    } else {
        controls.dclicktime += ticdup;
        if controls.dclicktime > 20 {
            controls.dclicks = 0;
            controls.dclickstate = false;
        }
    }

    // strafe double click (the original's own comment, preserved).
    let bstrafe = controls.mousebuttons[1];
    if bstrafe != controls.dclickstate2 && controls.dclicktime2 > 1 {
        controls.dclickstate2 = bstrafe;
        if controls.dclickstate2 {
            controls.dclicks2 += 1;
        }
        if controls.dclicks2 == 2 {
            cmd.buttons |= BT_USE as u8;
            controls.dclicks2 = 0;
        } else {
            controls.dclicktime2 = 0;
        }
    } else {
        controls.dclicktime2 += ticdup;
        if controls.dclicktime2 > 20 {
            controls.dclicks2 = 0;
            controls.dclickstate2 = false;
        }
    }

    forward += controls.mousey;
    if strafe {
        side += controls.mousex * 2;
    } else {
        cmd.angleturn -= (controls.mousex * 0x8) as i16;
    }
    controls.mousex = 0;
    controls.mousey = 0;

    forward = forward.clamp(-MAXPLMOVE, MAXPLMOVE);
    side = side.clamp(-MAXPLMOVE, MAXPLMOVE);

    cmd.forwardmove = cmd.forwardmove.saturating_add(forward as i8);
    cmd.sidemove = cmd.sidemove.saturating_add(side as i8);

    // special buttons (the original's own comment, preserved)
    if controls.sendpause {
        controls.sendpause = false;
        cmd.buttons = BT_SPECIAL as u8 | BTS_PAUSE as u8;
    }

    if doomstat::state().sendsave {
        let state = doomstat::state_mut();
        state.sendsave = false;
        cmd.buttons = (BT_SPECIAL
            | crate::d_event::BTS_SAVEGAME
            | (state.savegameslot << crate::d_event::BTS_SAVESHIFT)) as u8;
    }

    cmd
}

/// Everything the running game owns, gathered so the game flow
/// (`G_Ticker`'s actions, level loading, intermission, finale) can be
/// driven from one place instead of a dozen loose parameters.
pub struct GameWorld {
    pub thinkers: Thinkers,
    pub level: Level,
    pub renderer: Renderer,
    pub wad: WadFiles,
    /// `players[]`: grown as players join (slots of players not in the
    /// game hold a placeholder whose `mo` is [`ThinkerId::NONE`]);
    /// `doomstat::playeringame` says which are real.
    pub players: Vec<Player>,
    pub specials: crate::p_spec::SpecialsState,
    pub switches: crate::p_switch::SwitchState,
    pub active_plats: crate::p_plats::ActivePlats,
    pub active_ceilings: crate::p_ceilng::ActiveCeilings,
    pub brain: crate::p_enemy::BrainTargets,
    /// Legacy per-tic `validcount` parameter (see
    /// `p_maputl::bump_validcount`); ignored where it used to matter.
    pub validcount: i32,
    pub controls: Controls,
    /// `bodyque[BODYQUESIZE]`/`bodyqueslot`: the corpses of respawned
    /// players, flushed oldest-first (netgames).
    pub bodyque: [crate::p_tick::ThinkerId; BODYQUESIZE],
    pub bodyqueslot: usize,
    /// `consistancy[MAXPLAYERS][BACKUPTICS]`: each player's `mo->x` (or
    /// `rndindex`) per tic, compared with what the other machines send.
    pub consistancy: [[i16; crate::d_net::BACKUPTICS]; crate::doomdef::MAXPLAYERS as usize],
    /// Run `G_Ticker`'s netgame consistency check (set when a net game
    /// starts: the original runs it whenever `netgame`, but only the net
    /// protocol fills in the commands' `consistancy` field).
    pub consistency_check: bool,
}

/// (`BODYQUESIZE`, `g_game.c`)
pub const BODYQUESIZE: usize = 32;

impl GameWorld {
    /// A world around an opened IWAD and an initialised renderer, with
    /// no level loaded yet.
    pub fn new(wad: WadFiles, renderer: Renderer) -> Self {
        let mut specials = crate::p_spec::SpecialsState::default();
        specials.p_init_pic_anims(&wad, &renderer.rdata);
        let mut switches = crate::p_switch::SwitchState::default();
        switches.p_init_switch_list(&renderer.rdata, doomstat::state().gamemode);
        GameWorld {
            thinkers: Thinkers::new(),
            level: Level::default(),
            renderer,
            wad,
            players: Vec::new(),
            specials,
            switches,
            active_plats: Default::default(),
            active_ceilings: Default::default(),
            brain: Default::default(),
            validcount: 0,
            controls: Controls::default(),
            bodyque: [crate::p_tick::ThinkerId::NONE; BODYQUESIZE],
            bodyqueslot: 0,
            consistancy: [[0; crate::d_net::BACKUPTICS]; crate::doomdef::MAXPLAYERS as usize],
            consistency_check: false,
        }
    }

    /// `playeringame[]` for the players that exist.
    pub fn playeringame(&self) -> Vec<bool> {
        let flags = doomstat::state().playeringame;
        (0..self.players.len())
            .map(|i| flags.get(i).copied().unwrap_or(false))
            .collect()
    }
}

/// Port of `G_InitNew`. Nightmare/fast-monster state-tic-halving and the
/// missile-speed bumps aren't ported (`info::STATES`/`MOBJINFO` are
/// immutable statics here) — everything else applies.
pub fn g_init_new(world: &mut GameWorld, skill: Skill, episode: i32, map: i32) {
    let state = doomstat::state_mut();
    if state.paused {
        state.paused = false;
        crate::s_sound::s_resume_sound();
    }

    let mut skill = skill;
    if (skill as i32) > Skill::Nightmare as i32 {
        skill = Skill::Nightmare;
    }

    // This was quite messy with SPECIAL and commented parts (the
    // original's own comment, preserved).
    let mut episode = episode.max(1);
    match state.gamemode {
        GameMode::Retail => episode = episode.min(4),
        GameMode::Shareware => episode = episode.min(1), // only start episode 1 on shareware
        _ => episode = episode.min(3),
    }

    let mut map = map.max(1);
    if map > 9 && state.gamemode != GameMode::Commercial {
        map = 9;
    }

    // `D_CheckNetGame` always leaves at least player 1 in the game; code
    // that starts a game without it (tests, a bare `G_InitNew`) gets the
    // single-player default.
    if !state.playeringame.iter().any(|&b| b) {
        state.playeringame[0] = true;
    }

    m_clear_random();

    state.respawnmonsters = skill == Skill::Nightmare || state.respawnparm;

    // force players to be initialized upon first level load (the
    // original's own comment, preserved).
    for p in &mut world.players {
        p.playerstate = PlayerState::Reborn;
    }

    state.usergame = true; // will be set false if a demo (the original's own comment, preserved)
    state.paused = false;
    state.demoplayback = false;
    state.automapactive = false;
    state.viewactive = true;
    state.gameepisode = episode;
    state.gamemap = map;
    state.gameskill = skill;

    // set the sky map for the episode (the original's own comment,
    // preserved).
    let skytexture_name = if state.gamemode == GameMode::Commercial {
        if map < 12 {
            "SKY1"
        } else if map < 21 {
            "SKY2"
        } else {
            "SKY3"
        }
    } else {
        match episode {
            1 => "SKY1",
            2 => "SKY2",
            3 => "SKY3",
            4 => "SKY4", // Special Edition sky (the original's own comment, preserved)
            _ => "SKY1",
        }
    };
    world.renderer.sky.skytexture = world.renderer.rdata.texture_num_for_name(skytexture_name);

    g_do_load_level(world);
}

/// Port of `G_PlayerReborn`. Called after a player dies: almost
/// everything is cleared and initialized (the original's own comment).
pub fn g_player_reborn(p: &mut Player) {
    let frags = p.frags;
    let (killcount, itemcount, secretcount) = (p.killcount, p.itemcount, p.secretcount);

    *p = Player::blank(p.mo);

    p.frags = frags;
    p.killcount = killcount;
    p.itemcount = itemcount;
    p.secretcount = secretcount;

    p.usedown = true; // don't do anything immediately
    p.attackdown = true;
    p.playerstate = PlayerState::Live;
    p.health = MAXHEALTH;
    p.readyweapon = crate::doomdef::WeaponType::WpPistol;
    p.pendingweapon = crate::doomdef::WeaponType::WpPistol;
    p.weaponowned[crate::doomdef::WeaponType::WpFist as usize] = true;
    p.weaponowned[crate::doomdef::WeaponType::WpPistol as usize] = true;
    p.ammo[crate::doomdef::AmmoType::AmClip as usize] = 50;
    p.maxammo = MAXAMMO;
}

/// (`MAXHEALTH`, `p_local.h`)
pub const MAXHEALTH: i32 = 100;
/// (`maxammo[NUMAMMO]`, `p_inter.c`)
pub const MAXAMMO: [i32; crate::doomdef::NUMAMMO] = [200, 50, 300, 50];

/// Makes sure `world.players` has a slot for `playernum` (earlier empty
/// slots hold a placeholder with no mobj).
fn ensure_player_slot(world: &mut GameWorld, playernum: usize) {
    while world.players.len() <= playernum {
        world
            .players
            .push(Player::blank(crate::p_tick::ThinkerId::NONE));
    }
}

/// Port of `P_SpawnPlayer`: called when a player is spawned on the level
/// (level load, or a netgame respawn at a start). Spawns the `MT_PLAYER`
/// mobj at `mthing` and links it to the player.
pub fn p_spawn_player(world: &mut GameWorld, mthing: &crate::doomdata::MapThing) {
    let idx = (mthing.thing_type - 1) as usize;
    // not playing?
    if !doomstat::state().playeringame[idx] {
        return;
    }
    let mo = p_setup::p_spawn_player_mobj(&mut world.thinkers, &mut world.level, mthing);
    link_player(world, idx, mo);
}

/// The `player_t` half of `P_SpawnPlayer` for a freshly spawned
/// `MT_PLAYER` mobj `mo` (reborn defaults if it died, `mobj->player`/
/// `p->mo`, psprites, and waking up the status bar/HUD for the console
/// player).
pub fn link_player(world: &mut GameWorld, playernum: usize, mo: crate::p_tick::ThinkerId) {
    ensure_player_slot(world, playernum);

    if world.players[playernum].playerstate == PlayerState::Reborn {
        g_player_reborn(&mut world.players[playernum]);
    }

    let p = &mut world.players[playernum];
    let mobj = world.thinkers.mobj_mut(mo).expect("player start mobj");
    mobj.player = Some(playernum);
    mobj.health = p.health;

    p.mo = mo;
    p.playerstate = PlayerState::Live;
    p.refire = 0;
    p.message = None;
    p.damagecount = 0;
    p.bonuscount = 0;
    p.extralight = 0;
    p.fixedcolormap = 0;
    p.viewheight = crate::p_user::VIEWHEIGHT;

    // setup gun psprite
    crate::p_pspr::p_setup_psprites(&mut world.thinkers, &mut world.level, p);

    // give all cards in death match mode
    if doomstat::state().deathmatch {
        for c in p.cards.iter_mut() {
            *c = true;
        }
    }

    if playernum as i32 == doomstat::state().consoleplayer {
        // wake up the status bar
        if crate::st_stuff::st_is_initialized() {
            crate::st_stuff::st_start(&world.players[playernum]);
        }
        // wake up the heads up text
        if crate::hu_stuff::hu_is_initialized() {
            crate::hu_stuff::hu_start();
        }
    }
}

/// Port of `P_SetupLevel`: totals and per-player counters reset, the
/// level's lumps loaded, things spawned (players linked), specials
/// set up. `R_PrecacheLevel` isn't ported (the renderer caches lazily).
fn p_setup_level(world: &mut GameWorld) {
    let st = doomstat::state_mut();
    st.totalkills = 0;
    st.totalitems = 0;
    st.totalsecret = 0;
    st.wminfo.maxfrags = 0;
    st.wminfo.partime = 180;
    for p in &mut world.players {
        p.killcount = 0;
        p.secretcount = 0;
        p.itemcount = 0;
    }

    // Initial height of PointOfView will be set by player think.
    let console = st.consoleplayer as usize;
    if let Some(p) = world.players.get_mut(console) {
        p.viewz = 1;
    }

    // Make sure all sounds are stopped before Z_FreeTags: S_Start also
    // starts this level's music.
    crate::s_sound::s_start(&mut world.wad);

    world.thinkers = Thinkers::new(); // P_InitThinkers
    world.bodyqueslot = 0;
    world.bodyque = [crate::p_tick::ThinkerId::NONE; BODYQUESIZE];
    doomstat::state_mut().deathmatch_p = Some(0);

    let (episode, map, gamemode) = (st.gameepisode, st.gamemap, st.gamemode);
    let lumpname = if gamemode == GameMode::Commercial {
        format!("MAP{map:02}")
    } else {
        format!("E{episode}M{map}")
    };
    st.leveltime = 0;

    world.level = Level::load(&mut world.wad, &lumpname);
    world
        .level
        .resolve_textures(&world.renderer.rdata, &world.wad);
    let things_lump = world.wad.get_num_for_name(&lumpname) + 1; // + ML_THINGS
    p_setup::p_load_things(
        &mut world.thinkers,
        &mut world.level,
        &mut world.wad,
        things_lump,
    );

    // (P_SpawnMapThing calls P_SpawnPlayer for each start in game; its
    // mobj half ran inside p_load_things, the player_t linkage is done
    // here.)
    if !doomstat::state().deathmatch {
        let spawned: Vec<(usize, crate::p_tick::ThinkerId)> = world
            .thinkers
            .iter_mobjs()
            .filter(|(_, m)| m.mobj_type == crate::info::MobjType::MtPlayer)
            .filter_map(|(id, m)| m.player.map(|n| (n, id)))
            .collect();
        for (n, id) in spawned {
            link_player(world, n, id);
        }
    }

    // if deathmatch, randomly spawn the active players (the original's
    // own comment, preserved)
    if doomstat::state().deathmatch {
        let ingame = doomstat::state().playeringame;
        for (i, &here) in ingame.iter().enumerate() {
            if here {
                ensure_player_slot(world, i);
                world.players[i].mo = crate::p_tick::ThinkerId::NONE;
                g_death_match_spawn_player(world, i);
            }
        }
    }

    // set up world state
    world.specials.p_spawn_specials(
        &mut world.thinkers,
        &mut world.level,
        &world.wad,
        &mut world.active_plats,
        &mut world.active_ceilings,
        &mut world.switches,
    );
}

/// Port of `G_DoLoadLevel`. (`Z_CheckHeap` isn't ported: `z_zone`'s
/// heap check is meaningless here, see its docs.)
pub fn g_do_load_level(world: &mut GameWorld) {
    // Set the sky map (the original's own comment, preserved).
    world.renderer.sky.skyflatnum = world
        .renderer
        .rdata
        .flat_num_for_name(&world.wad, crate::r_sky::SKYFLATNAME);
    let gamemode = doomstat::state().gamemode;
    if gamemode == GameMode::Commercial {
        let map = doomstat::state().gamemap;
        let name = if map < 12 {
            "SKY1"
        } else if map < 21 {
            "SKY2"
        } else {
            "SKY3"
        };
        world.renderer.sky.skytexture = world.renderer.rdata.texture_num_for_name(name);
    }

    let state = doomstat::state_mut();
    state.levelstarttic = state.gametic; // for time calculation (the original's own comment, preserved)

    if state.wipegamestate == Some(GameStateEnum::Level) {
        state.wipegamestate = None; // force a wipe
    }

    state.gamestate = GameStateEnum::Level;

    for p in &mut world.players {
        if p.playerstate == PlayerState::Dead {
            p.playerstate = PlayerState::Reborn;
        }
        p.frags = [0; 4];
    }

    p_setup_level(world);

    let state = doomstat::state_mut();
    state.gameaction = crate::d_event::GameAction::Nothing;

    // clear cmd building stuff (the original's own comment, preserved)
    world.controls = Controls::default();
    state.paused = false;
}

/// Port of `G_CheckSpot`: false if the player cannot be respawned at
/// `mthing` because something is occupying it. On success flushes the
/// player's old corpse into the body queue and spawns a teleport fog.
pub fn g_check_spot(
    world: &mut GameWorld,
    playernum: usize,
    mthing: &crate::doomdata::MapThing,
) -> bool {
    use crate::m_fixed::FRACBITS;
    let (x, y) = ((mthing.x as i32) << FRACBITS, (mthing.y as i32) << FRACBITS);

    let corpse = world.players[playernum].mo;
    if world.thinkers.mobj(corpse).is_none() {
        // first spawn of level, before corpses
        for i in 0..playernum {
            if let Some(m) = world.thinkers.mobj(world.players[i].mo) {
                if m.x == x && m.y == y {
                    return false;
                }
            }
        }
        return true;
    }

    let ok = {
        let GameWorld {
            thinkers,
            level,
            players,
            renderer,
            ..
        } = world;
        crate::p_map::p_check_position(
            thinkers,
            level,
            players,
            &mut renderer.rmain,
            0,
            corpse,
            x,
            y,
        )
        .0
    };
    if !ok {
        return false;
    }

    // flush an old corpse if needed (the original's own comment,
    // preserved)
    if world.bodyqueslot >= BODYQUESIZE {
        let old = world.bodyque[world.bodyqueslot % BODYQUESIZE];
        if world.thinkers.mobj(old).is_some() {
            crate::p_mobj::p_remove_mobj(&mut world.thinkers, &mut world.level, old);
        }
    }
    world.bodyque[world.bodyqueslot % BODYQUESIZE] = corpse;
    world.bodyqueslot += 1;

    // spawn a teleport fog (the original's own comment, preserved)
    let ss = crate::r_main::RMain::point_in_subsector(&world.level, x, y);
    let floor = world.level.sectors[world.level.subsectors[ss].sector].floorheight;
    let an = ((crate::tables::ANG45.wrapping_mul((mthing.angle / 45) as u32))
        >> crate::tables::ANGLETOFINESHIFT) as usize;
    let fog = crate::p_mobj::p_spawn_mobj(
        &mut world.thinkers,
        &mut world.level,
        x + 20 * crate::tables::fine_cosine(an),
        y + 20 * crate::tables::FINESINE[an],
        crate::p_mobj::SpawnZ::At(floor),
        crate::info::MobjType::MtTfog,
    );
    let viewz = world
        .players
        .get(doomstat::state().consoleplayer as usize)
        .map_or(1, |p| p.viewz);
    if viewz != 1 {
        // don't start sound on first frame (the original's own comment,
        // preserved)
        crate::s_sound::s_start_sound(
            Some(crate::s_sound::SoundOrigin::Mobj(fog)),
            crate::sounds::Sfx::SfxTelept,
        );
    }
    true
}

/// Port of `G_DeathMatchSpawnPlayer`: spawns a player at one of the
/// random deathmatch spots; called at level load and each death.
///
/// # Panics
/// With fewer than 4 deathmatch spots (the original's `I_Error`).
pub fn g_death_match_spawn_player(world: &mut GameWorld, playernum: usize) {
    let st = doomstat::state();
    let selections = st.deathmatch_p.unwrap_or(0);
    if selections < 4 {
        panic!("Only {selections} deathmatch spots, 4 required");
    }

    for _ in 0..20 {
        let i = (crate::m_random::p_random() as usize) % selections;
        let spot = doomstat::state().deathmatchstarts[i];
        if g_check_spot(world, playernum, &spot) {
            let mut spot = spot;
            spot.thing_type = playernum as i16 + 1;
            doomstat::state_mut().deathmatchstarts[i].thing_type = spot.thing_type;
            p_spawn_player(world, &spot);
            return;
        }
    }

    // no good spot, so the player will probably get stuck (the
    // original's own comment, preserved)
    let start = doomstat::state().playerstarts[playernum];
    p_spawn_player(world, &start);
}

/// Port of `G_DoReborn`. Single player: reload the level. Netgame:
/// respawn at a start (a random spot in deathmatch, else the player's own
/// start, else any other player's free start, else the own one anyway).
pub fn g_do_reborn(world: &mut GameWorld, playernum: usize) {
    if !doomstat::state().netgame {
        // reload the level from scratch
        doomstat::state_mut().gameaction = crate::d_event::GameAction::LoadLevel;
        return;
    }

    // respawn at the start

    // first dissasociate the corpse
    if let Some(m) = world.thinkers.mobj_mut(world.players[playernum].mo) {
        m.player = None;
    }

    // spawn at random spot if in death match
    if doomstat::state().deathmatch {
        g_death_match_spawn_player(world, playernum);
        return;
    }

    let own = doomstat::state().playerstarts[playernum];
    if g_check_spot(world, playernum, &own) {
        p_spawn_player(world, &own);
        return;
    }

    // try to spawn at one of the other players spots (the original's own
    // comment, preserved)
    for i in 0..crate::doomdef::MAXPLAYERS as usize {
        let spot = doomstat::state().playerstarts[i];
        if g_check_spot(world, playernum, &spot) {
            let mut fake = spot;
            fake.thing_type = playernum as i16 + 1; // fake as other player
            p_spawn_player(world, &fake);
            return;
        }
        // he's going to be inside something. Too bad.
    }
    p_spawn_player(world, &own);
}

/// Port of `G_DoNewGame`.
pub fn g_do_new_game(world: &mut GameWorld) {
    let st = doomstat::state_mut();
    st.demoplayback = false;
    st.netgame = false;
    st.deathmatch = false;
    st.respawnparm = false;
    st.fastparm = false;
    st.nomonsters = false;
    st.consoleplayer = 0;
    let (skill, episode, map) = (st.d_skill, st.d_episode, st.d_map);
    g_init_new(world, skill, episode, map);
    doomstat::state_mut().gameaction = crate::d_event::GameAction::Nothing;
}

/// Par times per episode and map (`pars[4][10]`).
const PARS: [[i32; 10]; 4] = [
    [0; 10],
    [0, 30, 75, 120, 90, 165, 180, 180, 30, 165],
    [0, 90, 90, 90, 120, 90, 360, 240, 30, 170],
    [0, 90, 45, 90, 150, 90, 90, 165, 30, 135],
];

/// DOOM II par times (`cpars[32]`).
const CPARS: [i32; 32] = [
    30, 90, 120, 120, 90, 150, 120, 120, 270, 90, //  1-10
    210, 150, 150, 150, 210, 150, 420, 150, 210, 150, // 11-20
    240, 150, 180, 150, 150, 300, 330, 420, 300, 180, // 21-30
    120, 30, // 31-32
];

/// Port of `G_PlayerFinishLevel`. Can when a player completes a level
/// (the original's own comment): takes away cards and stuff.
fn g_player_finish_level(p: &mut Player, thinkers: &mut Thinkers) {
    p.powers = [0; crate::doomdef::NUMPOWERS];
    p.cards = [false; crate::doomdef::NUMCARDS];
    if let Some(mo) = thinkers.mobj_mut(p.mo) {
        mo.flags &= !crate::r_defs::mobj_flag::SHADOW; // cancel invisibility
    }
    p.extralight = 0; // cancel gun flashes
    p.fixedcolormap = 0; // cancel ir gogles
    p.damagecount = 0; // no palette changes
    p.bonuscount = 0;
}

/// Port of `G_DoCompleted`: the level is done — fills in `wminfo`
/// (stats, par time, next map) and starts the intermission, or goes to
/// the finale after episode-ending maps.
pub fn g_do_completed(world: &mut GameWorld, video: &mut crate::v_video::VVideo) {
    use crate::d_event::GameAction;
    doomstat::state_mut().gameaction = GameAction::Nothing;

    let ingame = doomstat::state().playeringame;
    for (i, p) in world.players.iter_mut().enumerate() {
        if ingame[i] {
            g_player_finish_level(p, &mut world.thinkers); // take away cards and stuff
        }
    }

    if doomstat::state().automapactive {
        let console = doomstat::state().consoleplayer as usize;
        if let Some(player) = world.players.get_mut(console) {
            crate::am_map::am_stop(&mut crate::am_map::AmCtx {
                player,
                thinkers: &mut world.thinkers,
                level: &world.level,
                wad: &mut world.wad,
            });
        }
    }

    let st = doomstat::state_mut();
    let (gamemode, gamemap, gameepisode) = (st.gamemode, st.gamemap, st.gameepisode);

    if gamemode != GameMode::Commercial {
        match gamemap {
            8 => {
                st.gameaction = GameAction::Victory;
                return;
            }
            9 => {
                for p in &mut world.players {
                    p.didsecret = true;
                }
            }
            _ => {}
        }
    }

    let console = st.consoleplayer as usize;
    st.wminfo.didsecret = world.players.get(console).is_some_and(|p| p.didsecret);
    st.wminfo.epsd = gameepisode - 1;
    st.wminfo.last = gamemap - 1;

    // wminfo.next is 0 biased, unlike gamemap (the original's own
    // comment, preserved)
    if gamemode == GameMode::Commercial {
        if st.secretexit {
            match gamemap {
                15 => st.wminfo.next = 30,
                31 => st.wminfo.next = 31,
                _ => {}
            }
        } else {
            match gamemap {
                31 | 32 => st.wminfo.next = 15,
                _ => st.wminfo.next = gamemap,
            }
        }
    } else if st.secretexit {
        st.wminfo.next = 8; // go to secret level
    } else if gamemap == 9 {
        // returning from secret level
        match gameepisode {
            1 => st.wminfo.next = 3,
            2 => st.wminfo.next = 5,
            3 => st.wminfo.next = 6,
            4 => st.wminfo.next = 2,
            _ => {}
        }
    } else {
        st.wminfo.next = gamemap; // go to next level
    }

    st.wminfo.maxkills = st.totalkills;
    st.wminfo.maxitems = st.totalitems;
    st.wminfo.maxsecret = st.totalsecret;
    st.wminfo.maxfrags = 0;
    st.wminfo.partime = if gamemode == GameMode::Commercial {
        35 * CPARS[(gamemap - 1) as usize]
    } else {
        35 * PARS[gameepisode as usize][gamemap as usize]
    };
    st.wminfo.pnum = console as i32;

    let leveltime = st.leveltime;
    for (i, p) in world.players.iter().enumerate() {
        st.wminfo.plyr[i].in_game = doomstat::state().playeringame[i];
        st.wminfo.plyr[i].skills = p.killcount;
        st.wminfo.plyr[i].sitems = p.itemcount;
        st.wminfo.plyr[i].ssecret = p.secretcount;
        st.wminfo.plyr[i].stime = leveltime;
        st.wminfo.plyr[i].frags = p.frags;
    }

    st.gamestate = GameStateEnum::Intermission;
    st.viewactive = false;
    st.automapactive = false;

    let wminfo = st.wminfo;
    let ingame = world.playeringame();
    crate::wi_stuff::wi_start(&wminfo, &ingame, &mut world.wad, video);
}

/// Port of `G_WorldDone` (what `WI_updateNoState` calls when the
/// intermission ends): queues `ga_worlddone`, and starts the finale
/// after DOOM II's episode-ending maps.
pub fn g_world_done(world: &mut GameWorld) {
    doomstat::state_mut().gameaction = crate::d_event::GameAction::WorldDone;
    let st = doomstat::state();
    let (secretexit, gamemode, gamemap, console) = (
        st.secretexit,
        st.gamemode,
        st.gamemap,
        st.consoleplayer as usize,
    );
    if secretexit {
        if let Some(p) = world.players.get_mut(console) {
            p.didsecret = true;
        }
    }

    if gamemode == GameMode::Commercial {
        match gamemap {
            15 | 31 => {
                if secretexit {
                    crate::f_finale::f_start_finale(&mut world.wad);
                }
            }
            6 | 11 | 20 | 30 => crate::f_finale::f_start_finale(&mut world.wad),
            _ => {}
        }
    }
}

/// Port of `G_DoWorldDone`.
pub fn g_do_world_done(world: &mut GameWorld) {
    let st = doomstat::state_mut();
    st.gamestate = GameStateEnum::Level;
    st.gamemap = st.wminfo.next + 1;
    g_do_load_level(world);
    let st = doomstat::state_mut();
    st.gameaction = crate::d_event::GameAction::Nothing;
    st.viewactive = true;
}

/// Port of `G_DeferedInitNew`. Can be called by the startup code or
/// the menu task; the new game starts at the next tic boundary once
/// `G_Ticker` handles [`crate::d_event::GameAction::NewGame`] (Phase
/// 9h) — until then this just records the request, like
/// [`g_exit_level`].
pub fn g_defered_init_new(skill: Skill, episode: i32, map: i32) {
    let state = doomstat::state_mut();
    state.d_skill = skill;
    state.d_episode = episode;
    state.d_map = map;
    state.gameaction = crate::d_event::GameAction::NewGame;
}

/// Port of `G_LoadGame` (the request half): records the file to load
/// and `gameaction = ga_loadgame`; `G_Ticker` runs [`g_do_load_game`].
pub fn g_load_game(name: &str) {
    let state = doomstat::state_mut();
    state.savename = name.to_string();
    state.gameaction = crate::d_event::GameAction::LoadGame;
}

/// Port of `G_SaveGame` (the request half): records the slot and
/// description and sets `sendsave`; `G_BuildTiccmd` turns that into a
/// `BTS_SAVEGAME` special button and `G_Ticker` then runs
/// [`g_do_save_game`] (like the original, the action isn't set here).
pub fn g_save_game(slot: i32, description: &str) {
    let state = doomstat::state_mut();
    state.savegameslot = slot;
    state.savedescription = description.to_string();
    state.sendsave = true;
}

/// (`VERSION`)
pub const VERSION: i32 = 110;
/// (`VERSIONSIZE`)
const VERSIONSIZE: usize = 16;
/// (`SAVESTRINGSIZE`)
const SAVESTRINGSIZE: usize = 24;
/// (`consistancy marker`, the last byte of a savegame)
const SAVE_MARKER: u8 = 0x1d;

fn version_string() -> [u8; VERSIONSIZE] {
    let mut v = [0u8; VERSIONSIZE];
    let text = format!("version {VERSION}");
    v[..text.len()].copy_from_slice(text.as_bytes());
    v
}

/// The savegame file name for `slot` (`SAVEGAMENAME"%d.dsg"`).
pub fn save_game_name(slot: i32) -> String {
    format!("{}{}.dsg", crate::d_englsh::SAVEGAMENAME, slot)
}

/// Serializes the running game (the body of `G_DoSaveGame`, without the
/// file write): header, players, world, thinkers, specials, marker.
pub fn g_save_game_bytes(world: &GameWorld, description: &str) -> Vec<u8> {
    use crate::doomdef::MAXPLAYERS;
    use crate::p_saveg::*;
    let st = doomstat::state();
    let mut w = SaveWriter::default();

    let mut desc = [0u8; SAVESTRINGSIZE];
    let d = description.as_bytes();
    let n = d.len().min(SAVESTRINGSIZE);
    desc[..n].copy_from_slice(&d[..n]);
    w.bytes(&desc);
    w.bytes(&version_string());

    w.u8(st.gameskill as u8);
    w.u8(st.gameepisode as u8);
    w.u8(st.gamemap as u8);
    let playeringame = world.playeringame();
    for i in 0..MAXPLAYERS as usize {
        w.bool(playeringame.get(i).copied().unwrap_or(false));
    }
    w.u8((st.leveltime >> 16) as u8);
    w.u8((st.leveltime >> 8) as u8);
    w.u8(st.leveltime as u8);

    p_archive_players(&mut w, &world.players, &playeringame);
    p_archive_world(&mut w, &world.level);
    p_archive_thinkers(&mut w, &world.thinkers);
    p_archive_specials(&mut w, &world.thinkers, &world.active_ceilings);

    w.u8(SAVE_MARKER); // consistancy marker
    w.buf
}

/// Port of `G_DoSaveGame`.
pub fn g_do_save_game(world: &mut GameWorld, video: &mut crate::v_video::VVideo) {
    let (slot, description) = {
        let st = doomstat::state();
        (st.savegameslot, st.savedescription.clone())
    };
    let name = save_game_name(slot);
    let bytes = g_save_game_bytes(world, &description);
    if !crate::m_misc::m_write_file(&name, &bytes) {
        eprintln!("G_DoSaveGame: couldn't write {name}");
    }
    let st = doomstat::state_mut();
    st.gameaction = crate::d_event::GameAction::Nothing;
    st.savedescription.clear();

    if let Some(p) = world.players.get_mut(st.consoleplayer as usize) {
        p.message = Some(crate::d_englsh::GGSAVED);
    }

    // draw the pattern into the back screen
    let gamemode = st.gamemode;
    world
        .renderer
        .rdraw
        .r_fill_back_screen(video, &mut world.wad, gamemode);
}

/// Loads a savegame from `data` (the body of `G_DoLoadGame`). A wrong
/// version is ignored silently, like the original; malformed data is an
/// error (the original's `I_Error("Bad savegame")`).
pub fn g_load_game_bytes(
    world: &mut GameWorld,
    data: &[u8],
) -> Result<(), crate::p_saveg::SaveError> {
    use crate::doomdef::MAXPLAYERS;
    use crate::p_saveg::*;
    let mut r = SaveReader::new(data);
    r.bytes(SAVESTRINGSIZE)?; // skip the description field
    if r.bytes(VERSIONSIZE)? != version_string() {
        return Ok(()); // bad version
    }

    let skill = r.u8()?;
    let episode = r.u8()?;
    let map = r.u8()?;
    let mut playeringame = [false; MAXPLAYERS as usize];
    for slot in playeringame.iter_mut() {
        *slot = r.bool()?;
    }
    if playeringame != [true, false, false, false] {
        return Err(SaveError(
            "savegame isn't a single-player game (multiplayer is Phase 11)".into(),
        ));
    }
    let skill = Skill::from_index(skill as usize)
        .ok_or_else(|| SaveError(format!("bad skill {skill} in savegame")))?;

    // load a base level
    g_init_new(world, skill, episode as i32, map as i32);

    // get the times
    let (a, b, c) = (r.u8()? as i32, r.u8()? as i32, r.u8()? as i32);
    doomstat::state_mut().leveltime = (a << 16) + (b << 8) + c;

    // dearchive all the modifications
    let playeringame = world.playeringame();
    p_unarchive_players(&mut r, &mut world.players, &playeringame)?;
    p_unarchive_world(&mut r, &mut world.level)?;
    p_unarchive_thinkers(
        &mut r,
        &mut world.thinkers,
        &mut world.level,
        &mut world.players,
    )?;
    p_unarchive_specials(
        &mut r,
        &mut world.thinkers,
        &mut world.level,
        &mut world.active_plats,
        &mut world.active_ceilings,
    )?;

    if r.u8()? != SAVE_MARKER {
        return Err(SaveError("Bad savegame".into()));
    }
    Ok(())
}

/// Port of `G_DoLoadGame`.
pub fn g_do_load_game(world: &mut GameWorld, video: &mut crate::v_video::VVideo) {
    let name = doomstat::state().savename.clone();
    doomstat::state_mut().gameaction = crate::d_event::GameAction::Nothing;

    let data = match std::fs::read(&name) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("G_DoLoadGame: couldn't read {name}: {e}");
            return;
        }
    };
    if let Err(e) = g_load_game_bytes(world, &data) {
        eprintln!("G_DoLoadGame: {e}");
        return;
    }

    // draw the pattern into the back screen
    let gamemode = doomstat::state().gamemode;
    world
        .renderer
        .rdraw
        .r_fill_back_screen(video, &mut world.wad, gamemode);
}

/// Port of `G_ExitLevel`. Sets `secretexit = false` and
/// `gameaction = ga_completed` (the original's own body, preserved) —
/// consumed by `G_DoCompleted` (Phase 9, not ported yet), so this just
/// records the request faithfully.
pub fn g_exit_level() {
    let state = doomstat::state_mut();
    state.secretexit = false;
    state.gameaction = crate::d_event::GameAction::Completed;
}

/// Port of `G_SecretExitLevel`. Here's for the german edition (the
/// original's own comment, preserved). IF NO WOLF3D LEVELS, NO SECRET
/// EXIT! (the original's own comment, preserved).
pub fn g_secret_exit_level(wad: &WadFiles, gamemode: crate::doomdef::GameMode) {
    let state = doomstat::state_mut();
    state.secretexit = !(gamemode == crate::doomdef::GameMode::Commercial
        && wad.check_num_for_name("map31").is_none());
    state.gameaction = crate::d_event::GameAction::Completed;
}

/// Port of `G_Ticker`: performs the pending game action (level
/// load, new game, level completion, ...), takes this tic's command,
/// handles pause, and runs the current game state's ticker. `cmd` is
/// player 0's ticcmd for this tic (the original reads it from
/// `netcmds`). Save/load and demo actions (Phase 10 / not ported) are
/// dropped with a message.
pub fn g_ticker(world: &mut GameWorld, video: &mut crate::v_video::VVideo, cmd: TicCmd) {
    let mut cmds = [TicCmd::default(); crate::doomdef::MAXPLAYERS as usize];
    cmds[doomstat::state().consoleplayer as usize] = cmd;
    g_ticker_cmds(world, video, &cmds);
}

/// [`g_ticker`] with every player's ticcmd for this tic (the original's
/// `netcmds[i][buf]`): `cmds[i]` is read for each player in the game.
pub fn g_ticker_cmds(
    world: &mut GameWorld,
    video: &mut crate::v_video::VVideo,
    cmds: &[TicCmd; crate::doomdef::MAXPLAYERS as usize],
) {
    use crate::d_event::GameAction;

    // do player reborns if needed
    let ingame_now = doomstat::state().playeringame;
    for i in 0..world.players.len() {
        if ingame_now.get(i).copied().unwrap_or(false)
            && world.players[i].playerstate == PlayerState::Reborn
        {
            g_do_reborn(world, i);
        }
    }

    // do things to change the game state
    loop {
        let action = doomstat::state().gameaction;
        match action {
            GameAction::Nothing => break,
            GameAction::LoadLevel => g_do_load_level(world),
            GameAction::NewGame => g_do_new_game(world),
            GameAction::Completed => g_do_completed(world, video),
            GameAction::Victory => crate::f_finale::f_start_finale(&mut world.wad),
            GameAction::WorldDone => g_do_world_done(world),
            GameAction::Screenshot => {
                doomstat::state_mut().gameaction = GameAction::Nothing;
            }
            GameAction::LoadGame => g_do_load_game(world, video),
            GameAction::SaveGame => g_do_save_game(world, video),
            GameAction::PlayDemo => {
                // Demos aren't ported.
                eprintln!("G_Ticker: {action:?} isn't supported");
                doomstat::state_mut().gameaction = GameAction::Nothing;
            }
        }
    }

    // read each player's ticcmd, then check for special buttons (the
    // original's own comments, merged)
    let ingame = doomstat::state().playeringame;
    let mut turbo: Option<usize> = None;
    for (i, player) in world.players.iter_mut().enumerate() {
        if !ingame.get(i).copied().unwrap_or(false) {
            continue;
        }
        player.cmd = cmds[i];

        // check for turbo cheats (the original's own comment, preserved)
        let gametic = doomstat::state().gametic;
        if player.cmd.forwardmove > TURBOTHRESHOLD
            && gametic & 31 == 0
            && (gametic >> 5) & 3 == i as i32
        {
            turbo = Some(i);
        }

        // check consistancy, and build new consistancy check (the
        // original's own comment, preserved)
        let ticdup = doomstat::state().ticdup.max(1);
        if world.consistency_check && doomstat::state().netgame && gametic % ticdup == 0 {
            let buf = ((gametic / ticdup) as usize) % crate::d_net::BACKUPTICS;
            if gametic > crate::d_net::BACKUPTICS as i32
                && world.consistancy[i][buf] != player.cmd.consistancy
            {
                panic!(
                    "consistency failure ({} should be {})",
                    player.cmd.consistancy, world.consistancy[i][buf]
                );
            }
            world.consistancy[i][buf] = match world.thinkers.mobj(player.mo) {
                Some(m) => m.x as i16,
                None => crate::m_random::rnd_index() as i16,
            };
        }

        if player.cmd.buttons & (BT_SPECIAL as u8) != 0
            && player.cmd.buttons & (crate::d_event::BT_SPECIALMASK as u8)
                == crate::d_event::BTS_SAVEGAME as u8
        {
            let st = doomstat::state_mut();
            if st.savedescription.is_empty() {
                st.savedescription = "NET GAME".to_string();
            }
            st.savegameslot = (player.cmd.buttons as i32 & crate::d_event::BTS_SAVEMASK)
                >> crate::d_event::BTS_SAVESHIFT;
            st.gameaction = GameAction::SaveGame;
        } else if player.cmd.buttons & (BT_SPECIAL as u8) != 0
            && player.cmd.buttons & (crate::d_event::BT_SPECIALMASK as u8) == BTS_PAUSE as u8
        {
            let state = doomstat::state_mut();
            state.paused = !state.paused;
            if state.paused {
                crate::s_sound::s_pause_sound();
            } else {
                crate::s_sound::s_resume_sound();
            }
        }
    }

    if let Some(i) = turbo {
        let console = doomstat::state().consoleplayer as usize;
        if let Some(p) = world.players.get_mut(console) {
            p.message = Some(match i {
                0 => "Green is turbo!",
                1 => "Indigo is turbo!",
                2 => "Brown is turbo!",
                _ => "Red is turbo!",
            });
        }
    }

    // do main actions
    match doomstat::state().gamestate {
        GameStateEnum::Level => {
            let playeringame = world.playeringame();
            let st = doomstat::state();
            let (paused, menuactive, netgame, demoplayback) =
                (st.paused, st.menuactive, st.netgame, st.demoplayback);
            p_ticker(
                &mut world.thinkers,
                &mut world.level,
                &mut world.renderer.rmain,
                world.validcount,
                &mut world.players,
                &playeringame,
                paused,
                menuactive,
                netgame,
                demoplayback,
                &mut world.specials,
                &mut world.renderer.rdata,
                &world.wad,
                &mut world.switches,
                &mut world.active_plats,
                &mut world.active_ceilings,
                &mut world.brain,
            );
            let display = doomstat::state().displayplayer as usize;
            if let Some(player) = world.players.get(display) {
                // ST_Ticker (after P_Ticker, as in G_Ticker).
                if crate::st_stuff::st_is_initialized() {
                    crate::st_stuff::st_ticker(player, &world.thinkers);
                }
                // AM_Ticker
                if let Some(mo) = world.thinkers.mobj(player.mo) {
                    crate::am_map::am_ticker((mo.x, mo.y));
                }
            }
            // HU_Ticker
            if crate::hu_stuff::hu_is_initialized() {
                crate::hu_stuff::hu_ticker(&mut world.players, &playeringame);
            }
        }
        GameStateEnum::Intermission => {
            if crate::wi_stuff::wi_is_active()
                && crate::wi_stuff::wi_ticker(&mut world.players, &mut world.wad)
                    == crate::wi_stuff::WiEvent::WorldDone
            {
                crate::wi_stuff::wi_end();
                g_world_done(world);
            }
        }
        GameStateEnum::Finale => crate::f_finale::f_ticker(&world.players, &mut world.wad),
        GameStateEnum::DemoScreen => {
            crate::d_main::d_page_ticker();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_ticcmd_forward_key_moves_forward() {
        let mut controls = Controls::default();
        controls.gamekeydown[KEY_UP as usize] = true;
        let cmd = g_build_ticcmd(&mut controls, 1);
        assert!(cmd.forwardmove > 0);
        assert_eq!(cmd.sidemove, 0);
    }

    #[test]
    fn build_ticcmd_right_key_turns() {
        let mut controls = Controls::default();
        controls.gamekeydown[KEY_RIGHT as usize] = true;
        let cmd = g_build_ticcmd(&mut controls, 1);
        assert_ne!(cmd.angleturn, 0);
    }

    #[test]
    fn build_ticcmd_strafe_and_right_moves_sideways_not_turns() {
        let mut controls = Controls::default();
        controls.gamekeydown[KEY_STRAFE as usize] = true;
        controls.gamekeydown[KEY_RIGHT as usize] = true;
        let cmd = g_build_ticcmd(&mut controls, 1);
        assert_ne!(cmd.sidemove, 0);
        assert_eq!(cmd.angleturn, 0);
    }

    #[test]
    fn responder_keydown_sets_gamekeydown_and_eats_event() {
        let mut controls = Controls::default();
        let ev = Event {
            event_type: EvType::KeyDown,
            data1: KEY_UP,
            data2: 0,
            data3: 0,
        };
        let eaten = g_responder(&mut controls, &ev);
        assert!(eaten);
        assert!(controls.gamekeydown[KEY_UP as usize]);
    }

    #[test]
    fn responder_keyup_clears_gamekeydown_and_does_not_eat_event() {
        let mut controls = Controls::default();
        controls.gamekeydown[KEY_UP as usize] = true;
        let ev = Event {
            event_type: EvType::KeyUp,
            data1: KEY_UP,
            data2: 0,
            data3: 0,
        };
        let eaten = g_responder(&mut controls, &ev);
        assert!(!eaten);
        assert!(!controls.gamekeydown[KEY_UP as usize]);
    }

    #[test]
    fn responder_pause_key_sets_sendpause() {
        let mut controls = Controls::default();
        let ev = Event {
            event_type: EvType::KeyDown,
            data1: KEY_PAUSE,
            data2: 0,
            data3: 0,
        };
        g_responder(&mut controls, &ev);
        assert!(controls.sendpause);
    }
}
