//! Phase 11a: multi-player game logic with the real WAD, driven by
//! per-player ticcmds (no network yet): coop spawns and respawns,
//! deathmatch spawns and frags, deathmatch-2 item respawn. (Own test
//! binary, one test: it writes process-wide `doomstat`.)

mod common;

use doommetal_rust::d_event::BT_USE;
use doommetal_rust::d_ticcmd::TicCmd;
use doommetal_rust::doomdef::{GameMode, Skill, MAXPLAYERS};
use doommetal_rust::doomstat;
use doommetal_rust::g_game::{self, GameWorld};
use doommetal_rust::i_video::IVideo;
use doommetal_rust::m_fixed::FRACBITS;
use doommetal_rust::p_inter::p_damage_mobj;
use doommetal_rust::r_main::Renderer;
use doommetal_rust::w_wad::WadFiles;
use doommetal_rust::z_zone::PurgeTag;

fn world() -> Option<(GameWorld, IVideo)> {
    let path = common::find_test_wad()?;
    doommetal_rust::m_argv::set_args(std::env::args().collect());
    std::env::set_var("SDL_VIDEODRIVER", "dummy");
    std::env::set_var("SDL_AUDIODRIVER", "dummy");
    let mut wad = WadFiles::new();
    wad.init_file(path);
    let mut renderer = Renderer::r_init(&mut wad, 10, 0);
    let sprnames = doommetal_rust::info::SPRNAMES;
    renderer
        .rthings
        .r_init_sprites(&wad, &renderer.rdata, &sprnames, false);
    doomstat::state_mut().gamemode = GameMode::Registered;
    let mut video = IVideo::i_init_graphics();
    let playpal = wad.cache_lump_name("PLAYPAL", PurgeTag::Cache).to_vec();
    video.i_set_palette(&playpal);
    doommetal_rust::st_stuff::st_init(&mut wad, &mut video.v_video);
    doommetal_rust::hu_stuff::hu_init(&mut wad);
    doomstat::state_mut().screenblocks = 10;
    doommetal_rust::m_menu::m_init();
    Some((GameWorld::new(wad, renderer), video))
}

fn tick(w: &mut GameWorld, v: &mut IVideo, cmds: [TicCmd; MAXPLAYERS as usize]) {
    g_game::g_ticker_cmds(w, &mut v.v_video, &cmds);
    doomstat::state_mut().gametic += 1;
}

fn forward() -> TicCmd {
    TicCmd {
        forwardmove: 50,
        ..Default::default()
    }
}

fn pos(w: &GameWorld, i: usize) -> (i32, i32) {
    let m = w.thinkers.mobj(w.players[i].mo).expect("player has a mobj");
    (m.x >> FRACBITS, m.y >> FRACBITS)
}

fn kill(w: &mut GameWorld, target: usize, source: Option<usize>) {
    let GameWorld {
        thinkers,
        level,
        players,
        renderer,
        ..
    } = w;
    let t = players[target].mo;
    let s = source.map(|i| players[i].mo);
    p_damage_mobj(
        thinkers,
        level,
        players,
        &mut renderer.rmain,
        t,
        s,
        s,
        10_000,
    );
}

