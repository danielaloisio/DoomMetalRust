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
//	DOOM main program (D_DoomMain) and game loop (D_DoomLoop),
//	plus functions to determine game mode (shareware, registered),
//	parse command line parameters, configure game parameters (turbo),
//	and call the startup functions.
//
//-----------------------------------------------------------------------------

//! Rust port of `d_main.h` / `d_main.c`: the event responder chain
//! ([`d_process_events`]), the display ([`d_display_world`]/
//! [`d_display`], with the wipe between game states), the title/credit
//! page loop ([`d_start_title`], [`d_advance_demo`],
//! [`d_do_advance_demo`], [`d_page_ticker`], [`d_page_drawer`]) and the
//! startup identification helpers ([`identify_version`], [`StartArgs`]).
//!
//! `D_DoomLoop`/`TryRunTics` themselves aren't ported as functions
//! here — the loop driver lives in `main.rs` instead (a `fn` wrapping
//! an infinite `loop` doesn't gain anything from being wrapped again
//! here just to mirror the original's file layout, and `main.rs`
//! needs to own the top-level `IVideo`/`GameWorld` anyway as the
//! binary's actual entry point). See `main.rs` for the loop, and its
//! module docs for which `D_DoomLoop` path it follows (`singletics`,
//! not `TryRunTics` — a real, documented behavioral difference from
//! the original's default netgame-ready pacing).
//!
//! # Not ported
//!
//! * Demo playback/recording: the attract loop's `demo1`..`demo4`
//!   entries (`G_DeferedPlayDemo`) are skipped — the sequence goes
//!   title → credits → (title/help) → title.
//! * Network setup (`D_CheckNetGame`), `-file`/PWAD handling, response
//!   files, `-devparm` data directories, `D_DoomMain`'s banners.
//! * `IdentifyVersion` picks the game mode from the *lumps* (`MAP01`,
//!   `E4M1`, `E3M1`, `E1M1`) instead of the IWAD's file name, so the
//!   Ultimate DOOM `doom.wad` is `retail` (the original's file-name
//!   check would call it `registered` and lock episode 4).
//! * Save/load — Phase 10. (`S_UpdateSounds` is called from
//!   `main.rs`'s loop right after `G_Ticker`, as in `D_DoomLoop`;
//!   `I_UpdateSound`/`I_SubmitSound` are empty in the original.)

use crate::d_event::Event;
use crate::d_net::BACKUPTICS;
use crate::d_player::Player;
use crate::doomdef::GameState as GameStateEnum;
use crate::doomdef::{SCREENHEIGHT, SCREENWIDTH};
use crate::doomstat;
use crate::g_game::{self, GameWorld};
use crate::i_video::IVideo;
use crate::p_setup::Level;
use crate::p_tick::Thinkers;
use crate::r_main::{Renderer, ViewPlayer};
use crate::r_things::{PSprite, PlayerSprites};
use crate::w_wad::WadFiles;

/// `demosequence`/`pagetic`/`pagename`/`advancedemo`.
#[derive(Debug, Clone, Copy)]
struct PageState {
    demosequence: i32,
    pagetic: i32,
    pagename: &'static str,
    advancedemo: bool,
}

thread_local! {
    static PAGES: std::cell::Cell<PageState> = const {
        std::cell::Cell::new(PageState {
            demosequence: 0,
            pagetic: 0,
            pagename: "TITLEPIC",
            advancedemo: false,
        })
    };
}

/// Port of `D_PageTicker`. Handles timing for warped projection
/// (the original's own comment): when the page's time is up the next
/// attract-loop entry is queued.
pub fn d_page_ticker() {
    let mut p = PAGES.with(|p| p.get());
    p.pagetic -= 1;
    if p.pagetic < 0 {
        p.advancedemo = true; // D_AdvanceDemo
    }
    PAGES.with(|c| c.set(p));
}

