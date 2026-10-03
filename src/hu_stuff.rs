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
// DESCRIPTION:  Heads-up displays
//
//-----------------------------------------------------------------------------

//! Rust port of `hu_stuff.h` / `hu_stuff.c`. Heads-up displays: the pickup/
//! status message line at the top-left ([`hu_ticker`] moves
//! `player.message` there), the automap's level title, and the
//! multiplayer chat (input line, per-player input buffers, macros).
//!
//! # State
//!
//! The file-scope statics are one [`HuState`] in a `thread_local!`
//! behind C-named free functions ([`hu_init`], [`hu_start`],
//! [`hu_ticker`], [`hu_drawer`], [`hu_erase`], [`hu_responder`]) — the
//! same convention as `s_sound.rs`/`st_stuff.rs`. `plr`
//! (`&players[consoleplayer]`) is taken from the `players` slice each
//! call. The widgets' `boolean* on` pointers become the state's own
//! flags passed at each draw/erase (see `hu_lib`'s docs).
//!
//! # Divergences
//!
//! * `plr->message = lastmessage` (a `static char[]` in C) leaks the
//!   string to get the `&'static str` [`Player::message`] holds —
//!   only chat lines, only in network games.
//! * [`hu_start`] takes the level title from `gameepisode`/`gamemap`
//!   directly like the original; the commented-out Plutonia/TNT
//!   branches (`FIXME` in C) stay unported, so those WADs would show
//!   DOOM II's titles too.
//! * The chat queue functions are free functions over the state, and
//!   [`hu_dequeue_chat_char`] (called from `G_BuildTiccmd`) is a no-op
//!   returning 0 before [`hu_init`].

use std::cell::RefCell;
use std::rc::Rc;

use crate::d_englsh as msg;
use crate::d_event::{EvType, Event};
use crate::d_player::Player;
use crate::doomdef::{
    GameMode, KEY_ENTER, KEY_ESCAPE, KEY_LALT, KEY_RALT, KEY_RSHIFT, MAXPLAYERS, TICRATE,
};
use crate::doomstat;
use crate::hu_lib::{Font, HuIText, HuSText, HuTextLine, ViewWin};
use crate::hu_tables::{ENGLISH_SHIFTXFORM, FRENCH_KEYMAP, FRENCH_SHIFTXFORM, MAPNAMES, MAPNAMES2};
use crate::r_draw::RDraw;
use crate::s_sound::s_start_sound;
use crate::sounds::Sfx;
use crate::v_video::{patch_height, PatchData, VVideo};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// Fonts: the first font character (`HU_FONTSTART`).
pub const HU_FONTSTART: u8 = b'!';
/// The last font character (`HU_FONTEND`).
pub const HU_FONTEND: u8 = b'_';
/// (`HU_FONTSIZE`)
pub const HU_FONTSIZE: usize = (HU_FONTEND - HU_FONTSTART + 1) as usize;

/// Chat destination meaning everybody (`HU_BROADCAST`).
pub const HU_BROADCAST: u8 = 5;

/// (`HU_MSGREFRESH`)
pub const HU_MSGREFRESH: i32 = KEY_ENTER;
pub const HU_MSGX: i32 = 0;
pub const HU_MSGY: i32 = 0;
/// (`HU_MSGWIDTH`) in characters.
pub const HU_MSGWIDTH: i32 = 64;
/// (`HU_MSGHEIGHT`) in lines.
pub const HU_MSGHEIGHT: usize = 1;
/// (`HU_MSGTIMEOUT`)
pub const HU_MSGTIMEOUT: i32 = 4 * TICRATE;

pub const HU_TITLEX: i32 = 0;
/// (`HU_INPUTTOGGLE`)
pub const HU_INPUTTOGGLE: u8 = b't';
pub const HU_INPUTX: i32 = HU_MSGX;
pub const HU_INPUTWIDTH: i32 = 64;
pub const HU_INPUTHEIGHT: i32 = 1;

const QUEUESIZE: usize = 128;

