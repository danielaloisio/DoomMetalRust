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
//	Put all global tate variables here.
//
//-----------------------------------------------------------------------------

//! Rust port of `doomstat.h` / `doomstat.c` (partial — see note below).
//!
//! All the global variables that store the internal state. Theoretically
//! speaking, the internal state of the engine should be found by looking
//! at the variables collected here, and every relevant module will have
//! to include this header file. In practice, things are a bit messy (the
//! original's own words).
//!
//! # Scope
//!
//! Every field from `doomstat.h` is ported here **except** `players:
//! [player_t; MAXPLAYERS]` — `player_t` embeds `mobj_t*`/`pspdef_t`,
//! which are Phase 5/6 world-data-structure work per the plan, not yet
//! ported. All 72 other fields only depend on primitives or already
//! ported types (`d_player`, `d_net`, `d_ticcmd`, `doomdata`, `doomdef`),
//! so they're ported now, matching the plan's Phase 2 scope decision.
//!
//! # Globals strategy
//!
//! Per the plan: a single `static` holding a `GameState` struct (field
//! names/order mirroring the original header), behind a `OnceLock` +
//! `UnsafeCell` for interior mutability, with all `unsafe` confined to
//! [`state`]/[`state_mut`] rather than scattered across call sites. Doom
//! is single-threaded — this is not about real concurrency, just the
//! lowest-ceremony way to get a mutable global in safe-callable Rust
//! without `static mut`.
//!
//! A couple of representation notes versus the raw C types:
//! - `debugfile: FILE*` becomes `Option<std::fs::File>` (`None` ==
//!   NULL — the original only ever opens this when `-debugfile` is
//!   passed).
//! - `deathmatch_p: mapthing_t*` (a pointer *into* the
//!   `deathmatchstarts` array, used as a "next free slot" cursor)
//!   becomes `Option<usize>`, an index into that same array, since a raw
//!   pointer into another struct field has no safe Rust equivalent and
//!   an index expresses the actual intent (a cursor, not an arbitrary
//!   pointer) more directly.
//! - `doomcom`/`netbuffer` (`doomcom_t*`/`doomdata_t*`, the latter
//!   pointing *inside* the former's `data` field in the original) become
//!   `Option<DoomCom>` and a `netbuffer()` accessor returning
//!   `Option<&DoomData>` borrowed from it, preserving "netbuffer points
//!   inside doomcom" without a raw pointer.

use std::fs::File;
use std::sync::OnceLock;

use crate::d_net::{DoomCom, DoomData, BACKUPTICS, MAXNETNODES};
use crate::d_player::WbStartStruct;
use crate::d_ticcmd::TicCmd;
use crate::doomdata::MapThing;
use crate::doomdef::{
    GameMission, GameMode, GameState as GameStateEnum, Language, Skill, MAXPLAYERS, NUMAMMO,
};
use crate::global_cell::GlobalCell;

/// Player spawn spots for deathmatch (`MAX_DM_STARTS`).
pub const MAX_DM_STARTS: usize = 10;

/// Port of the `doomstat.h` global state block (see module docs for the
/// one field intentionally not ported yet: `players`).
pub struct GameState {
    // ------------------------
    // Command line parameters.
    /// checkparm of -nomonsters
    pub nomonsters: bool,
    /// checkparm of -respawn
    pub respawnparm: bool,
    /// checkparm of -fast
    pub fastparm: bool,
    /// DEBUG: launched with -devparm
    pub devparm: bool,

    // -----------------------------------------------------
    // Game Mode - identify IWAD as shareware, retail etc.
    pub gamemode: GameMode,
    pub gamemission: GameMission,
    /// Set if homebrew PWAD stuff has been added.
    pub modifiedgame: bool,

    // -------------------------------------------
    // Language.
    pub language: Language,

    // -------------------------------------------
    // Selected skill type, map etc.
    // Defaults for menu, methinks.
    pub startskill: Skill,
    pub startepisode: i32,
    pub startmap: i32,
    pub autostart: bool,

    // Selected by user.
    pub gameskill: Skill,
    pub gameepisode: i32,
    pub gamemap: i32,

    /// Nightmare mode flag, single player.
    pub respawnmonsters: bool,

    /// Netgame? Only true if >1 player.
    pub netgame: bool,