/// Port of `D_PageDrawer`.
pub fn d_page_drawer(v: &mut crate::v_video::VVideo, wad: &mut WadFiles) {
    let name = PAGES.with(|p| p.get().pagename);
    let patch = wad
        .cache_lump_name(name, crate::z_zone::PurgeTag::Cache)
        .to_vec();
    v.v_draw_patch(0, 0, 0, &patch);
}

/// Port of `D_AdvanceDemo`: called after each demo or intro
/// demosequence finishes (the original's own comment).
pub fn d_advance_demo() {
    let mut p = PAGES.with(|p| p.get());
    p.advancedemo = true;
    PAGES.with(|c| c.set(p));
}

/// Whether an advance is pending (`advancedemo`); the main loop calls
/// [`d_do_advance_demo`] when it is.
pub fn d_advancedemo_pending() -> bool {
    PAGES.with(|p| p.get().advancedemo)
}

/// Port of `D_DoAdvanceDemo`. This cycles through the title, the
/// demos and the credit screens (the original's own comment); the
/// demo entries are skipped, see the module docs.
pub fn d_do_advance_demo(world: &mut GameWorld) {
    use crate::doomdef::GameMode;
    let mut p = PAGES.with(|p| p.get());

    if let Some(player) = world
        .players
        .get_mut(doomstat::state().consoleplayer as usize)
    {
        player.playerstate = crate::d_player::PlayerState::Live; // not reborn
    }
    p.advancedemo = false;

    let st = doomstat::state_mut();
    st.usergame = false; // no save / end game here
    st.paused = false;
    st.gameaction = crate::d_event::GameAction::Nothing;
    let gamemode = st.gamemode;

    let modulus = if gamemode == GameMode::Retail { 7 } else { 6 };
    loop {
        p.demosequence = (p.demosequence + 1) % modulus;
        // (1, 3, 5, 6 are the demos: not ported, skipped)
        if !matches!(p.demosequence, 1 | 3 | 5 | 6) {
            break;
        }
    }

    match p.demosequence {
        0 => {
            p.pagetic = if gamemode == GameMode::Commercial {
                35 * 11
            } else {
                170
            };
            st.gamestate = GameStateEnum::DemoScreen;
            p.pagename = "TITLEPIC";
            if gamemode == GameMode::Commercial {
                crate::s_sound::s_start_music(
                    &mut world.wad,
                    crate::sounds::MusicEnum::MusDm2ttl as i32,
                );
            } else {
                crate::s_sound::s_start_music(
                    &mut world.wad,
                    crate::sounds::MusicEnum::MusIntro as i32,
                );
            }
        }
        2 => {
            p.pagetic = 200;
            st.gamestate = GameStateEnum::DemoScreen;
            p.pagename = "CREDIT";
        }
        4 => {
            st.gamestate = GameStateEnum::DemoScreen;
            if gamemode == GameMode::Commercial {
                p.pagetic = 35 * 11;
                p.pagename = "TITLEPIC";
                crate::s_sound::s_start_music(
                    &mut world.wad,
                    crate::sounds::MusicEnum::MusDm2ttl as i32,
                );
            } else {
                p.pagetic = 200;
                p.pagename = if gamemode == GameMode::Retail {
                    "CREDIT"
                } else {
                    "HELP2"
                };
            }
        }
        _ => {}
    }
    PAGES.with(|c| c.set(p));
}

/// Port of `D_StartTitle`: begins the attract loop at the title page
/// (the menu's "end game" and the startup call it). The first page
/// appears when the main loop runs [`d_do_advance_demo`].
pub fn d_start_title() {
    doomstat::state_mut().gameaction = crate::d_event::GameAction::Nothing;
    let mut p = PAGES.with(|p| p.get());
    p.demosequence = -1;
    p.advancedemo = true; // D_AdvanceDemo
    PAGES.with(|c| c.set(p));
}

/// The command-line parameters `D_DoomMain` reads for starting play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartArgs {
    pub skill: crate::doomdef::Skill,
    pub episode: i32,
    pub map: i32,
    /// `-warp`/`-episode`: skip the title and go straight into a game.
    pub autostart: bool,
}

