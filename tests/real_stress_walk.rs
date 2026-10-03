//! Stress: walk E1M1 for a long time with pseudo-random input (moving,
//! strafing, turning, using, firing) and check the game never panics
//! and the player never leaves the map's blockmap. (Own test binary:
//! it writes process-wide `doomstat`.)

mod common;

use doommetal_rust::d_main::d_display_world;
use doommetal_rust::d_ticcmd::TicCmd;
use doommetal_rust::doomdef::{GameMode, Skill};
use doommetal_rust::doomstat;
use doommetal_rust::g_game::{self, GameWorld};
use doommetal_rust::i_video::IVideo;
use doommetal_rust::r_main::Renderer;
use doommetal_rust::w_wad::WadFiles;
use doommetal_rust::z_zone::PurgeTag;

fn world() -> Option<(GameWorld, IVideo)> {
    let path = common::find_test_wad()?;
    static ARGS: std::sync::Once = std::sync::Once::new();
    ARGS.call_once(|| doommetal_rust::m_argv::set_args(std::env::args().collect()));
    let mut wad = WadFiles::new();
    wad.init_file(path);
    let mut renderer = Renderer::r_init(&mut wad, 10, 0);
    let sprnames = doommetal_rust::info::SPRNAMES;
    renderer
        .rthings
        .r_init_sprites(&wad, &renderer.rdata, &sprnames, false);
    doomstat::state_mut().gamemode = GameMode::Registered;
    std::env::set_var("SDL_VIDEODRIVER", "dummy");
    std::env::set_var("SDL_AUDIODRIVER", "dummy");
    let mut video = IVideo::i_init_graphics();
    let playpal = wad.cache_lump_name("PLAYPAL", PurgeTag::Cache).to_vec();
    video.i_set_palette(&playpal);
    doommetal_rust::st_stuff::st_init(&mut wad, &mut video.v_video);
    doommetal_rust::hu_stuff::hu_init(&mut wad);
    doomstat::state_mut().screenblocks = 10;
    doommetal_rust::m_menu::m_init();
    Some((GameWorld::new(wad, renderer), video))
}

fn tick(w: &mut GameWorld, v: &mut IVideo, fwd: i8, buttons: u8) {
    let cmd = TicCmd {
        forwardmove: fwd,
        buttons,
        ..Default::default()
    };
    g_game::g_ticker(w, &mut v.v_video, cmd);
    d_display_world(w, v);
    doomstat::state_mut().gametic += 1;
}

#[test]
fn random_walk_never_panics_or_leaves_the_map() {
    let Some((mut w, mut v)) = world() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    // Regression: the move bbox once had left/right swapped, so no line
    // was ever checked and the player walked through every wall. E1M1's
    // line 4 is a solid wall at y = -3648 (x 960..1024).
    g_game::g_init_new(&mut w, Skill::Medium, 1, 1);
    {
        let GameWorld {
            thinkers,
            level,
            players,
            renderer,
            ..
        } = &mut w;
        let mo = players[0].mo;
        let mut check = |y: i32| {
            doommetal_rust::p_map::p_check_position(
                thinkers,
                level,
                players,
                &mut renderer.rmain,
                0,
                mo,
                1002 << 16,
                y << 16,
            )
            .0
        };
        assert!(check(-3620), "open floor north of the wall");
        assert!(!check(-3650), "the solid wall blocks the player");
    }

    // Regression: USE on a door line in front of the player opens it.
    g_game::g_init_new(&mut w, Skill::Medium, 1, 1);
    {
        let (li, _) = w
            .level
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.special == 1 && l.backsector.is_some())
            .expect("E1M1 has a DR door");
        let ln = w.level.lines[li];
        let v1 = w.level.vertexes[ln.v1];
        let (mx, my) = (v1.x + ln.dx / 2, v1.y + ln.dy / 2);
        // stand 40 units from the middle, on the front side, facing it
        let (nx, ny) = (ln.dy, -ln.dx); // right-hand normal = front side
        let len = ((nx as f64).powi(2) + (ny as f64).powi(2)).sqrt();
        let (ux, uy) = (nx as f64 / len, ny as f64 / len);
        let (px, py) = (
            mx + (ux * 40.0 * 65536.0) as i32,
            my + (uy * 40.0 * 65536.0) as i32,
        );
        let ang = ((-uy).atan2(-ux) / std::f64::consts::TAU * 4294967296.0).rem_euclid(4294967296.0)
            as u32;
        let mo = w.players[0].mo;
        {
            let GameWorld {
                thinkers, level, ..
            } = &mut w;
            doommetal_rust::p_mobj::p_unset_thing_position(thinkers, level, mo);
            let m = thinkers.mobj_mut(mo).unwrap();
            m.x = px;
            m.y = py;
            m.angle = ang;
            doommetal_rust::p_mobj::p_set_thing_position(thinkers, level, mo);
        }
        let sec = ln.backsector.unwrap();
        let before = w.level.sectors[sec].ceilingheight;
        tick(&mut w, &mut v, 0, 0); // release USE first (usedown starts true)
        for _ in 0..10 {
            tick(&mut w, &mut v, 0, 2);
        }
        let after = w.level.sectors[sec].ceilingheight;
        assert!(
            after > before,
            "USE opened the door (ceiling {before} -> {after})"
        );
    }

    // `STRESS_TICS` / `STRESS_SEED` / `STRESS_MAPS=all` widen the search
    // (the defaults keep the suite fast).
    let tics: i32 = std::env::var("STRESS_TICS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let seed0: u32 = std::env::var("STRESS_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let maps: Vec<(i32, i32)> = if std::env::var("STRESS_MAPS").is_ok() {
        (1..=3).flat_map(|e| (1..=9).map(move |m| (e, m))).collect()
    } else {
        vec![(1, 1), (1, 2), (1, 3)]
    };
    for (episode, map) in maps {
        g_game::g_init_new(&mut w, Skill::Medium, episode, map);
        let mut seed: u32 = 12345 + map as u32 + episode as u32 * 100 + seed0 * 7919;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            (seed >> 16) & 0x7fff
        };
        let (mut fwd, mut side, mut turn, mut btn) = (0i8, 0i8, 0i16, 0u8);
        for tic in 0..tics {
            if tic % 20 == 0 {
                fwd = [50, 25, 0, -25][rnd() as usize % 4];
                side = [0, 40, -40, 0][rnd() as usize % 4];
                turn = ((rnd() % 1500) as i16 - 750) * 2;
                btn = [0, 1, 2, 0][rnd() as usize % 4];
            }
            let cmd = TicCmd {
                forwardmove: fwd,
                sidemove: side,
                angleturn: turn,
                buttons: btn,
                ..Default::default()
            };
            g_game::g_ticker(&mut w, &mut v.v_video, cmd);
            if tic % 3 == 0 {
                d_display_world(&mut w, &mut v);
            }
            doomstat::state_mut().gametic += 1;
            if doomstat::state().gamestate != doommetal_rust::doomdef::GameState::Level {
                break;
            }
            if let Some(p) = w.players.first() {
                if let Some(mo) = w.thinkers.mobj(p.mo) {
                    let l = &w.level;
                    let bx = (mo.x >> 16) - (l.bmaporgx >> 16);
                    let by = (mo.y >> 16) - (l.bmaporgy >> 16);
                    assert!(
                        bx >= 0 && by >= 0 && bx < l.bmapwidth * 128 && by < l.bmapheight * 128,
                        "map {map} tic {tic}: player left the map at ({}, {})",
                        mo.x >> 16,
                        mo.y >> 16
                    );
                }
            }
        }
    }
}
