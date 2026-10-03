//! Phase 9g: the end-of-episode finale with the real art — the typed
//! text over the tiled flat, the episode's end picture, and episode 3's
//! scrolling bunny. (Own test binary: it sets the process-wide
//! `doomstat` game mode/episode.)

mod common;

use doommetal_rust::d_event::GameAction;
use doommetal_rust::d_player::Player;
use doommetal_rust::doomdef::{GameMode, GameState};
use doommetal_rust::doomstat;
use doommetal_rust::f_finale::{
    f_drawer, f_finalecount, f_finalestage, f_start_finale, f_ticker, TEXTSPEED, TEXTWAIT,
};
use doommetal_rust::hu_stuff::hu_init;
use doommetal_rust::v_video::VVideo;
use doommetal_rust::w_wad::WadFiles;
use doommetal_rust::z_zone::PurgeTag;

#[test]
fn episode_finales_type_the_text_then_show_the_end_picture() {
    let Some(path) = common::find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let mut wad = WadFiles::new();
    wad.init_file(path);
    hu_init(&mut wad);
    let playpal = wad.cache_lump_name("PLAYPAL", PurgeTag::Cache).to_vec();
    let mut thinkers = doommetal_rust::p_tick::Thinkers::new();
    let mo = thinkers.add_thinker(
        doommetal_rust::p_tick::ThinkFn::MobjThinker,
        doommetal_rust::p_tick::ThinkerData::Mobj(doommetal_rust::r_defs::Mobj::blank(
            doommetal_rust::info::MobjType::MtPlayer,
        )),
    );
    let players = vec![Player::for_test(mo)];

    for episode in 1..=3 {
        {
            let st = doomstat::state_mut();
            st.gamemode = GameMode::Retail;
            st.gameepisode = episode;
            st.gamemap = 8;
            st.gameaction = GameAction::WorldDone;
        }
        f_start_finale(&mut wad);
        let st = doomstat::state();
        assert_eq!(st.gamestate, GameState::Finale);
        assert_eq!(st.gameaction, GameAction::Nothing);
        assert_eq!((f_finalestage(), f_finalecount()), (0, 0));

        let mut v = VVideo::new();
        let sprites: Vec<doommetal_rust::r_defs::SpriteDef> = Vec::new();

        // partway through the text
        for _ in 0..200 {
            f_ticker(&players, &mut wad);
        }
        f_drawer(&mut v, &mut wad, &sprites, 0);
        let typed = v.screens[0].clone();
        common::save_palettized_png(
            &common::scratch_dir().join(format!("doommetalrust_finale_e{episode}_text.png")),
            320,
            200,
            &typed,
            &playpal,
        );
        assert!(
            typed.iter().all(|&p| p != 0),
            "the flat tiles the whole screen"
        );

        // more text is on screen later than earlier
        let early = typed.clone();
        for _ in 0..200 {
            f_ticker(&players, &mut wad);
        }
        f_drawer(&mut v, &mut wad, &sprites, 0);
        assert_ne!(early, v.screens[0], "the text keeps typing");

        // run to the end of the text: stage 1, counter restarts
        let mut guard = 0;
        while f_finalestage() == 0 {
            f_ticker(&players, &mut wad);
            guard += 1;
            assert!(guard < 6000, "episode {episode}: the text never finished");
        }
        assert_eq!(f_finalestage(), 1);
        assert!(f_finalecount() < 5, "counter reset on the stage change");
        assert!(guard > 3 * TEXTSPEED as usize + TEXTWAIT as usize);
        assert!(doomstat::state().wipegamestate.is_none(), "forces a wipe");

        // the end picture
        f_drawer(&mut v, &mut wad, &sprites, 0);
        common::save_palettized_png(
            &common::scratch_dir().join(format!("doommetalrust_finale_e{episode}_end.png")),
            320,
            200,
            &v.screens[0],
            &playpal,
        );
        assert!(v.screens[0].iter().filter(|&&p| p != 0).count() > 10_000);

        if episode == 3 {
            // the bunny scrolls, then "The End" appears
            let first = v.screens[0].clone();
            for _ in 0..600 {
                f_ticker(&players, &mut wad);
            }
            f_drawer(&mut v, &mut wad, &sprites, 0);
            assert_ne!(first, v.screens[0], "the picture scrolled");
            for _ in 0..600 {
                f_ticker(&players, &mut wad);
            }
            f_drawer(&mut v, &mut wad, &sprites, 0);
            common::save_palettized_png(
                &common::scratch_dir().join("doommetalrust_finale_e3_theend.png"),
                320,
                200,
                &v.screens[0],
                &playpal,
            );
        }
    }
}