/// The chat macros (`chat_macros[]`).
pub static CHAT_MACROS: [&str; 10] = [
    msg::HUSTR_CHATMACRO0,
    msg::HUSTR_CHATMACRO1,
    msg::HUSTR_CHATMACRO2,
    msg::HUSTR_CHATMACRO3,
    msg::HUSTR_CHATMACRO4,
    msg::HUSTR_CHATMACRO5,
    msg::HUSTR_CHATMACRO6,
    msg::HUSTR_CHATMACRO7,
    msg::HUSTR_CHATMACRO8,
    msg::HUSTR_CHATMACRO9,
];

/// (`player_names[]`)
pub static PLAYER_NAMES: [&str; 4] = [
    msg::HUSTR_PLRGREEN,
    msg::HUSTR_PLRINDIGO,
    msg::HUSTR_PLRBROWN,
    msg::HUSTR_PLRRED,
];

/// Everything `hu_stuff.c` keeps in file scope.
struct HuState {
    hu_font: Font,
    shiftxform: &'static [u8; 128],

    w_title: HuTextLine,
    /// (`chat_on`)
    chat_on: bool,
    w_chat: HuIText,
    chat_dest: [u8; MAXPLAYERS as usize],
    w_inputbuffer: Vec<HuIText>,

    message_on: bool,
    /// (`message_dontfuckwithme`, non-static in C: the menu sets it.)
    message_dontfuckwithme: bool,
    message_nottobefuckedwith: bool,
    w_message: HuSText,
    message_counter: i32,
    headsupactive: bool,

    // chat queue
    chatchars: [u8; QUEUESIZE],
    head: usize,
    tail: usize,

    // HU_Responder's statics
    shiftdown: bool,
    altdown: bool,
    num_nobrainers: i32,
}

thread_local! {
    static HU: RefCell<Option<HuState>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut HuState) -> R) -> R {
    HU.with(|s| f(s.borrow_mut().as_mut().expect("HU_Init has not run")))
}

/// Whether [`hu_init`] has run on this thread.
pub fn hu_is_initialized() -> bool {
    HU.with(|s| s.borrow().is_some())
}

/// Drops the heads-up state (tests, shutdown).
pub fn hu_shutdown() {
    HU.with(|s| *s.borrow_mut() = None);
}

fn empty_font() -> Font {
    Rc::from(Vec::<PatchData>::new())
}

/// Port of `HU_Init`: picks the shift table and loads the font
/// (`STCFN033`..`STCFN095`).
pub fn hu_init(wad: &mut WadFiles) {
    let shiftxform = if doomstat::state().french {
        &FRENCH_SHIFTXFORM
    } else {
        &ENGLISH_SHIFTXFORM
    };

    // load the heads-up font
    let font: Vec<PatchData> = (0..HU_FONTSIZE)
        .map(|i| {
            let name = format!("STCFN{:03}", HU_FONTSTART as usize + i);
            Rc::from(wad.cache_lump_name(&name, PurgeTag::Static))
        })
        .collect();
    let font: Font = font.into();

    HU.with(|s| {
        *s.borrow_mut() = Some(HuState {
            hu_font: font.clone(),
            shiftxform,
            w_title: HuTextLine::new(0, 0, font.clone(), HU_FONTSTART as i32),
            chat_on: false,
            w_chat: HuIText::new(HU_INPUTX, 0, font.clone(), HU_FONTSTART as i32),
            chat_dest: [0; MAXPLAYERS as usize],
            w_inputbuffer: Vec::new(),
            message_on: false,
            message_dontfuckwithme: false,
            message_nottobefuckedwith: false,
            w_message: HuSText::new(HU_MSGX, HU_MSGY, HU_MSGHEIGHT, font, HU_FONTSTART as i32),
            message_counter: 0,
            headsupactive: false,
            chatchars: [0; QUEUESIZE],
            head: 0,
            tail: 0,
            shiftdown: false,
            altdown: false,
            num_nobrainers: 0,
        });
    });
}

/// `hu_font[]`, the heads-up font (`m_menu`/`m_misc` draw text with it).
///
/// # Panics
/// Before [`hu_init`].
pub fn hu_font() -> Font {
    with(|s| s.hu_font.clone())
}

/// Port of `HU_Stop`.
pub fn hu_stop() {
    with(|s| s.headsupactive = false);
}