#[test]
fn coop_and_deathmatch_play_with_several_players() {
    let Some((mut w, mut v)) = world() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    // ---- coop: players 1 and 2 ----
    {
        let st = doomstat::state_mut();
        st.playeringame = [true, true, false, false];
        st.netgame = true;
        st.deathmatch = false;
    }
    g_game::g_init_new(&mut w, Skill::Medium, 1, 1);
    assert!(w.players.len() >= 2);
    let starts = doomstat::state().playerstarts;
    #[allow(clippy::needless_range_loop)]
    for i in 0..2 {
        assert_eq!(
            pos(&w, i),
            (starts[i].x as i32, starts[i].y as i32),
            "player {} spawned at its start",
            i + 1
        );
        let m = w.thinkers.mobj(w.players[i].mo).unwrap();
        assert_eq!(m.player, Some(i));
    }
    assert_ne!(w.players[0].mo, w.players[1].mo);
    // no phantom players 3 and 4
    let player_mobjs = w
        .thinkers
        .iter_mobjs()
        .filter(|(_, m)| m.mobj_type == doommetal_rust::info::MobjType::MtPlayer)
        .count();
    assert_eq!(player_mobjs, 2);

    // each player follows its own ticcmd
    let before = [pos(&w, 0), pos(&w, 1)];
    for _ in 0..10 {
        let mut cmds = [TicCmd::default(); 4];
        cmds[0] = forward(); // only player 1 walks
        tick(&mut w, &mut v, cmds);
    }
    assert_ne!(pos(&w, 0), before[0], "player 1 walked");
    assert_eq!(pos(&w, 1), before[1], "player 2 stood still");

    // death and respawn at the start (netgame: no level reload)
    let corpse = w.players[1].mo;
    kill(&mut w, 1, None);
    for _ in 0..40 {
        tick(&mut w, &mut v, [TicCmd::default(); 4]); // let the death anim play
    }
    for _ in 0..3 {
        let mut cmds = [TicCmd::default(); 4];
        cmds[1] = TicCmd {
            buttons: BT_USE as u8,
            ..Default::default()
        };
        tick(&mut w, &mut v, cmds);
    }
    assert_ne!(w.players[1].mo, corpse, "a fresh body was spawned");
    assert_eq!(w.players[1].health, 100);
    assert_eq!(
        pos(&w, 1),
        (starts[1].x as i32, starts[1].y as i32),
        "respawned at its own start"
    );
    assert_eq!(w.bodyqueslot, 1, "the corpse went into the body queue");

    // ---- deathmatch ----
    {
        let st = doomstat::state_mut();
        st.deathmatch = true;
        st.altdeath = true;
    }
    g_game::g_init_new(&mut w, Skill::Medium, 1, 1);
    let dm = doomstat::state().deathmatchstarts;
    let n = doomstat::state().deathmatch_p.unwrap();
    assert!(n >= 4, "E1M1 has deathmatch starts");
    for i in 0..2 {
        let p = pos(&w, i);
        assert!(
            dm[..n].iter().any(|s| (s.x as i32, s.y as i32) == p),
            "player {} spawned at a deathmatch start",
            i + 1
        );
        assert!(
            w.players[i].cards.iter().all(|&c| c),
            "all keys in deathmatch"
        );
    }
    assert_ne!(pos(&w, 0), pos(&w, 1), "not on top of each other");

    // frags: player 1 kills player 2
    kill(&mut w, 1, Some(0));
    assert_eq!(w.players[0].frags[1], 1);
    // an environment death is counted in the victim's own slot (the
    // status bar subtracts that diagonal from the total)
    kill(&mut w, 0, None);
    assert_eq!(w.players[0].frags[0], 1);

    // ---- deathmatch 2: picked-up items come back after 30 seconds ----
    g_game::g_init_new(&mut w, Skill::Medium, 1, 1);
    let item = w
        .thinkers
        .iter_mobjs()
        .find(|(_, m)| {
            m.flags & doommetal_rust::r_defs::mobj_flag::SPECIAL != 0
                && m.flags & doommetal_rust::r_defs::mobj_flag::DROPPED == 0
        })
        .map(|(id, m)| (id, m.spawnpoint, m.mobj_type))
        .expect("E1M1 has pickups");
    let count = |w: &GameWorld, ty| {
        w.thinkers
            .iter_mobjs()
            .filter(|(_, m)| m.mobj_type == ty)
            .count()
    };
    let before = count(&w, item.2);
    doommetal_rust::p_mobj::p_remove_mobj(&mut w.thinkers, &mut w.level, item.0);
    assert_eq!(w.level.itemrespawnque.len(), 1);
    doomstat::state_mut().leveltime = 29 * 35;
    doommetal_rust::p_mobj::p_respawn_specials(&mut w.thinkers, &mut w.level);
    assert_eq!(w.level.itemrespawnque.len(), 1, "not yet");
    doomstat::state_mut().leveltime = 31 * 35;
    doommetal_rust::p_mobj::p_respawn_specials(&mut w.thinkers, &mut w.level);
    assert_eq!(w.level.itemrespawnque.len(), 0, "pulled from the queue");
    // (the removed thinker is swept on the next run; the respawned one
    // is live)
    let live = w
        .thinkers
        .iter_mobjs()
        .filter(|(id, m)| m.mobj_type == item.2 && w.thinkers.is_live(*id))
        .count();
    assert!(live >= before, "the item is back");

    // leave the global state as single player
    let st = doomstat::state_mut();
    st.playeringame = [true, false, false, false];
    st.netgame = false;
    st.deathmatch = false;
    st.altdeath = false;
}
