//! Phase 9h2: the title/credit page loop, game-mode detection and the
//! startup arguments. (Own test binary: it mutates `doomstat`.)

mod common;

use doommetal_rust::d_main::{
    d_advancedemo_pending, d_do_advance_demo, d_page_drawer, d_page_ticker, d_start_title,
    identify_version, StartArgs,
};
use doommetal_rust::doomdef::{GameMode, GameState, Skill};
use doommetal_rust::doomstat;
use doommetal_rust::g_game::GameWorld;
use doommetal_rust::r_main::Renderer;
use doommetal_rust::v_video::VVideo;
use doommetal_rust::w_wad::WadFiles;

fn args(list: &[&str]) -> Vec<String> {
    std::iter::once("doom")
        .chain(list.iter().copied())
        .map(String::from)
        .collect()
}

#[test]
fn startup_arguments_are_parsed_like_d_doom_main() {
    let none = StartArgs::parse(&args(&[]), GameMode::Retail);
    assert!(!none.autostart);
    let warp = StartArgs::parse(&args(&["-warp", "2", "3"]), GameMode::Retail);
    assert_eq!((warp.episode, warp.map, warp.autostart), (2, 3, true));
    let warp2 = StartArgs::parse(&args(&["-warp", "7"]), GameMode::Commercial);
    assert_eq!((warp2.episode, warp2.map), (1, 7));
    let skill = StartArgs::parse(&args(&["-skill", "4", "-episode", "3"]), GameMode::Retail);
    assert_eq!((skill.skill, skill.episode, skill.map), (Skill::Hard, 3, 1));
    // a flag with no value is ignored
    assert!(!StartArgs::parse(&args(&["-warp"]), GameMode::Retail).autostart);
}

#[test]
fn title_pages_cycle_and_draw_the_real_art() {
    let Some(path) = common::find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let mut wad = WadFiles::new();
    wad.init_file(path);
    assert_eq!(identify_version(&wad), GameMode::Retail);
    doomstat::state_mut().gamemode = GameMode::Retail;

    let renderer = Renderer::r_init(&mut wad, 11, 0);
    let mut world = GameWorld::new(wad, renderer);

    d_start_title();
    assert!(d_advancedemo_pending());
    d_do_advance_demo(&mut world);
    assert!(!d_advancedemo_pending());
    assert_eq!(doomstat::state().gamestate, GameState::DemoScreen);

    // TITLEPIC lasts 170 tics, then the credits (200), then title again
    let mut v = VVideo::new();
    d_page_drawer(&mut v, &mut world.wad);
    let title: Vec<u8> = v.screens[0].clone();
    assert!(title.iter().any(|&b| b != 0));

    for _ in 0..170 {
        d_page_ticker();
        assert!(!d_advancedemo_pending());
    }
    d_page_ticker();
    assert!(d_advancedemo_pending());
    d_do_advance_demo(&mut world);
    d_page_drawer(&mut v, &mut world.wad);
    assert_ne!(v.screens[0], title, "credit page differs from the title");

    for _ in 0..200 {
        d_page_ticker();
    }
    d_page_ticker();
    d_do_advance_demo(&mut world); // sequence 4 (Retail: CREDIT again)
    d_page_ticker();
    for _ in 0..210 {
        d_page_ticker();
    }
    d_do_advance_demo(&mut world); // wraps to 0: the title
    d_page_drawer(&mut v, &mut world.wad);
    assert_eq!(v.screens[0], title);
}