/// Port of `HU_Start`: sets up the message line, the level title and
/// the chat widgets for a level.
pub fn hu_start() {
    if with(|s| s.headsupactive) {
        hu_stop();
    }

    let st = doomstat::state();
    let (gamemode, episode, map) = (st.gamemode, st.gameepisode, st.gamemap);

    with(|s| {
        s.message_on = false;
        s.message_dontfuckwithme = false;
        s.message_nottobefuckedwith = false;
        s.chat_on = false;

        // create the message widget
        s.w_message = HuSText::new(
            HU_MSGX,
            HU_MSGY,
            HU_MSGHEIGHT,
            s.hu_font.clone(),
            HU_FONTSTART as i32,
        );

        // create the map title widget
        let title_y = 167 - patch_height(&s.hu_font[0]);
        s.w_title = HuTextLine::new(HU_TITLEX, title_y, s.hu_font.clone(), HU_FONTSTART as i32);

        let title = level_title(gamemode, episode, map);
        for b in title.bytes() {
            s.w_title.add_char(b);
        }

        // create the chat widget
        let input_y = HU_MSGY + HU_MSGHEIGHT as i32 * (patch_height(&s.hu_font[0]) + 1);
        s.w_chat = HuIText::new(HU_INPUTX, input_y, s.hu_font.clone(), HU_FONTSTART as i32);

        // create the inputbuffer widgets
        s.w_inputbuffer = (0..MAXPLAYERS)
            .map(|_| HuIText::new(0, 0, empty_font(), 0))
            .collect();

        s.headsupactive = true;
    });
}

/// `HU_TITLE`/`HU_TITLE2`: the automap's level name for the game mode
/// (the Plutonia/TNT branches are `FIXME`d out in the original).
fn level_title(gamemode: GameMode, episode: i32, map: i32) -> &'static str {
    match gamemode {
        GameMode::Shareware | GameMode::Registered | GameMode::Retail => {
            MAPNAMES[((episode - 1) * 9 + map - 1) as usize]
        }
        _ => MAPNAMES2[(map - 1) as usize],
    }
}

/// Port of `HU_Drawer`.
pub fn hu_drawer(v: &mut VVideo) {
    with(|s| {
        s.w_message.draw(s.message_on, v);
        s.w_chat.draw(s.chat_on, v);
        if doomstat::state().automapactive {
            s.w_title.draw(false, v);
        }
    });
}

/// Port of `HU_Erase`.
pub fn hu_erase(view: &ViewWin, rdraw: &RDraw, v: &mut VVideo) {
    with(|s| {
        s.w_message.erase(s.message_on, view, rdraw, v);
        s.w_chat.erase(s.chat_on, view, rdraw, v);
        s.w_title.erase(view, rdraw, v);
    });
}

/// Port of `HU_Ticker`. `players[consoleplayer]` is `plr`;
/// `playeringame` is the original's global of the same name.
pub fn hu_ticker(players: &mut [Player], playeringame: &[bool]) {
    let consoleplayer = doomstat::state().consoleplayer as usize;
    let show_messages = doomstat::state().show_messages != 0;
    let netgame = doomstat::state().netgame;
    let gamemode = doomstat::state().gamemode;

    with(|s| {
        // tick down message counter if message is up
        if s.message_counter != 0 {
            s.message_counter -= 1;
            if s.message_counter == 0 {
                s.message_on = false;
                s.message_nottobefuckedwith = false;
            }
        }

        if show_messages || s.message_dontfuckwithme {
            // display message if necessary
            if let Some(message) = players[consoleplayer].message {
                if !s.message_nottobefuckedwith || s.message_dontfuckwithme {
                    s.w_message.add_message(None, message);
                    players[consoleplayer].message = None;
                    s.message_on = true;
                    s.message_counter = HU_MSGTIMEOUT;
                    s.message_nottobefuckedwith = s.message_dontfuckwithme;
                    s.message_dontfuckwithme = false;
                }
            }
        } // else message_on = false;

        // check for incoming chat characters
        if netgame {
            for i in 0..MAXPLAYERS as usize {
                if !playeringame.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let c = players[i].cmd.chatchar;
                if i != consoleplayer && c != 0 {
                    if c <= HU_BROADCAST {
                        s.chat_dest[i] = c;
                    } else {
                        let mut c = c;
                        if c.is_ascii_lowercase() {
                            c = s.shiftxform[c as usize];
                        }
                        let rc = s.w_inputbuffer[i].key_in(c);
                        if rc && c as i32 == KEY_ENTER {
                            if !s.w_inputbuffer[i].l.l.is_empty()
                                && (s.chat_dest[i] as usize == consoleplayer + 1
                                    || s.chat_dest[i] == HU_BROADCAST)
                            {
                                let text =
                                    String::from_utf8_lossy(&s.w_inputbuffer[i].l.l).into_owned();
                                s.w_message.add_message(Some(PLAYER_NAMES[i]), &text);

                                s.message_nottobefuckedwith = true;
                                s.message_on = true;
                                s.message_counter = HU_MSGTIMEOUT;
                                if gamemode == GameMode::Commercial {
                                    s_start_sound(None, Sfx::SfxRadio);
                                } else {
                                    s_start_sound(None, Sfx::SfxTink);
                                }
                            }
                            s.w_inputbuffer[i].reset();
                        }
                    }
                    players[i].cmd.chatchar = 0;
                }
            }
        }
    });
}

