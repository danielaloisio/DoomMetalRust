//! Phase 6's closing milestone: "consegue jogar um nível single-player
//! sem inimigos" (play a level, single player, no enemies) — runs the
//! *real* game loop (`g_game::g_init_new`/`g_ticker`, `d_main::d_display`)
//! against the real E1M1, simulating a held-forward keypress for a few
//! seconds of tics, and verifies the player's mobj actually moved and
//! every frame rendered without holes — the same real path `main.rs`
//! drives, just under SDL's headless "dummy" video driver so it runs
//! without a real display (same technique `tests/video_screenshot.rs`
//! uses for Phase 4's milestone).
//!
//! Skipped (not failed) if no doom.wad is found — same convention as
//! the other `tests/real_*.rs` files.

// The menu/automap phases pass the world's pieces as `&mut &mut T`
// (deref-coerced), which clippy would rather see without the `&mut`.
#![allow(clippy::needless_borrow)]

use doommetal_rust::d_event::{EvType, Event};
use doommetal_rust::d_main::{d_display, d_display_world};
use doommetal_rust::d_player::Player;
use doommetal_rust::doomdef::Skill;
use doommetal_rust::doomstat;
use doommetal_rust::g_game;
use doommetal_rust::i_video::IVideo;
use doommetal_rust::m_argv;
use doommetal_rust::p_setup::Level;
use doommetal_rust::p_tick::Thinkers;
use doommetal_rust::r_main::Renderer;
use doommetal_rust::w_wad::WadFiles;
use doommetal_rust::z_zone::PurgeTag;

mod common;
use common::{find_test_wad, save_palettized_png, scratch_dir};