impl StartArgs {
    /// Parses `-skill n`, `-episode n`, `-warp [e] m` from `args`
    /// (`myargv`), like `D_DoomMain` (`args[0]` is the program name).
    pub fn parse(args: &[String], gamemode: crate::doomdef::GameMode) -> Self {
        use crate::doomdef::{GameMode, Skill};
        let find = |name: &str| {
            args.iter()
                .enumerate()
                .skip(1)
                .find(|(_, a)| a.eq_ignore_ascii_case(name))
                .map(|(i, _)| i)
        };
        let digit = |s: &str| s.bytes().next().map(|b| b as i32 - b'0' as i32);

        let mut out = StartArgs {
            skill: Skill::Medium,
            episode: 1,
            map: 1,
            autostart: false,
        };

        if let Some(p) = find("-skill").filter(|&p| p + 1 < args.len()) {
            let n = args[p + 1]
                .bytes()
                .next()
                .map_or(3, |b| b as i32 - b'1' as i32);
            out.skill = match n {
                0 => Skill::Baby,
                1 => Skill::Easy,
                2 => Skill::Medium,
                3 => Skill::Hard,
                _ => Skill::Nightmare,
            };
            out.autostart = true;
        }

        if let Some(p) = find("-episode").filter(|&p| p + 1 < args.len()) {
            out.episode = digit(&args[p + 1]).unwrap_or(1);
            out.map = 1;
            out.autostart = true;
        }

        if let Some(p) = find("-warp").filter(|&p| p + 1 < args.len()) {
            if gamemode == GameMode::Commercial {
                out.map = args[p + 1].parse().unwrap_or(1);
            } else {
                out.episode = digit(&args[p + 1]).unwrap_or(1);
                out.map = args.get(p + 2).and_then(|a| digit(a)).unwrap_or(1);
            }
            out.autostart = true;
        }
        out
    }
}

/// Port of `IdentifyVersion`'s game-mode decision, from the IWAD's
/// lumps (see the module docs): DOOM II has `MAP01`; Ultimate DOOM
/// `E4M1`; registered `E3M1`; shareware only `E1M1`.
pub fn identify_version(wad: &WadFiles) -> crate::doomdef::GameMode {
    use crate::doomdef::GameMode;
    if wad.check_num_for_name("map01").is_some() {
        GameMode::Commercial
    } else if wad.check_num_for_name("e4m1").is_some() {
        GameMode::Retail
    } else if wad.check_num_for_name("e3m1").is_some() {
        GameMode::Registered
    } else if wad.check_num_for_name("e1m1").is_some() {
        GameMode::Shareware
    } else {
        GameMode::Indetermined
    }
}

/// Port of `D_ProcessEvents`. Send all the events of the given
/// timestamp down the responder chain (the original's own comment,
/// preserved): `M_Responder` first, then `G_Responder`. Returns the
/// events nobody ate for the caller to feed to the input state
/// ([`g_game::g_responder`]) — see [`d_process_event`].
pub fn d_process_events(world: &mut GameWorld, video: &mut IVideo, events: &[Event]) {
    for ev in events {
        if !d_process_event(world, video, ev) {
            g_game::g_responder(&mut world.controls, ev);
        }
    }
}

