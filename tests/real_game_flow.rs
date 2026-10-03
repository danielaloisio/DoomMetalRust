//! Phase 9h: the real game flow with the real WAD — the pieces
//! `g_ticker` drives: spawning/linking the player, dying and being
//! reborn (the level reloads), finishing a level (intermission, then
//! the next map with the inventory kept), secret exits, the episode
//! finale, and starting a new game from `idclev`/the menu. (Own test
//! binary: it writes process-wide `doomstat`.)

mod common;

use doommetal_rust::d_event::{GameAction, BT_USE};
use doommetal_rust::d_player::PlayerState;
use doommetal_rust::d_ticcmd::TicCmd;
use doommetal_rust::doomdef::{AmmoType, GameMode, GameState, Skill, WeaponType};
use doommetal_rust::doomstat;
use doommetal_rust::g_game::{self, GameWorld};
use doommetal_rust::r_main::Renderer;
use doommetal_rust::v_video::VVideo;
use doommetal_rust::w_wad::WadFiles;

fn world() -> Option<(GameWorld, VVideo)> {
    let path = common::find_test_wad()?;
    doommetal_rust::m_argv::set_args(std::env::args().collect());
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

fn tick(w: &mut GameWorld, v: &mut VVideo, buttons: u8) {
    let cmd = TicCmd {
        buttons,
        ..Default::default()
    };
    g_game::g_ticker(w, v, cmd);
    doomstat::state_mut().gametic += 1;
}

#[test]
fn the_game_flows_from_level_to_level_death_finale_and_new_game() {
    let Some((mut w, mut v)) = world() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    // ---- a new game on E1M1: player spawned, linked, level set up ----
    g_game::g_init_new(&mut w, Skill::Medium, 1, 1);
    let st = doomstat::state();
    assert_eq!((st.gameepisode, st.gamemap), (1, 1));
    assert_eq!(st.gamestate, GameState::Level);
    assert!(
        st.totalkills > 0 && st.totalitems > 0,
        "P_LoadThings counted"
    );
    assert!(
        st.totalsecret > 0,
        "P_SpawnSpecials counted the secret sectors"
    );
    assert_eq!(w.players.len(), 1);
    let p = &w.players[0];
    assert_eq!(p.playerstate, PlayerState::Live);
    assert_eq!((p.health, p.ammo[AmmoType::AmClip as usize]), (100, 50));
    assert!(p.weaponowned[WeaponType::WpPistol as usize]);
    assert_eq!(w.thinkers.mobj(p.mo).unwrap().player, Some(0));
    assert_eq!(w.thinkers.mobj(p.mo).unwrap().health, 100);

    // ---- death and rebirth: USE after dying reloads the level ----
    {
        let mo = w.players[0].mo;
        let g_game::GameWorld {
            thinkers,
            level,
            players,
            renderer,
            ..
        } = &mut w;
        doommetal_rust::p_inter::p_damage_mobj(
            thinkers,
            level,
            players,
            &mut renderer.rmain,
            mo,
            None,
            None,
            1000,
        );
    }
    assert_eq!(w.players[0].playerstate, PlayerState::Dead);
    // spend the pistol ammo and pick up a weapon: rebirth must reset it
    w.players[0].ammo[AmmoType::AmClip as usize] = 3;
    w.players[0].weaponowned[WeaponType::WpShotgun as usize] = true;
    let dead_mo = w.players[0].mo;
    for _ in 0..40 {
        tick(&mut w, &mut v, 0);
    }
    assert_eq!(w.players[0].playerstate, PlayerState::Dead, "still dead");
    tick(&mut w, &mut v, BT_USE as u8); // P_DeathThink: use = reborn
                                        // (the reborn is noticed and the level reloads within a tic or two)
    for _ in 0..3 {
        tick(&mut w, &mut v, 0);
    }
    let p = &w.players[0];
    assert_eq!(p.playerstate, PlayerState::Live);
    assert_eq!((p.health, p.ammo[AmmoType::AmClip as usize]), (100, 50));
    assert!(
        !p.weaponowned[WeaponType::WpShotgun as usize],
        "inventory reset"
    );
    // (a fresh arena: the new mobj can reuse the old one's id)
    let _ = dead_mo;
    assert_eq!(
        w.thinkers.mobj(p.mo).unwrap().health,
        100,
        "a live mobj on a reloaded level"
    );
    assert_eq!(doomstat::state().gameaction, GameAction::Nothing);

    // ---- finishing E1M1: intermission, then E1M2 with the inventory ----
    w.players[0].weaponowned[WeaponType::WpShotgun as usize] = true;
    w.players[0].ammo[AmmoType::AmShell as usize] = 12;
    w.players[0].killcount = 7;
    g_game::g_exit_level();
    tick(&mut w, &mut v, 0);
    assert_eq!(doomstat::state().gamestate, GameState::Intermission);
    assert!(doommetal_rust::wi_stuff::wi_is_active());
    let wm = doomstat::state().wminfo;
    assert_eq!((wm.epsd, wm.last, wm.next), (0, 0, 1));
    assert_eq!(wm.plyr[0].skills, 7);
    assert_eq!(wm.maxkills, doomstat::state().totalkills);
    assert_eq!(wm.partime, 35 * 30);
    // skip the counting and the map with USE presses (each needs a
    // release in between)
    let mut guard = 0;
    while doomstat::state().gamestate == GameState::Intermission {
        tick(
            &mut w,
            &mut v,
            if guard % 2 == 0 { BT_USE as u8 } else { 0 },
        );
        guard += 1;
        assert!(guard < 2000, "the intermission never ended");
    }
    assert!(!doommetal_rust::wi_stuff::wi_is_active());
    let st = doomstat::state();
    assert_eq!((st.gamestate, st.gamemap), (GameState::Level, 2));
    let p = &w.players[0];
    assert!(
        p.weaponowned[WeaponType::WpShotgun as usize],
        "weapons carry over"
    );
    assert_eq!(p.ammo[AmmoType::AmShell as usize], 12);
    assert_eq!(p.playerstate, PlayerState::Live);
    assert_eq!(p.killcount, 0, "per-level counters restart");

    // ---- secret exit: E1M3 -> E1M9 (wminfo.next = 8) ----
    g_game::g_init_new(&mut w, Skill::Medium, 1, 3);
    g_game::g_secret_exit_level(&w.wad, GameMode::Registered);
    tick(&mut w, &mut v, 0);
    assert_eq!(doomstat::state().wminfo.next, 8);
    doommetal_rust::wi_stuff::wi_end();

    // ---- the last map of the episode goes to the finale ----
    g_game::g_init_new(&mut w, Skill::Medium, 1, 8);
    g_game::g_exit_level();
    tick(&mut w, &mut v, 0);
    assert_eq!(doomstat::state().gamestate, GameState::Finale);
    assert_eq!(doommetal_rust::f_finale::f_finalestage(), 0);

    // ---- idclev / the menu: a deferred new game ----
    g_game::g_defered_init_new(Skill::Hard, 2, 1);
    tick(&mut w, &mut v, 0);
    let st = doomstat::state();
    assert_eq!(
        (st.gamestate, st.gameepisode, st.gamemap, st.gameskill),
        (GameState::Level, 2, 1, Skill::Hard)
    );
    assert_eq!(w.players[0].playerstate, PlayerState::Live);
    assert_eq!(
        w.players[0].health, 100,
        "a new game: the inventory is reborn too"
    );
}