#[test]
fn playing_e1m1_moves_the_player_and_renders_every_frame() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    // Headless SDL, same technique as tests/video_screenshot.rs.
    std::env::set_var("SDL_VIDEODRIVER", "dummy");
    std::env::set_var("SDL_AUDIODRIVER", "dummy");
    m_argv::set_args(std::env::args().collect());

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let mut renderer = Renderer::r_init(&mut wad, 10, 0);
    let sprnames = doommetal_rust::info::SPRNAMES;
    renderer
        .rthings
        .r_init_sprites(&wad, &renderer.rdata, &sprnames, false);

    doomstat::state_mut().gamemode = doommetal_rust::doomdef::GameMode::Registered;

    let mut video = IVideo::i_init_graphics();
    let playpal = wad.cache_lump_name("PLAYPAL", PurgeTag::Cache).to_vec();
    video.i_set_palette(&playpal);

    // The status bar, heads-up text and menu (P_SpawnPlayer wakes the
    // first two up when the level starts).
    doommetal_rust::st_stuff::st_init(&mut wad, &mut video.v_video);
    doommetal_rust::hu_stuff::hu_init(&mut wad);
    doomstat::state_mut().screenblocks = 10; // this test's setblocks
    doommetal_rust::m_menu::m_init();

    let mut world = g_game::GameWorld::new(wad, renderer);
    g_game::g_init_new(&mut world, Skill::Medium, 1, 1);
    assert_eq!(world.players.len(), 1, "P_SpawnPlayer created player 1");
    let player_mo = world.players[0].mo;
    assert_eq!(
        world.thinkers.mobj(player_mo).unwrap().player,
        Some(0),
        "and linked it to its mobj"
    );

    let start_x = world.thinkers.mobj(player_mo).unwrap().x;
    let start_y = world.thinkers.mobj(player_mo).unwrap().y;

    // Hold the "up" key down for every tic, like a player pressing and
    // holding W/UpArrow — g_responder is the real input path (same one
    // main.rs's event loop drives), fed a synthetic keydown event once.
    let key_up = doommetal_rust::doomdef::KEY_UPARROW;
    g_game::g_responder(
        &mut world.controls,
        &Event {
            event_type: EvType::KeyDown,
            data1: key_up,
            data2: 0,
            data3: 0,
        },
    );

    let mut last_frame = Vec::new();
    for tic in 0..70u32 {
        // 2 seconds at 35 Hz.
        if tic == 30 {
            // what a pickup does (P_TouchSpecialThing)
            world.players[0].message = Some("Picked up a clip.");
        }
        let cmd = g_game::g_build_ticcmd(&mut world.controls, 1);
        g_game::g_ticker(&mut world, &mut video.v_video, cmd);

        d_display_world(&mut world, &mut video);

        let frame = video.v_video.screens[0].clone();
        assert_eq!(frame.len(), (320 * 200) as usize);

        if tic == 0 || tic == 34 || tic == 69 {
            let out_path = scratch_dir().join(format!("doommetalrust_gameloop_tic{tic}.png"));
            save_palettized_png(&out_path, 320, 200, &frame, &playpal);
            eprintln!("tic {tic}: frame -> {}", out_path.display());
        }
        last_frame = frame;
    }

    assert!(!last_frame.is_empty());

    // The status bar is on screen: the bottom 32 rows are not blank and
    // differ from the 3D view above them.
    {
        let bar = &last_frame[(doommetal_rust::st_lib::ST_Y * 320) as usize..];
        assert!(
            bar.iter().filter(|&&b| b != 0).count() > 5000,
            "status bar drawn"
        );
        // the STBAR patch's leftmost column is opaque on every row
        let stbar = world.wad.cache_lump_name("STBAR", PurgeTag::Cache).to_vec();
        let col0 = i32::from_le_bytes(stbar[8..12].try_into().unwrap()) as usize;
        assert_eq!(
            stbar[col0 + 3],
            last_frame[(doommetal_rust::st_lib::ST_Y * 320) as usize]
        );
    }

    let end_x = world.thinkers.mobj(player_mo).unwrap().x;
    let end_y = world.thinkers.mobj(player_mo).unwrap().y;
    let moved = (end_x - start_x).unsigned_abs() as i64 + (end_y - start_y).unsigned_abs() as i64;
    assert!(
        moved > 0,
        "holding forward for 2s should have moved the player (start ({start_x},{start_y}), end ({end_x},{end_y}))"
    );

    eprintln!(
        "player moved {} fixed units over 70 tics ({}, {}) -> ({}, {})",
        moved, start_x, start_y, end_x, end_y
    );

    // Phase 7e: hold fire (Ctrl) through the real input path for two
    // more seconds — the weapon has been up since the walk, so the
    // pistol must actually shoot: ammo is spent.
    let clip = doommetal_rust::doomdef::AmmoType::AmClip as usize;
    let ammo_before = world.players[0].ammo[clip];
    g_game::g_responder(
        &mut world.controls,
        &Event {
            event_type: EvType::KeyDown,
            data1: doommetal_rust::doomdef::KEY_RCTRL,
            data2: 0,
            data3: 0,
        },
    );
    for _ in 0..70 {
        let cmd = g_game::g_build_ticcmd(&mut world.controls, 1);
        g_game::g_ticker(&mut world, &mut video.v_video, cmd);
    }
    assert!(
        world.players[0].ammo[clip] < ammo_before,
        "holding fire for 2s should have spent pistol ammo ({ammo_before} -> {})",
        world.players[0].ammo[clip]
    );
    assert!(world.players[0].pending_pspr.is_empty());
    eprintln!(
        "pistol ammo {} -> {}",
        ammo_before, world.players[0].ammo[clip]
    );

    // The rest exercises the menu/automap with the world's pieces borrowed
    // separately (they don't need the game flow).
    let g_game::GameWorld {
        thinkers,
        level,
        renderer,
        wad,
        players,
        ..
    } = &mut world;
    let player = &mut players[0];
    let mut thinkers = thinkers;
    let mut level = level;
    let mut renderer = renderer;
    let mut wad = wad;
    let mut player = player;

    // Phase 9d: the menu, through the real responder and display.
    use doommetal_rust::doomdef::{
        KEY_DOWNARROW, KEY_ENTER, KEY_ESCAPE, KEY_F10, KEY_F11, KEY_F4, KEY_LEFTARROW,
        KEY_RIGHTARROW,
    };
    use doommetal_rust::m_menu::{m_current, m_responder, MenuCtx, MenuId};
    let press = |key: i32,
                 player: &mut Player,
                 wad: &mut WadFiles,
                 video: &mut IVideo,
                 renderer: &mut Renderer|
     -> (bool, Option<Vec<u8>>) {
        let ev = Event {
            event_type: EvType::KeyDown,
            data1: key,
            data2: 0,
            data3: 0,
        };
        let mut ctx = MenuCtx {
            players: std::slice::from_mut(player),
            wad,
            video: &mut video.v_video,
            renderer,
            palette: None,
        };
        let eaten = m_responder(&ev, &mut ctx);
        (eaten, ctx.palette.take())
    };
    let show = |name: &str,
                renderer: &mut Renderer,
                level: &mut Level,
                wad: &mut WadFiles,
                thinkers: &Thinkers,
                player: &Player,
                video: &mut IVideo| {
        d_display(
            renderer,
            level,
            wad,
            thinkers,
            player,
            std::slice::from_ref(player),
            video,
        );
        let frame = video.v_video.screens[0].clone();
        let out = scratch_dir().join(format!("doommetalrust_menu_{name}.png"));
        save_palettized_png(&out, 320, 200, &frame, &playpal);
        eprintln!("{name}: {}", out.display());
        frame
    };

    // ESC opens the main menu.
    assert!(!doomstat::state().menuactive);
    let (eaten, _) = press(KEY_ESCAPE, &mut player, &mut wad, &mut video, &mut renderer);
    assert!(eaten && doomstat::state().menuactive);
    assert_eq!(m_current(), (MenuId::Main, 0));
    let plain = video.v_video.screens[0].clone();
    let with_menu = show(
        "main",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    assert_ne!(plain, with_menu, "the menu was drawn over the view");

    // Down, Enter: Options.
    press(
        KEY_DOWNARROW,
        &mut player,
        &mut wad,
        &mut video,
        &mut renderer,
    );
    press(KEY_ENTER, &mut player, &mut wad, &mut video, &mut renderer);
    assert_eq!(m_current(), (MenuId::Options, 0));
    show(
        "options",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );

    // Down x3 -> "screen size"; Right -> bigger (setblocks 11: full).
    for _ in 0..3 {
        press(
            KEY_DOWNARROW,
            &mut player,
            &mut wad,
            &mut video,
            &mut renderer,
        );
    }
    assert_eq!(m_current(), (MenuId::Options, 3));
    press(
        KEY_RIGHTARROW,
        &mut player,
        &mut wad,
        &mut video,
        &mut renderer,
    );
    assert_eq!(doomstat::state().screenblocks, 11);
    show(
        "size11",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    assert_eq!(
        renderer.view.viewheight, 200,
        "full-screen view applied next frame"
    );

    // Left x2 -> setblocks 9: a bordered 288x144 view.
    press(
        KEY_LEFTARROW,
        &mut player,
        &mut wad,
        &mut video,
        &mut renderer,
    );
    press(
        KEY_LEFTARROW,
        &mut player,
        &mut wad,
        &mut video,
        &mut renderer,
    );
    assert_eq!(doomstat::state().screenblocks, 9);
    let frame = show(
        "size9",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    assert_eq!(
        (renderer.rdraw.scaledviewwidth, renderer.view.viewheight),
        (288, 144)
    );
    // the border pattern (FLOOR7_2) is on screen at the very corner
    assert_ne!(frame[0], 0, "border drawn");

    // Backspace back, then F4 jumps to the sound menu; Left lowers sfx.
    press(KEY_ESCAPE, &mut player, &mut wad, &mut video, &mut renderer);
    assert!(!doomstat::state().menuactive);
    show(
        "size9_closed",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    press(KEY_F4, &mut player, &mut wad, &mut video, &mut renderer);
    assert_eq!(m_current(), (MenuId::Sound, 0));
    let before = doomstat::state().snd_sfx_volume;
    press(
        KEY_LEFTARROW,
        &mut player,
        &mut wad,
        &mut video,
        &mut renderer,
    );
    assert_eq!(doomstat::state().snd_sfx_volume, before - 1);
    show(
        "sound",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    press(KEY_ESCAPE, &mut player, &mut wad, &mut video, &mut renderer);

    // F11 cycles the gamma and hands back the palette to set.
    let (eaten, palette) = press(KEY_F11, &mut player, &mut wad, &mut video, &mut renderer);
    assert!(eaten && palette.is_some());
    assert_eq!(video.v_video.usegamma, 1);
    assert_eq!(player.message, Some("Gamma correction level 1"));

    // F10 quits: a y/n message, then 'y' asks the main loop to exit.
    press(KEY_F10, &mut player, &mut wad, &mut video, &mut renderer);
    assert!(doomstat::state().menuactive);
    show(
        "quit",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    let (eaten, _) = press(
        b'y' as i32,
        &mut player,
        &mut wad,
        &mut video,
        &mut renderer,
    );
    assert!(eaten && doomstat::state().quit_requested);

    // Phase 9e: the automap, through the real responder/ticker/display.
    use doommetal_rust::am_map::{am_responder, am_ticker, AmCtx};
    use doommetal_rust::doomdef::{KEY_RIGHTARROW as KRIGHT, KEY_TAB};
    let am_event = |key: i32,
                    kind: EvType,
                    player: &mut Player,
                    thinkers: &mut Thinkers,
                    level: &Level,
                    wad: &mut WadFiles|
     -> bool {
        let ev = Event {
            event_type: kind,
            data1: key,
            data2: 0,
            data3: 0,
        };
        am_responder(
            &ev,
            &mut AmCtx {
                player,
                thinkers,
                level,
                wad,
            },
        )
    };
    let count_in = |frame: &[u8], lo: u8, hi: u8| -> usize {
        frame[..320 * 168]
            .iter()
            .filter(|&&b| b >= lo && b < hi)
            .count()
    };
    doomstat::state_mut().menuactive = false;

    assert!(!doomstat::state().automapactive);
    assert!(am_event(
        KEY_TAB,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad
    ));
    assert!(doomstat::state().automapactive);
    let map = show(
        "am_start",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    // REDS (176..192): the level's mapped walls; WHITE (209): the arrow
    assert!(count_in(&map, 176, 192) > 30, "walls drawn in red");
    assert!(count_in(&map, 209, 210) > 0, "the player arrow");

    // grid on (message), then off-screen effects of zoom
    assert!(am_event(
        b'g' as i32,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad
    ));
    assert_eq!(player.message, Some("Grid ON"));
    let gridded = show(
        "am_grid",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    assert!(count_in(&gridded, 102, 106) > 20, "grid lines (GRAYS+8)");

    // zoom in for a few tics
    am_event(
        b'=' as i32,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad,
    );
    for _ in 0..30 {
        let pos = {
            let m = thinkers.mobj(player.mo).unwrap();
            (m.x, m.y)
        };
        am_ticker(pos);
    }
    am_event(
        b'=' as i32,
        EvType::KeyUp,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad,
    );
    let zoomed = show(
        "am_zoom",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    assert_ne!(zoomed, gridded, "zoom changed the picture");

    // follow mode: pan keys aren't eaten; turn it off and they are
    assert!(!am_event(
        KRIGHT,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad
    ));
    assert!(am_event(
        b'f' as i32,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad
    ));
    assert_eq!(player.message, Some("Follow Mode OFF"));
    assert!(am_event(
        KRIGHT,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad
    ));
    am_event(
        KRIGHT,
        EvType::KeyUp,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad,
    );

    // marks
    assert!(am_event(
        b'm' as i32,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad
    ));
    assert_eq!(player.message, Some("Marked Spot 0"));
    assert!(am_event(
        b'c' as i32,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad
    ));
    assert_eq!(player.message, Some("All Marks Cleared"));
    am_event(
        b'm' as i32,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad,
    );

    // 'iddt' twice: full map, then things
    for _ in 0..2 {
        for c in "iddt".bytes() {
            am_event(
                c as i32,
                EvType::KeyDown,
                &mut player,
                &mut thinkers,
                &level,
                &mut wad,
            );
        }
    }
    // go big ('0'), which zooms all the way out to the whole level
    am_event(
        b'0' as i32,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad,
    );
    let full = show(
        "am_iddt_big",
        &mut renderer,
        &mut level,
        &mut wad,
        &thinkers,
        &player,
        &mut video,
    );
    assert!(
        count_in(&full, 112, 128) > 3,
        "things drawn in green (iddt x2)"
    );
    assert!(
        count_in(&full, 176, 192) > count_in(&map, 176, 192),
        "cheating shows every wall"
    );

    // TAB leaves the automap
    assert!(am_event(
        KEY_TAB,
        EvType::KeyDown,
        &mut player,
        &mut thinkers,
        &level,
        &mut wad
    ));
    assert!(!doomstat::state().automapactive);
}