/// One event down the responder chain, minus the final key/mouse state
/// tracking (`G_Responder`'s tail): `M_Responder`, then — by game
/// state — `HU_Responder`/`ST_Responder`/`AM_Responder` in a level or
/// `F_Responder` in a finale. Returns true if something ate it.
pub fn d_process_event(world: &mut GameWorld, video: &mut IVideo, ev: &Event) -> bool {
    // M_Responder
    if crate::m_menu::m_is_initialized() {
        let mut ctx = crate::m_menu::MenuCtx {
            players: &mut world.players,
            wad: &mut world.wad,
            video: &mut video.v_video,
            renderer: &mut world.renderer,
            palette: None,
        };
        let eaten = crate::m_menu::m_responder(ev, &mut ctx);
        let palette = ctx.palette.take();
        if let Some(palette) = palette {
            video.i_set_palette(&palette);
        }
        if eaten {
            return true;
        }
    }

    // G_Responder
    match doomstat::state().gamestate {
        GameStateEnum::Level => {
            if world.players.is_empty() {
                return false;
            }
            let ingame = world.playeringame();
            let console = (doomstat::state().consoleplayer as usize).min(world.players.len() - 1);
            crate::hu_stuff::hu_is_initialized()
                && crate::hu_stuff::hu_responder(ev, &mut world.players, &ingame)
                || crate::st_stuff::st_is_initialized()
                    && crate::st_stuff::st_responder(
                        ev,
                        &mut world.players[console],
                        &mut world.thinkers,
                        &mut world.wad,
                    )
                || crate::am_map::am_responder(
                    ev,
                    &mut crate::am_map::AmCtx {
                        player: &mut world.players[console],
                        thinkers: &mut world.thinkers,
                        level: &world.level,
                        wad: &mut world.wad,
                    },
                )
        }
        GameStateEnum::Finale => crate::f_finale::f_responder(ev),
        _ => false,
    }
}

/// Builds the [`ViewPlayer`] `R_RenderPlayerView` needs from a live
/// [`Player`]/its mobj — the per-frame glue `R_SetupFrame` reads
/// `viewplayer->...` for in the original, made explicit here since this
/// port passes it as a value instead of a global.
fn view_player(thinkers: &Thinkers, level: &Level, player: &Player) -> ViewPlayer {
    let mobj = thinkers
        .mobj(player.mo)
        .expect("player mobj is always live");
    let sector = level.subsectors[mobj.subsector.expect("spawned mobj has a subsector")].sector;

    ViewPlayer {
        x: mobj.x,
        y: mobj.y,
        angle: mobj.angle,
        viewz: player.viewz,
        extralight: player.extralight,
        fixedcolormap: player.fixedcolormap,
        sprites: PlayerSprites {
            sector_lightlevel: level.sectors[sector].lightlevel as i32,
            invisibility: player.power(crate::doomdef::PowerType::PwInvisibility),
            psprites: std::array::from_fn(|i| {
                let psp = player.psprites[i];
                crate::p_pspr::psprite_sprite_frame(&psp).map(|(sprite, frame)| PSprite {
                    sprite: sprite as i32,
                    frame,
                    sx: psp.sx,
                    sy: psp.sy,
                })
            }),
        },
    }
}

/// `D_Display`'s function-local statics (`viewactivestate`,
/// `menuactivestate`, `inhelpscreensstate`, `fullscreen`,
/// `oldgamestate`, `borderdrawcount`) for the `GS_LEVEL` slice.
#[derive(Debug, Clone, Copy)]
struct DisplayState {
    /// `oldgamestate == GS_LEVEL`.
    oldgamestate_was_level: bool,
    borderdrawcount: i32,
    viewactivestate: bool,
    menuactivestate: bool,
    inhelpscreensstate: bool,
    fullscreen: bool,
    /// The game state drawn last frame (`oldgamestate`).
    last_gamestate: Option<GameStateEnum>,
}

thread_local! {
    static DISPLAY: std::cell::Cell<DisplayState> = const {
        std::cell::Cell::new(DisplayState {
            oldgamestate_was_level: false,
            borderdrawcount: 0,
            viewactivestate: false,
            menuactivestate: false,
            inhelpscreensstate: false,
            fullscreen: false,
            last_gamestate: None,
        })
    };
}