    /// Flag: true only if started as net deathmatch. An enum might
    /// handle altdeath/cooperative better (the original's own comment).
    pub deathmatch: bool,
    /// `deathmatch == 2` (`-altdeath`): items respawn, weapons don't
    /// stay. Only meaningful with `deathmatch`.
    pub altdeath: bool,

    // -------------------------
    // Internal parameters for sound rendering.
    /// Maximum volume for sound.
    pub snd_sfx_volume: i32,
    /// Maximum volume for music.
    pub snd_music_volume: i32,
    pub snd_music_device: i32,
    pub snd_sfx_device: i32,
    pub snd_desired_music_device: i32,
    pub snd_desired_sfx_device: i32,
    /// Number of internal mixing channels (`numChannels`, defined in
    /// `s_sound.c`; `m_misc.c`'s `snd_channels` config default is 3).
    pub num_channels: i32,
    /// (`showMessages`, `m_menu.c`) Show messages has default, 0 = off,
    /// 1 = on (the original's own comment); the menu (Phase 9d) toggles it.
    pub show_messages: i32,
    /// (`screenblocks`, `m_menu.c`) The screen size setting: 3..=11 (11 =
    /// full screen without the status bar).
    pub screenblocks: i32,
    /// (`detailLevel`, `m_menu.c`) 0 = high, 1 = low (unsupported: the
    /// original's own menu just reports "low detail mode n.a.").
    pub detail_level: i32,
    /// Set by the menu's quit confirmation (`I_Quit`); the main loop
    /// exits when it sees it.
    pub quit_requested: bool,
    /// (`french`) The French-language game: `hu_stuff` then uses the
    /// French shift table/keymap. Set by `IdentifyVersion` (Phase 9h).
    pub french: bool,

    // -------------------------
    // Status flags for refresh.
    pub statusbaractive: bool,
    /// In AutoMap mode?
    pub automapactive: bool,
    /// Menu overlayed?
    pub menuactive: bool,
    /// Game Pause?
    pub paused: bool,

    pub viewactive: bool,
    pub nodrawers: bool,
    pub noblit: bool,

    pub viewwindowx: i32,
    pub viewwindowy: i32,
    pub viewheight: i32,
    pub viewwidth: i32,
    pub scaledviewwidth: i32,

    /// This one is related to the 3-screen display mode. ANG90 = left
    /// side, ANG270 = right.
    pub viewangleoffset: i32,

    /// Player taking events, and displaying.
    pub consoleplayer: i32,
    pub displayplayer: i32,

    // -------------------------------------
    // Scores, rating. Statistics on a given map, for intermission.
    pub totalkills: i32,
    pub totalitems: i32,
    pub totalsecret: i32,

    /// gametic at level start
    pub levelstarttic: i32,
    /// tics in game play for par
    pub leveltime: i32,

    // --------------------------------------
    // DEMO playback/recording related stuff.
    /// No demo, there is a human player in charge?  Disable save/end
    /// game?
    pub usergame: bool,

    pub demoplayback: bool,
    pub demorecording: bool,

    /// Quit after playing a demo from cmdline.
    pub singledemo: bool,

    pub gamestate: GameStateEnum,

    /// (`gameaction_t gameaction`) What [`crate::g_game::g_ticker`]
    /// should do at the top of the next tic — Phase 7b4 adds
    /// [`crate::g_game::g_exit_level`]/[`crate::g_game::g_secret_exit_level`],
    /// which set this to [`crate::d_event::GameAction::Completed`]; the
    /// intermission/finale handling that consumes it (`G_DoCompleted`)
    /// is still Phase 9, so nothing reads this back out yet.
    pub gameaction: crate::d_event::GameAction,

    /// (`d_skill`/`d_episode`/`d_map`) The parameters of the pending
    /// [`crate::d_event::GameAction::NewGame`], set by
    /// [`crate::g_game::g_defered_init_new`].
    pub d_skill: Skill,
    pub d_episode: i32,
    pub d_map: i32,
    /// (`savename`/`savegameslot`/`savedescription`/`sendsave`) The
    /// pending load/save request (Phase 10 consumes them).
    pub savename: String,
    pub savegameslot: i32,
    pub savedescription: String,
    pub sendsave: bool,

