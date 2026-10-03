//! Phase 9f: the intermission with the real E1 art — stats counting up,
//! then the "you are here" map. Writes PNGs of both for a look.

mod common;

use doommetal_rust::d_event::BT_USE;
use doommetal_rust::d_player::{Player, WbPlayerStruct, WbStartStruct};
use doommetal_rust::doomdef::{GameMode, TICRATE};
use doommetal_rust::p_tick::{ThinkFn, ThinkerData, Thinkers};
use doommetal_rust::r_defs::Mobj;
use doommetal_rust::v_video::VVideo;
use doommetal_rust::w_wad::WadFiles;
use doommetal_rust::wi_stuff::{wi_drawer, wi_mode, wi_start_for, wi_ticker, WiEvent, WiMode};
use doommetal_rust::z_zone::PurgeTag;

#[test]
fn e1m1_intermission_counts_then_shows_the_map() {
    let Some(path) = common::find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let mut wad = WadFiles::new();
    wad.init_file(path);
    let playpal = wad.cache_lump_name("PLAYPAL", PurgeTag::Cache).to_vec();

    let mut wbs = WbStartStruct {
        epsd: 0,
        last: 0,
        next: 1,
        maxkills: 20,
        maxitems: 10,
        maxsecret: 3,
        partime: 30 * TICRATE,
        pnum: 0,
        ..Default::default()
    };
    wbs.plyr[0] = WbPlayerStruct {
        in_game: true,
        skills: 17,
        sitems: 10,
        ssecret: 2,
        stime: 95 * TICRATE,
        ..Default::default()
    };

    let mut v = VVideo::new();
    wi_start_for(
        &wbs,
        &[true, false, false, false],
        &mut wad,
        &mut v,
        (GameMode::Registered, false, false, false),
    );

    let mut thinkers = Thinkers::new();
    let mo = thinkers.add_thinker(
        ThinkFn::MobjThinker,
        ThinkerData::Mobj(Mobj::blank(doommetal_rust::info::MobjType::MtPlayer)),
    );
    let mut players = vec![Player::for_test(mo)];

    // let the counters get going, look at a frame mid-count
    for _ in 0..120 {
        wi_ticker(&mut players, &mut wad);
    }
    wi_drawer(&mut v);
    let mid = v.screens[0].clone();
    common::save_palettized_png(
        &common::scratch_dir().join("doommetalrust_intermission_stats.png"),
        320,
        200,
        &mid,
        &playpal,
    );
    assert!(
        mid.iter().filter(|&&b| b != 0).count() > 20_000,
        "full-screen art"
    );

    // press use to skip to the end of the count, release, press: the map
    players[0].cmd.buttons = BT_USE as u8;
    wi_ticker(&mut players, &mut wad);
    players[0].cmd.buttons = 0;
    wi_ticker(&mut players, &mut wad);
    players[0].cmd.buttons = BT_USE as u8;
    wi_ticker(&mut players, &mut wad);
    assert_eq!(wi_mode(), WiMode::ShowNextLoc);
    players[0].cmd.buttons = 0;

    for _ in 0..8 {
        wi_ticker(&mut players, &mut wad);
    }
    wi_drawer(&mut v);
    let map = v.screens[0].clone();
    common::save_palettized_png(
        &common::scratch_dir().join("doommetalrust_intermission_map.png"),
        320,
        200,
        &map,
        &playpal,
    );
    assert_ne!(mid, map);

    // run it out: the map, then the short NoState, then G_WorldDone
    let mut done = false;
    for _ in 0..(4 * TICRATE + 20) {
        if wi_ticker(&mut players, &mut wad) == WiEvent::WorldDone {
            done = true;
            break;
        }
    }
    assert!(done, "the intermission ended by itself");
    doommetal_rust::wi_stuff::wi_end();
}
