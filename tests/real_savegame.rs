//! Phase 10: save/load round trip with the real WAD. Play some tics,
//! start a moving door, serialize, load into a fresh world and compare.
//! (Own test binary: it writes process-wide `doomstat`.)

mod common;

use doommetal_rust::d_ticcmd::TicCmd;
use doommetal_rust::doomdef::{GameMode, Skill};
use doommetal_rust::doomstat;
use doommetal_rust::g_game::{self, GameWorld};
use doommetal_rust::p_doors::VlDoorType;
use doommetal_rust::r_main::Renderer;
use doommetal_rust::v_video::VVideo;
use doommetal_rust::w_wad::WadFiles;

fn world() -> Option<(GameWorld, VVideo)> {
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
    let mut video = VVideo::new();
    doommetal_rust::st_stuff::st_init(&mut wad, &mut video);
    doommetal_rust::hu_stuff::hu_init(&mut wad);
    Some((GameWorld::new(wad, renderer), video))
}

fn tick(w: &mut GameWorld, v: &mut VVideo, forward: i8) {
    let cmd = TicCmd {
        forwardmove: forward,
        ..Default::default()
    };
    g_game::g_ticker(w, v, cmd);
    doomstat::state_mut().gametic += 1;
}

#[test]
fn save_and_load_round_trip_preserves_the_world() {
    let Some((mut w, mut v)) = world() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    g_game::g_init_new(&mut w, Skill::Medium, 1, 1);
    for _ in 0..40 {
        tick(&mut w, &mut v, 25);
    }

    // start a moving door through the first tagged line
    let line = w.level.lines.iter().find(|l| l.tag != 0).cloned();
    let line = line.expect("E1M1 has a tagged line");
    {
        let GameWorld {
            thinkers, level, ..
        } = &mut w;
        let started = doommetal_rust::p_doors::ev_do_door(thinkers, level, &line, VlDoorType::Open);
        assert!(started, "a door thinker is running when we save");
    }
    for _ in 0..10 {
        tick(&mut w, &mut v, 25);
    }

    let bytes = g_game::g_save_game_bytes(&w, "round trip");
    let leveltime = doomstat::state().leveltime;
    let (health, x, y, z) = {
        let p = &w.players[0];
        let mo = w.thinkers.mobj(p.mo).unwrap();
        assert_eq!(p.health, mo.health);
        (p.health, mo.x, mo.y, mo.z)
    };
    let floors: Vec<_> = w
        .level
        .sectors
        .iter()
        .map(|s| (s.floorheight, s.ceilingheight, s.lightlevel))
        .collect();

    // load into a completely fresh world
    let (mut w2, _v2) = world().unwrap();
    doomstat::state_mut().leveltime = 0;
    g_game::g_load_game_bytes(&mut w2, &bytes).expect("load");
    assert_eq!(doomstat::state().leveltime, leveltime);
    let p = &w2.players[0];
    let mo = w2.thinkers.mobj(p.mo).unwrap();
    assert_eq!((p.health, mo.x, mo.y, mo.z), (health, x, y, z));
    let floors2: Vec<_> = w2
        .level
        .sectors
        .iter()
        .map(|s| (s.floorheight, s.ceilingheight, s.lightlevel))
        .collect();
    assert_eq!(floors, floors2);

    // saving the loaded world gives the very same bytes
    let again = g_game::g_save_game_bytes(&w2, "round trip");
    assert_eq!(bytes, again, "save -> load -> save is stable");

    // and it keeps running
    let mut v2 = VVideo::new();
    for _ in 0..20 {
        tick(&mut w2, &mut v2, 25);
    }

    bad_savegames_are_rejected_or_ignored();
}

fn bad_savegames_are_rejected_or_ignored() {
    let (mut w, mut v) = world().unwrap();
    g_game::g_init_new(&mut w, Skill::Medium, 1, 1);
    for _ in 0..5 {
        tick(&mut w, &mut v, 0);
    }
    let bytes = g_game::g_save_game_bytes(&w, "x");

    // truncated
    assert!(g_game::g_load_game_bytes(&mut w, &bytes[..bytes.len() / 2]).is_err());
    // bad consistency marker
    let mut bad = bytes.clone();
    *bad.last_mut().unwrap() ^= 0xff;
    assert!(g_game::g_load_game_bytes(&mut w, &bad).is_err());
    // wrong version: silently ignored (Ok, nothing loaded)
    let mut ver = bytes.clone();
    ver[24] ^= 0xff; // first byte of the version string, after the 24-byte description
    assert!(g_game::g_load_game_bytes(&mut w, &ver).is_ok());
}