/// Port of `HU_queueChatChar`. A full queue drops the character and
/// tells the player (`plr->message = HUSTR_MSGU`).
pub fn hu_queue_chat_char(c: u8, plr: &mut Player) {
    with(|s| queue_in(s, c, plr));
}

fn queue_in(s: &mut HuState, c: u8, plr: &mut Player) {
    if ((s.head + 1) & (QUEUESIZE - 1)) == s.tail {
        plr.message = Some(msg::HUSTR_MSGU);
    } else {
        s.chatchars[s.head] = c;
        s.head = (s.head + 1) & (QUEUESIZE - 1);
    }
}

/// Port of `HU_dequeueChatChar`. 0 when the queue is empty (and before
/// [`hu_init`]).
pub fn hu_dequeue_chat_char() -> u8 {
    if !hu_is_initialized() {
        return 0;
    }
    with(|s| {
        if s.head != s.tail {
            let c = s.chatchars[s.tail];
            s.tail = (s.tail + 1) & (QUEUESIZE - 1);
            c
        } else {
            0
        }
    })
}

/// `message_dontfuckwithme` (the menu sets it to force its own
/// messages through).
pub fn hu_set_message_dontfuckwithme(on: bool) {
    with(|s| s.message_dontfuckwithme = on);
}

/// (`chat_on`)
pub fn hu_chat_on() -> bool {
    with(|s| s.chat_on)
}

/// `french`'s `ForeignTranslation`.
fn foreign_translation(ch: u8) -> u8 {
    if ch < 128 {
        FRENCH_KEYMAP[ch as usize]
    } else {
        ch
    }
}

