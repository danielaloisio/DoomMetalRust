//! Phase 9c: the heads-up display against the real WAD's font — a
//! pickup message goes from `player.message` to pixels, and a network
//! game's incoming chat (another player's `cmd.chatchar` stream) shows
//! up as a message prefixed with the sender's name.

mod common;

use doommetal_rust::d_player::Player;
use doommetal_rust::doomdef::{GameMode, KEY_ENTER};
use doommetal_rust::doomstat;
use doommetal_rust::hu_stuff::{hu_drawer, hu_init, hu_start, hu_ticker, HU_BROADCAST};
use doommetal_rust::p_tick::{ThinkFn, ThinkerData, Thinkers};
use doommetal_rust::r_defs::Mobj;
use doommetal_rust::v_video::VVideo;
use doommetal_rust::w_wad::WadFiles;

#[test]
fn messages_and_incoming_chat_reach_the_screen() {
    let Some(path) = common::find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let mut wad = WadFiles::new();
    wad.init_file(path);

    {
        let st = doomstat::state_mut();
        st.gamemode = GameMode::Registered;
        st.gameepisode = 1;
        st.gamemap = 1;
        st.consoleplayer = 0;
    }
    hu_init(&mut wad);
    hu_start(); // E1M1's title is built here (HU_TITLE)

    let mut thinkers = Thinkers::new();
    let mk = |t: &mut Thinkers| {
        t.add_thinker(
            ThinkFn::MobjThinker,
            ThinkerData::Mobj(Mobj::blank(doommetal_rust::info::MobjType::MtPlayer)),
        )
    };
    let (a, b) = (mk(&mut thinkers), mk(&mut thinkers));
    let mut players = vec![Player::for_test(a), Player::for_test(b)];
    let ingame = [true, true];

    // 1. A pickup message becomes pixels in the top-left corner.
    let mut v = VVideo::new();
    players[0].message = Some("Picked up a clip.");
    hu_ticker(&mut players, &ingame);
    hu_drawer(&mut v);
    let lit: usize = v.screens[0].iter().filter(|&&p| p != 0).count();
    assert!(lit > 50, "message text was drawn ({lit} pixels)");

    // 2. Network chat from player 2 (index 1): "hi" + Enter, broadcast.
    doomstat::state_mut().netgame = true;
    let mut v2 = VVideo::new();
    let send = |players: &mut Vec<Player>, c: u8| {
        players[1].cmd.chatchar = c;
        hu_ticker(players, &ingame);
    };
    send(&mut players, HU_BROADCAST);
    send(&mut players, b'h');
    send(&mut players, b'i');
    send(&mut players, KEY_ENTER as u8);
    assert_eq!(players[1].cmd.chatchar, 0, "consumed");
    hu_drawer(&mut v2);
    assert!(v2.screens[0].iter().any(|&p| p != 0));
    // The message line now reads "Indigo: HI" (prefix + text): more ink
    // than "Picked up a clip." had is not guaranteed, but the line was
    // replaced, not appended below.
    let sw = 320usize;
    assert!(v2.screens[0][sw * 16..].iter().all(|&p| p == 0));
}