    /// (`boolean secretexit`) Set by
    /// [`crate::g_game::g_secret_exit_level`] — read by `G_DoCompleted`
    /// (Phase 9, not ported yet).
    pub secretexit: bool,

    //-----------------------------
    // Internal parameters, fixed.
    pub gametic: i32,

    // Bookkeeping on players - state.
    // `players: [player_t; MAXPLAYERS]` intentionally NOT ported yet —
    // see module docs.
    /// Alive? Disconnected?
    pub playeringame: [bool; MAXPLAYERS as usize],

    /// Player spawn spots for deathmatch.
    pub deathmatchstarts: [MapThing; MAX_DM_STARTS],
    /// Index into `deathmatchstarts` of the next free slot — see module
    /// docs for why this is an index rather than the original's
    /// `mapthing_t*`.
    pub deathmatch_p: Option<usize>,

    /// Player spawn spots.
    pub playerstarts: [MapThing; MAXPLAYERS as usize],

    /// Intermission stats. Parameters for world map / intermission.
    pub wminfo: WbStartStruct,

    /// LUT of ammunition limits for each kind. This doubles with
    /// BackPack powerup item.
    pub maxammo: [i32; NUMAMMO],

    //-----------------------------------------
    // Internal parameters, used for engine.
    // File handling stuff.
    pub basedefault: String,
    pub debugfile: Option<File>,

    /// If true, load all graphics at level load.
    pub precache: bool,

    /// wipegamestate can be set to -1 to force a wipe on the next draw.
    /// The original stores this as `gamestate_t` but relies on it also
    /// holding the sentinel value -1 (outside `gamestate_t`'s actual
    /// range) to mean "force wipe" — `Option<GameStateEnum>` makes that
    /// sentinel explicit instead of overloading the enum's range.
    pub wipegamestate: Option<GameStateEnum>,

    pub mouse_sensitivity: i32,

    /// Debug flag to cancel adaptiveness.
    pub singletics: bool,

    pub bodyqueslot: i32,

    /// Needed to store the number of the dummy sky flat. Used for
    /// rendering, as well as tracking projectiles etc.
    pub skyflatnum: i32,

    // Netgame stuff (buffers and pointers, i.e. indices).
    /// This is ???
    pub doomcom: Option<DoomCom>,

    pub localcmds: [TicCmd; BACKUPTICS],
    pub rndindex: i32,

    pub maketic: i32,
    pub nettics: [i32; MAXNETNODES],

    pub netcmds: [[TicCmd; BACKUPTICS]; MAXPLAYERS as usize],
    pub ticdup: i32,
}

impl GameState {
    /// This points inside doomcom (`netbuffer`) — the original's
    /// `netbuffer` is a raw pointer aliasing `doomcom->data`; here it's
    /// just a borrow of the same field, see module docs.
    pub fn netbuffer(&self) -> Option<&DoomData> {
        self.doomcom.as_ref().map(|c| &c.data)
    }

    /// Mutable variant of [`GameState::netbuffer`].
    pub fn netbuffer_mut(&mut self) -> Option<&mut DoomData> {
        self.doomcom.as_mut().map(|c| &mut c.data)
    }
}