/// Port of `D_Display`: draws the current game state — the level view
/// (with status bar, automap, HUD, border, pause picture), the
/// intermission or the finale — with the menu on top, and presents the
/// frame. On a game-state change the palette goes back to the default.
fn d_display_world_inner(world: &mut GameWorld, video: &mut IVideo) {
    let gamestate = doomstat::state().gamestate;
    let mut ds = DISPLAY.with(|d| d.get());

    // clean up border stuff
    if ds.last_gamestate != Some(gamestate) && gamestate != GameStateEnum::Level {
        let playpal = world
            .wad
            .cache_lump_name("PLAYPAL", crate::z_zone::PurgeTag::Cache)
            .to_vec();
        video.i_set_palette(&playpal);
    }
    if gamestate != GameStateEnum::Level {
        ds.oldgamestate_was_level = false;
    }
    ds.last_gamestate = Some(gamestate);
    DISPLAY.with(|d| d.set(ds));

    match gamestate {
        GameStateEnum::Level => {
            let display = doomstat::state().displayplayer as usize;
            if let Some(player) = world.players.get(display) {
                d_display(
                    &mut world.renderer,
                    &mut world.level,
                    &mut world.wad,
                    &world.thinkers,
                    player,
                    &world.players,
                    video,
                );
            }
        }
        GameStateEnum::Intermission => {
            if crate::wi_stuff::wi_is_active() {
                crate::wi_stuff::wi_drawer(&mut video.v_video);
            }
            finish_non_level(world, video);
        }
        GameStateEnum::Finale => {
            crate::f_finale::f_drawer(
                &mut video.v_video,
                &mut world.wad,
                &world.renderer.rthings.sprites,
                world.renderer.rstate.firstspritelump,
            );
            finish_non_level(world, video);
        }
        GameStateEnum::DemoScreen => {
            d_page_drawer(&mut video.v_video, &mut world.wad);
            finish_non_level(world, video);
        }
    }
}