/// Port of `HU_Responder`. Returns true when the key was eaten.
pub fn hu_responder(ev: &Event, players: &mut [Player], playeringame: &[bool]) -> bool {
    let consoleplayer = doomstat::state().consoleplayer as usize;
    let netgame = doomstat::state().netgame;
    let french = doomstat::state().french;
    let mut eatkey = false;

    let numplayers = playeringame.iter().filter(|&&p| p).count();

    with(|s| {
        if ev.data1 == KEY_RSHIFT {
            s.shiftdown = ev.event_type == EvType::KeyDown;
            return false;
        } else if ev.data1 == KEY_RALT || ev.data1 == KEY_LALT {
            s.altdown = ev.event_type == EvType::KeyDown;
            return false;
        }

        if ev.event_type != EvType::KeyDown {
            return false;
        }

        let destination_keys = [
            msg::HUSTR_KEYGREEN,
            msg::HUSTR_KEYINDIGO,
            msg::HUSTR_KEYBROWN,
            msg::HUSTR_KEYRED,
        ];

        if !s.chat_on {
            if ev.data1 == HU_MSGREFRESH {
                s.message_on = true;
                s.message_counter = HU_MSGTIMEOUT;
                eatkey = true;
            } else if netgame && ev.data1 == HU_INPUTTOGGLE as i32 {
                eatkey = true;
                s.chat_on = true;
                s.w_chat.reset();
                queue_in(s, HU_BROADCAST, &mut players[consoleplayer]);
            } else if netgame && numplayers > 2 {
                for (i, &dest_key) in destination_keys.iter().enumerate() {
                    if ev.data1 == dest_key as i32 {
                        if playeringame.get(i).copied().unwrap_or(false) && i != consoleplayer {
                            eatkey = true;
                            s.chat_on = true;
                            s.w_chat.reset();
                            queue_in(s, i as u8 + 1, &mut players[consoleplayer]);
                            break;
                        } else if i == consoleplayer {
                            s.num_nobrainers += 1;
                            players[consoleplayer].message = Some(if s.num_nobrainers < 3 {
                                msg::HUSTR_TALKTOSELF1
                            } else if s.num_nobrainers < 6 {
                                msg::HUSTR_TALKTOSELF2
                            } else if s.num_nobrainers < 9 {
                                msg::HUSTR_TALKTOSELF3
                            } else if s.num_nobrainers < 32 {
                                msg::HUSTR_TALKTOSELF4
                            } else {
                                msg::HUSTR_TALKTOSELF5
                            });
                        }
                    }
                }
            }
        } else {
            let mut c = ev.data1 as u8;
            // send a macro
            if s.altdown {
                let m = c.wrapping_sub(b'0');
                if m > 9 {
                    return false;
                }
                let macromessage = CHAT_MACROS[m as usize];

                // kill last message with a '\n'
                queue_in(s, KEY_ENTER as u8, &mut players[consoleplayer]); // DEBUG!!!

                // send the macro message
                for b in macromessage.bytes() {
                    queue_in(s, b, &mut players[consoleplayer]);
                }
                queue_in(s, KEY_ENTER as u8, &mut players[consoleplayer]);

                // leave chat mode and notify that it was sent
                s.chat_on = false;
                players[consoleplayer].message = Some(macromessage);
                eatkey = true;
            } else {
                if french {
                    c = foreign_translation(c);
                }
                if (s.shiftdown || c.is_ascii_lowercase()) && c < 128 {
                    c = s.shiftxform[c as usize];
                }
                eatkey = s.w_chat.key_in(c);
                if eatkey {
                    // static unsigned char buf[20]; // DEBUG
                    queue_in(s, c, &mut players[consoleplayer]);
                }
                if c as i32 == KEY_ENTER {
                    s.chat_on = false;
                    if !s.w_chat.l.l.is_empty() {
                        let text = String::from_utf8_lossy(&s.w_chat.l.l).into_owned();
                        // See the module docs on why this is leaked.
                        players[consoleplayer].message = Some(Box::leak(text.into_boxed_str()));
                    }
                } else if c as i32 == KEY_ESCAPE {
                    s.chat_on = false;
                }
            }
        }
        eatkey
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wad() -> Option<WadFiles> {
        let path = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut w = WadFiles::new();
        w.init_file(path);
        Some(w)
    }

    /// A fresh heads-up state with the real font and its widgets set up
    /// as `HU_Start` does (minus the level title, which needs a level).
    fn fixture() -> Option<Player> {
        let mut w = wad()?;
        hu_shutdown();
        hu_init(&mut w);
        with(|s| {
            s.w_inputbuffer = (0..MAXPLAYERS)
                .map(|_| HuIText::new(0, 0, empty_font(), 0))
                .collect();
            s.headsupactive = true;
        });
        let mut thinkers = crate::p_tick::Thinkers::new();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj(crate::r_defs::Mobj::blank(
                crate::info::MobjType::MtPlayer,
            )),
        );
        Some(Player::for_test(mo))
    }

    fn key(c: i32, event_type: EvType) -> Event {
        Event {
            event_type,
            data1: c,
            data2: 0,
            data3: 0,
        }
    }

    #[test]
    fn level_titles_by_game_mode() {
        assert_eq!(level_title(GameMode::Shareware, 1, 1), msg::HUSTR_E1M1);
        assert_eq!(level_title(GameMode::Retail, 4, 9), msg::HUSTR_E4M9);
        assert_eq!(level_title(GameMode::Registered, 2, 3), msg::HUSTR_E2M3);
        assert_eq!(level_title(GameMode::Commercial, 1, 5), msg::HUSTR_5);
        // "NEWLEVEL" placeholders past episode 4
        assert_eq!(MAPNAMES[36], "NEWLEVEL");
    }

    #[test]
    fn font_and_tables_load() {
        let Some(_p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        with(|s| {
            assert_eq!(s.hu_font.len(), HU_FONTSIZE);
            assert_eq!(s.shiftxform[b'a' as usize], b'A');
            assert_eq!(s.shiftxform[b'1' as usize], b'!'); // shift-1
        });
        assert_eq!(ENGLISH_SHIFTXFORM[0], 0);
        assert_eq!(FRENCH_SHIFTXFORM[b'a' as usize], b'A');
    }

    #[test]
    fn a_player_message_moves_to_the_message_line_and_times_out() {
        let Some(p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut players = vec![p];
        players[0].message = Some("Picked up a clip.");
        hu_ticker(&mut players, &[true]);

        assert_eq!(players[0].message, None, "consumed");
        with(|s| {
            assert!(s.message_on);
            assert_eq!(s.message_counter, HU_MSGTIMEOUT);
            assert_eq!(s.w_message.l[s.w_message.cl].l, b"Picked up a clip.");
        });

        // it stays up for HU_MSGTIMEOUT tics, then goes off
        for _ in 0..HU_MSGTIMEOUT - 1 {
            hu_ticker(&mut players, &[true]);
            assert!(with(|s| s.message_on));
        }
        hu_ticker(&mut players, &[true]);
        assert!(!with(|s| s.message_on));
    }

    #[test]
    fn a_protected_message_blocks_ordinary_ones_until_it_times_out() {
        let Some(p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut players = vec![p];
        // The menu's "dontfuckwithme" message goes through and protects
        // itself.
        hu_set_message_dontfuckwithme(true);
        players[0].message = Some("Menu message");
        hu_ticker(&mut players, &[true]);
        with(|s| {
            assert!(s.message_on);
            assert!(s.message_nottobefuckedwith);
            assert!(!s.message_dontfuckwithme);
        });

        // an ordinary message now waits (it isn't consumed)
        players[0].message = Some("Picked up a clip.");
        hu_ticker(&mut players, &[true]);
        assert_eq!(players[0].message, Some("Picked up a clip."));
        with(|s| assert_eq!(s.w_message.l[s.w_message.cl].l, b"Menu message"));

        // after the timeout it is free to show
        for _ in 0..HU_MSGTIMEOUT {
            hu_ticker(&mut players, &[true]);
        }
        hu_ticker(&mut players, &[true]);
        assert_eq!(players[0].message, None);
        with(|s| assert_eq!(s.w_message.l[s.w_message.cl].l, b"Picked up a clip."));
    }

    #[test]
    fn drawer_only_paints_text_while_the_message_is_on() {
        let Some(p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut players = vec![p];
        let mut v = VVideo::new();
        hu_drawer(&mut v);
        assert!(v.screens[0].iter().all(|&b| b == 0));

        players[0].message = Some("HELLO");
        hu_ticker(&mut players, &[true]);
        hu_drawer(&mut v);
        assert!(v.screens[0].iter().any(|&b| b != 0));
        // all in the top rows (the message line sits at y=0)
        let sw = crate::doomdef::SCREENWIDTH as usize;
        assert!(v.screens[0][sw * 16..].iter().all(|&b| b == 0));
    }

    #[test]
    fn enter_refreshes_the_message_line() {
        let Some(p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut players = vec![p];
        assert!(hu_responder(
            &key(KEY_ENTER, EvType::KeyDown),
            &mut players,
            &[true]
        ));
        with(|s| {
            assert!(s.message_on);
            assert_eq!(s.message_counter, HU_MSGTIMEOUT);
        });
        // key-ups and other keys aren't eaten
        assert!(!hu_responder(
            &key(KEY_ENTER, EvType::KeyUp),
            &mut players,
            &[true]
        ));
        assert!(!hu_responder(
            &key(b'x' as i32, EvType::KeyDown),
            &mut players,
            &[true]
        ));
    }

    #[test]
    fn chat_typing_queues_characters_and_enter_sends_it() {
        let Some(p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut players = vec![p];
        with(|s| s.chat_on = true); // (`t` opens chat only in a netgame)
        assert!(hu_responder(
            &key(b'h' as i32, EvType::KeyDown),
            &mut players,
            &[true]
        ));
        assert!(hu_responder(
            &key(b'i' as i32, EvType::KeyDown),
            &mut players,
            &[true]
        ));
        assert!(!hu_responder(
            &key(1, EvType::KeyDown),
            &mut players,
            &[true]
        ));
        with(|s| assert_eq!(s.w_chat.l.l, b"HI"));

        assert!(hu_responder(
            &key(KEY_ENTER, EvType::KeyDown),
            &mut players,
            &[true]
        ));
        assert!(!with(|s| s.chat_on));
        assert_eq!(players[0].message, Some("HI"));

        // what the game loop will put in the ticcmds, in order
        assert_eq!(hu_dequeue_chat_char(), b'H');
        assert_eq!(hu_dequeue_chat_char(), b'I');
        assert_eq!(hu_dequeue_chat_char(), KEY_ENTER as u8);
        assert_eq!(hu_dequeue_chat_char(), 0);
    }

    #[test]
    fn escape_cancels_chat_and_alt_digit_sends_a_macro() {
        let Some(p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut players = vec![p];
        with(|s| s.chat_on = true);
        hu_responder(&key(b'a' as i32, EvType::KeyDown), &mut players, &[true]);
        hu_responder(&key(KEY_ESCAPE, EvType::KeyDown), &mut players, &[true]);
        assert!(!with(|s| s.chat_on));
        assert_eq!(players[0].message, None);
        while hu_dequeue_chat_char() != 0 {}

        with(|s| s.chat_on = true);
        hu_responder(&key(KEY_RALT, EvType::KeyDown), &mut players, &[true]);
        assert!(hu_responder(
            &key(b'0' as i32, EvType::KeyDown),
            &mut players,
            &[true]
        ));
        assert!(!with(|s| s.chat_on));
        assert_eq!(players[0].message, Some(CHAT_MACROS[0]));
        let mut sent = Vec::new();
        loop {
            let c = hu_dequeue_chat_char();
            if c == 0 {
                break;
            }
            sent.push(c);
        }
        let mut want = vec![KEY_ENTER as u8];
        want.extend_from_slice(CHAT_MACROS[0].as_bytes());
        want.push(KEY_ENTER as u8);
        assert_eq!(sent, want);

        // a non-digit with alt held is not a macro
        with(|s| s.chat_on = true);
        assert!(!hu_responder(
            &key(b'z' as i32, EvType::KeyDown),
            &mut players,
            &[true]
        ));
    }

    #[test]
    fn a_full_chat_queue_drops_characters_and_warns() {
        let Some(p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut plr = p;
        for _ in 0..QUEUESIZE - 1 {
            hu_queue_chat_char(b'x', &mut plr);
        }
        assert_eq!(plr.message, None);
        hu_queue_chat_char(b'y', &mut plr);
        assert_eq!(plr.message, Some(msg::HUSTR_MSGU));
        let mut n = 0;
        while hu_dequeue_chat_char() != 0 {
            n += 1;
        }
        assert_eq!(n, QUEUESIZE - 1, "the 128th was dropped");
    }

    #[test]
    fn shift_upper_cases_and_uses_the_shift_table() {
        let Some(p) = fixture() else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut players = vec![p];
        with(|s| s.chat_on = true);
        hu_responder(&key(KEY_RSHIFT, EvType::KeyDown), &mut players, &[true]);
        hu_responder(&key(b'1' as i32, EvType::KeyDown), &mut players, &[true]);
        hu_responder(&key(b',' as i32, EvType::KeyDown), &mut players, &[true]);
        hu_responder(&key(KEY_RSHIFT, EvType::KeyUp), &mut players, &[true]);
        hu_responder(&key(b',' as i32, EvType::KeyDown), &mut players, &[true]);
        with(|s| assert_eq!(s.w_chat.l.l, b"!<,"));
    }
}