impl Default for GameState {
    fn default() -> Self {
        GameState {
            nomonsters: false,
            respawnparm: false,
            fastparm: false,
            devparm: false,

            // Port of doomstat.c's explicit initializers.
            gamemode: GameMode::Indetermined,
            gamemission: GameMission::Doom,
            modifiedgame: false,

            language: Language::English,

            startskill: Skill::Baby,
            startepisode: 0,
            startmap: 0,
            autostart: false,

            gameskill: Skill::Baby,
            gameepisode: 0,
            gamemap: 0,

            respawnmonsters: false,
            netgame: false,
            altdeath: false,
            deathmatch: false,

            // s_sound.c's initializers (`snd_SfxVolume = 15`, ...).
            snd_sfx_volume: 15,
            snd_music_volume: 15,
            snd_music_device: 0,
            snd_sfx_device: 0,
            snd_desired_music_device: 0,
            snd_desired_sfx_device: 0,
            num_channels: 3,
            show_messages: 1,
            screenblocks: 9,
            detail_level: 0,
            quit_requested: false,
            french: false,

            statusbaractive: false,
            automapactive: false,
            menuactive: false,
            paused: false,

            viewactive: false,
            nodrawers: false,
            noblit: false,

            viewwindowx: 0,
            viewwindowy: 0,
            viewheight: 0,
            viewwidth: 0,
            scaledviewwidth: 0,

            viewangleoffset: 0,

            consoleplayer: 0,
            displayplayer: 0,

            totalkills: 0,
            totalitems: 0,
            totalsecret: 0,

            levelstarttic: 0,
            leveltime: 0,

            usergame: false,
            demoplayback: false,
            demorecording: false,
            singledemo: false,

            gamestate: GameStateEnum::DemoScreen,
            gameaction: crate::d_event::GameAction::Nothing,
            d_skill: Skill::Baby,
            d_episode: 0,
            d_map: 0,
            savename: String::new(),
            savegameslot: 0,
            savedescription: String::new(),
            sendsave: false,
            secretexit: false,

            gametic: 0,

            playeringame: [false; MAXPLAYERS as usize],

            deathmatchstarts: [MapThing::default(); MAX_DM_STARTS],
            deathmatch_p: None,

            playerstarts: [MapThing::default(); MAXPLAYERS as usize],

            wminfo: WbStartStruct::default(),

            maxammo: [0; NUMAMMO],

            basedefault: String::new(),
            debugfile: None,

            precache: true,

            // `gamestate_t wipegamestate = GS_DEMOSCREEN;` (d_main.c)
            wipegamestate: Some(GameStateEnum::DemoScreen),

            mouse_sensitivity: 0,
            singletics: false,
            bodyqueslot: 0,

            skyflatnum: 0,

            doomcom: None,

            localcmds: [TicCmd::default(); BACKUPTICS],
            rndindex: 0,

            maketic: 0,
            nettics: [0; MAXNETNODES],

            netcmds: [[TicCmd::default(); BACKUPTICS]; MAXPLAYERS as usize],
            ticdup: 0,
        }
    }
}

static STATE: OnceLock<GlobalCell<GameState>> = OnceLock::new();

fn cell() -> &'static GlobalCell<GameState> {
    STATE.get_or_init(|| GlobalCell::new(GameState::default()))
}

/// Shared read access to the global game state. Single-threaded engine —
/// see [`GlobalCell`]'s docs for why this is safe despite the `unsafe`
/// block.
pub fn state() -> &'static GameState {
    unsafe { cell().get() }
}

/// Exclusive/mutable access to the global game state.
pub fn state_mut() -> &'static mut GameState {
    unsafe { cell().get_mut() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The global GameState is process-wide; serialize tests that mutate
    // it so they don't interfere with each other.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_match_doomstat_c_initializers() {
        let _guard = TEST_LOCK.lock().unwrap();
        let s = GameState::default();
        // doomstat.c explicitly initializes these three (everything else
        // is C's implicit static zero-init, which Default::default()
        // mirrors field-by-field above).
        assert_eq!(s.gamemode, GameMode::Indetermined);
        assert_eq!(s.gamemission, GameMission::Doom);
        assert_eq!(s.language, Language::English);
        assert!(!s.modifiedgame);
    }

    #[test]
    fn global_state_accessors_share_one_instance() {
        let _guard = TEST_LOCK.lock().unwrap();
        state_mut().gametic = 42;
        assert_eq!(state().gametic, 42);
        // reset for other tests sharing the process-global state
        state_mut().gametic = 0;
    }

    #[test]
    fn netbuffer_borrows_doomcom_data() {
        let _guard = TEST_LOCK.lock().unwrap();
        let gs = state_mut();
        assert!(gs.netbuffer().is_none());

        gs.doomcom = Some(DoomCom {
            id: 0,
            intnum: 0,
            command: 0,
            remotenode: 0,
            datalength: 0,
            numnodes: 0,
            ticdup: 0,
            extratics: 0,
            deathmatch: 0,
            savegame: 0,
            episode: 0,
            map: 0,
            skill: 0,
            consoleplayer: 0,
            numplayers: 0,
            angleoffset: 0,
            drone: 0,
            data: DoomData {
                checksum: 0,
                retransmitfrom: 0,
                starttic: 0,
                player: 0,
                numtics: 0,
                cmds: [TicCmd::default(); BACKUPTICS],
            },
        });

        assert!(gs.netbuffer().is_some());
        gs.doomcom = None; // reset for other tests
    }
}