thread_local! {
    static WIPE_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// While the wipe captures the new picture, the frame isn't shown.
    static HOLD_PRESENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Turns the screen wipe on or off (a port addition, for fast tests;
/// the original always wipes).
pub fn d_set_wipe(on: bool) {
    WIPE_ENABLED.with(|w| w.set(on));
}

/// `I_FinishUpdate` at the end of `D_Display` — held back while the
/// wipe is capturing the picture to melt to.
fn present(video: &mut IVideo) {
    if !HOLD_PRESENT.with(|h| h.get()) {
        video.i_finish_update();
    }
}

/// Port of `D_Display`'s state dispatch plus its screen wipe: when the
/// game state changed since the last frame (`gamestate !=
/// wipegamestate`) the old picture melts into the new one.
pub fn d_display_world(world: &mut GameWorld, video: &mut IVideo) {
    let gamestate = doomstat::state().gamestate;
    let wipe = WIPE_ENABLED.with(|w| w.get()) && doomstat::state().wipegamestate != Some(gamestate);
    if wipe {
        // save the current screen (wipe_StartScreen)
        crate::f_wipe::wipe_start_screen(&mut video.v_video, 0, 0, SCREENWIDTH, SCREENHEIGHT);
        HOLD_PRESENT.with(|h| h.set(true));
    }
    d_display_world_inner(world, video);
    HOLD_PRESENT.with(|h| h.set(false));
    doomstat::state_mut().wipegamestate = Some(gamestate);

    if !wipe {
        return;
    }
    // wipe_EndScreen, then melt (the loop at the end of D_Display)
    crate::f_wipe::wipe_end_screen(&mut video.v_video, 0, 0, SCREENWIDTH, SCREENHEIGHT);
    let mut wipestart = crate::i_system::i_get_time() - 1;
    loop {
        let (nowtime, tics) = loop {
            let now = crate::i_system::i_get_time();
            if now - wipestart != 0 {
                break (now, now - wipestart);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        wipestart = nowtime;
        let done = crate::f_wipe::wipe_screen_wipe(
            &mut video.v_video,
            crate::f_wipe::WIPE_MELT,
            0,
            0,
            SCREENWIDTH,
            SCREENHEIGHT,
            tics,
        );
        if crate::m_menu::m_is_initialized() {
            crate::m_menu::m_drawer(&mut video.v_video, &mut world.wad);
        }
        video.i_finish_update(); // page flip or blit buffer
        if done {
            break;
        }
    }
}

/// The tail of `D_Display` for the states with no 3D view: the menu on
/// top, then the frame goes to the window.
fn finish_non_level(world: &mut GameWorld, video: &mut IVideo) {
    if crate::m_menu::m_is_initialized() {
        crate::m_menu::m_drawer(&mut video.v_video, &mut world.wad);
    }
    present(video);
}

/// Port of the `GS_LEVEL` slice of `D_Display` this phase needs — draw
/// current display (the original's own comment, preserved). `things` is
/// the level's mobj pool in spawn order, same convention as
/// `tests/real_frame.rs` (Phase 6b) — see `r_main`'s module docs. See
/// module docs for everything else `D_Display` does that isn't ported.
///
/// Also here (from `D_Display`): applying a pending `R_SetViewSize`,
/// the border around a reduced view (`R_FillBackScreen` once per
/// level/resize, `R_DrawViewBorder` while the menu is up and for three
/// frames after anything that could have smeared it), the pause
/// picture, and `M_Drawer` on top of everything.
pub fn d_display(
    renderer: &mut Renderer,
    level: &mut Level,
    wad: &mut WadFiles,
    thinkers: &Thinkers,
    player: &Player,
    players: &[Player],
    video: &mut IVideo,
) {
    let mut ds = DISPLAY.with(|d| d.get());
    let st = crate::doomstat::state();
    let (menuactive, automapactive, paused) = (st.menuactive, st.automapactive, st.paused);
    let mut redrawsbar = false;

    // change the view size if needed
    if renderer.r_apply_pending_view_size() {
        ds.oldgamestate_was_level = false; // force background redraw
        ds.borderdrawcount = 3;
    }

    // HU_Erase: restore the border under last frame's message text.
    if crate::hu_stuff::hu_is_initialized() {
        let view = crate::hu_lib::ViewWin {
            x: renderer.rdraw.viewwindowx,
            y: renderer.rdraw.viewwindowy,
            width: renderer.view.viewwidth,
            height: renderer.view.viewheight,
        };
        crate::hu_stuff::hu_erase(&view, &renderer.rdraw, &mut video.v_video);
    }

    let vp = view_player(thinkers, level, player);
    let things: Vec<crate::r_defs::Mobj> = thinkers.iter_mobjs().map(|(_, m)| *m).collect();

    // ST_Drawer(viewheight == 200, redrawsbar) — before the 3D view in
    // the original too, though the two never overlap on screen. The
    // status bar reports palette changes (damage/pickup/radsuit) for us
    // to apply. Skipped when the status bar was never initialised
    // (headless tools/tests that render without one).
    // do buffered drawing: the automap replaces the view
    if automapactive {
        crate::am_map::am_drawer(&mut video.v_video, level, thinkers, player, players);
    }

    let fullscreen_now = renderer.view.viewheight == crate::doomdef::SCREENHEIGHT;
    let inhelpscreens = crate::m_menu::m_is_initialized() && crate::m_menu::m_inhelpscreens();
    if !fullscreen_now && ds.fullscreen {
        redrawsbar = true;
    }
    if ds.inhelpscreensstate && !inhelpscreens {
        redrawsbar = true; // just put away the help screen
    }
    if crate::st_stuff::st_is_initialized() {
        if let Some(palette) =
            crate::st_stuff::st_drawer(fullscreen_now, redrawsbar, player, &mut video.v_video)
        {
            video.i_set_palette(&palette);
        }
    }
    ds.fullscreen = fullscreen_now;

    // draw the view directly
    if !automapactive {
        renderer.r_render_player_view(&vp, level, wad, &things, &mut video.v_video.screens[0]);
    }

    // HU_Drawer: messages (and the automap title) over the view.
    if crate::hu_stuff::hu_is_initialized() {
        crate::hu_stuff::hu_drawer(&mut video.v_video);
    }

    // see if the border needs to be initially drawn
    if !ds.oldgamestate_was_level {
        ds.viewactivestate = false; // view was not active
        renderer
            .rdraw
            .r_fill_back_screen(&mut video.v_video, wad, st.gamemode); // draw the pattern into the back screen
    }

    // see if the border needs to be updated to the screen
    if !automapactive && renderer.rdraw.scaledviewwidth != 320 {
        if menuactive || ds.menuactivestate || !ds.viewactivestate {
            ds.borderdrawcount = 3;
        }
        if ds.borderdrawcount != 0 {
            renderer.rdraw.r_draw_view_border(&mut video.v_video); // erase old menu stuff
            ds.borderdrawcount -= 1;
        }
    }

    ds.menuactivestate = menuactive;
    ds.viewactivestate = !automapactive; // viewactive
    ds.inhelpscreensstate = inhelpscreens;
    ds.oldgamestate_was_level = true;

    // draw pause pic
    if paused {
        let y = if automapactive {
            4
        } else {
            renderer.rdraw.viewwindowy + 4
        };
        let patch = wad
            .cache_lump_name("M_PAUSE", crate::z_zone::PurgeTag::Cache)
            .to_vec();
        video.v_video.v_draw_patch_direct(
            renderer.rdraw.viewwindowx + (renderer.rdraw.scaledviewwidth - 68) / 2,
            y,
            0,
            &patch,
        );
    }

    // menus go directly to the screen
    if crate::m_menu::m_is_initialized() {
        crate::m_menu::m_drawer(&mut video.v_video, wad); // menu is drawn even on top of everything
    }

    DISPLAY.with(|d| d.set(ds));
    present(video);
}

/// The game as [`crate::d_net::NetHost`]: what `TryRunTics`/`NetUpdate`
/// reach for through globals in the original — the clock, polling input
/// into a ticcmd, and running one game tic (`D_DoAdvanceDemo`,
/// `M_Ticker`, `G_Ticker`, `gametic++`).
pub struct MainHost<'a> {
    pub world: &'a mut GameWorld,
    pub video: &'a mut IVideo,
    /// Set when the window was closed (checked by the main loop).
    pub quit: bool,
}

impl crate::d_net::NetHost for MainHost<'_> {
    fn get_time(&mut self) -> i32 {
        crate::i_system::i_get_time()
    }

    fn build_ticcmd(&mut self, maketic: i32) -> crate::d_ticcmd::TicCmd {
        // I_StartTic(); D_ProcessEvents(); G_BuildTiccmd()
        let events = self.video.i_start_tic();
        if self.video.should_quit() {
            self.quit = true;
        }
        d_process_events(self.world, self.video, &events);
        let mut cmd = g_game::g_build_ticcmd(&mut self.world.controls, 1);
        // the consistency value for the tic this command is for
        let console = doomstat::state().consoleplayer as usize;
        cmd.consistancy = self.world.consistancy[console][(maketic as usize) % BACKUPTICS];
        cmd
    }

    fn gametic(&self) -> i32 {
        doomstat::state().gametic
    }

    fn run_tic(&mut self, cmds: &[crate::d_ticcmd::TicCmd; crate::d_net::MAXPLAYERS]) {
        if d_advancedemo_pending() {
            d_do_advance_demo(self.world);
        }
        crate::m_menu::m_ticker();
        self.world.validcount += 1;
        g_game::g_ticker_cmds(self.world, &mut self.video.v_video, cmds);
        doomstat::state_mut().gametic += 1;
    }

    fn menu_ticker(&mut self) {
        crate::m_menu::m_ticker();
    }

    fn player_left(&mut self, player: usize) {
        doomstat::state_mut().playeringame[player] = false;
        let console = doomstat::state().consoleplayer as usize;
        if let Some(p) = self.world.players.get_mut(console) {
            p.message = Some(match player {
                0 => "Player 1 left the game",
                1 => "Player 2 left the game",
                2 => "Player 3 left the game",
                _ => "Player 4 left the game",
            });
        }
    }

    fn idle(&mut self) {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    fn abort_requested(&mut self) -> bool {
        // CheckAbort: ESC while waiting for the other machines
        let events = self.video.i_start_tic();
        if self.video.should_quit() {
            self.quit = true;
        }
        events.iter().any(|e| {
            e.event_type == crate::d_event::EvType::KeyDown && e.data1 == crate::doomdef::KEY_ESCAPE
        })
    }
}
