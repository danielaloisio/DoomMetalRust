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
// DESCRIPTION:
//	DOOM selection menu, options, episode etc.
//	Sliders and icons. Kinda widget stuff.
//
//-----------------------------------------------------------------------------

//! Rust port of `m_menu.h` / `m_menu.c`. The DOOM menu: main, new game
//! (episode/skill), options, sound volume, read-this screens, quit and
//! end-game confirmations, and the load/save slots.
//!
//! # Representation
//!
//! The original wires `menu_t`/`menuitem_t` tables together with
//! function pointers (`routine`, the draw routines) and `menu_t*`
//! links. Here menus are [`MenuId`]s into a table, item routines are
//! the [`Action`] enum and draw routines [`DrawFn`], and a pending
//! message's response routine is a [`MsgResponse`] — the same
//! dispatch, without pointers. The file-scope statics live in one
//! [`MenuState`] in a `thread_local!` behind C-named free functions
//! ([`m_init`], [`m_responder`], [`m_ticker`], [`m_drawer`], ...), like
//! `st_stuff.rs`/`hu_stuff.rs`. `menuactive` is `doomstat`'s.
//!
//! What the menu touches outside itself comes in a [`MenuCtx`]
//! (players, WAD, video buffer, renderer) and, for the gamma palette
//! change, goes back out through [`MenuCtx::palette`].
//!
//! # Divergences
//!
//! * The save/load slot menus, the save-name editor and
//!   `M_LoadSelect`/`M_DoSave` call `g_load_game`/`g_save_game`
//!   (Phase 10); the slot files are `doomsav0.dsg`.. in the current
//!   directory, in this port's own format (see `p_saveg`).
//! * `I_Quit` becomes `doomstat.quit_requested`, which the main loop
//!   acts on (after the quit sound had a chance to start).
//! * The `-devparm` F1 screenshot writes the PCX right away instead of
//!   going through `gameaction = ga_screenshot`.
//! * `R_SetViewSize` is [`crate::r_main::Renderer::r_set_view_size`],
//!   applied at the start of the next `d_display`, like the original.
//! * The low-detail toggle only flips `detailLevel` and reports "low
//!   detail mode n.a." on stderr — as the original does.

use std::cell::RefCell;

use crate::d_englsh as msg;
use crate::d_event::{EvType, Event};
use crate::d_player::Player;
use crate::doomdef::{
    GameMode, GameState, Skill, KEY_BACKSPACE, KEY_DOWNARROW, KEY_ENTER, KEY_EQUALS, KEY_ESCAPE,
    KEY_F1, KEY_F10, KEY_F11, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8, KEY_F9,
    KEY_LEFTARROW, KEY_MINUS, KEY_RIGHTARROW, KEY_UPARROW, SCREENWIDTH,
};
use crate::doomstat;
use crate::dstrings::ENDMSG;
use crate::g_game::{g_defered_init_new, g_load_game, g_save_game};
use crate::hu_stuff::{hu_font, hu_set_message_dontfuckwithme, HU_FONTSIZE, HU_FONTSTART};
use crate::i_system::i_get_time;
use crate::m_misc::m_screenshot;
use crate::r_main::Renderer;
use crate::s_sound::s_start_sound;
use crate::sounds::Sfx;
use crate::v_video::{patch_height, patch_width, VVideo};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// (`SAVESTRINGSIZE`)
pub const SAVESTRINGSIZE: usize = 24;
/// (`SKULLXOFF`)
pub const SKULLXOFF: i32 = -32;
/// (`LINEHEIGHT`)
pub const LINEHEIGHT: i32 = 16;

/// The menus (`&MainDef`, `&EpiDef`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuId {
    Main,
    Episode,
    NewGame,
    Options,
    ReadThis1,
    ReadThis2,
    Sound,
    Load,
    Save,
}

const MENU_COUNT: usize = 9;

/// A menu item's routine (`void (*routine)(int choice)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewGame,
    Episode,
    ChooseSkill,
    LoadGame,
    SaveGame,
    Options,
    EndGame,
    ReadThis,
    ReadThis2,
    QuitDoom,
    ChangeMessages,
    ChangeSensitivity,
    SfxVol,
    MusicVol,
    ChangeDetail,
    SizeDisplay,
    Sound,
    FinishReadThis,
    LoadSelect,
    SaveSelect,
}

/// A menu's draw routine (`void (*routine)()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawFn {
    Main,
    ReadThis1,
    ReadThis2,
    NewGame,
    Episode,
    Options,
    Sound,
    Load,
    Save,
}

/// What answers a pending message (`messageRoutine`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgResponse {
    QuickSave,
    QuickLoad,
    VerifyNightmare,
    EndGame,
    Quit,
}

/// (`menuitem_t`) `status`: 0 = no cursor here, 1 = ok, 2 = arrows ok,
/// -1 = spacer.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub status: i16,
    pub name: &'static str,
    pub routine: Option<Action>,
    pub alpha_key: u8,
}

/// (`menu_t`)
#[derive(Debug, Clone)]
pub struct Menu {
    pub numitems: usize,
    pub prev: Option<MenuId>,
    pub items: Vec<MenuItem>,
    pub draw: DrawFn,
    pub x: i32,
    pub y: i32,
    /// Last item user was on in menu.
    pub last_on: usize,
}

fn item(status: i16, name: &'static str, routine: Option<Action>, key: u8) -> MenuItem {
    MenuItem {
        status,
        name,
        routine,
        alpha_key: key,
    }
}

/// The menu tables (`MainDef`, `EpiDef`, ... in `MenuId` order).
fn build_menus() -> Vec<Menu> {
    use Action::*;
    let slots = |routine| {
        (b'1'..=b'6')
            .map(|k| item(1, "", Some(routine), k))
            .collect::<Vec<_>>()
    };
    vec![
        // MainDef
        Menu {
            numitems: 6,
            prev: None,
            items: vec![
                item(1, "M_NGAME", Some(NewGame), b'n'),
                item(1, "M_OPTION", Some(Options), b'o'),
                item(1, "M_LOADG", Some(LoadGame), b'l'),
                item(1, "M_SAVEG", Some(SaveGame), b's'),
                item(1, "M_RDTHIS", Some(ReadThis), b'r'),
                item(1, "M_QUITG", Some(QuitDoom), b'q'),
            ],
            draw: DrawFn::Main,
            x: 97,
            y: 64,
            last_on: 0,
        },
        // EpiDef
        Menu {
            numitems: 4,
            prev: Some(MenuId::Main),
            items: vec![
                item(1, "M_EPI1", Some(Episode), b'k'),
                item(1, "M_EPI2", Some(Episode), b't'),
                item(1, "M_EPI3", Some(Episode), b'i'),
                item(1, "M_EPI4", Some(Episode), b't'),
            ],
            draw: DrawFn::Episode,
            x: 48,
            y: 63,
            last_on: 0,
        },
        // NewDef
        Menu {
            numitems: 5,
            prev: Some(MenuId::Episode),
            items: vec![
                item(1, "M_JKILL", Some(ChooseSkill), b'i'),
                item(1, "M_ROUGH", Some(ChooseSkill), b'h'),
                item(1, "M_HURT", Some(ChooseSkill), b'h'),
                item(1, "M_ULTRA", Some(ChooseSkill), b'u'),
                item(1, "M_NMARE", Some(ChooseSkill), b'n'),
            ],
            draw: DrawFn::NewGame,
            x: 48,
            y: 63,
            last_on: 2, // hurtme
        },
        // OptionsDef
        Menu {
            numitems: 8,
            prev: Some(MenuId::Main),
            items: vec![
                item(1, "M_ENDGAM", Some(EndGame), b'e'),
                item(1, "M_MESSG", Some(ChangeMessages), b'm'),
                item(1, "M_DETAIL", Some(ChangeDetail), b'g'),
                item(2, "M_SCRNSZ", Some(SizeDisplay), b's'),
                item(-1, "", None, 0),
                item(2, "M_MSENS", Some(ChangeSensitivity), b'm'),
                item(-1, "", None, 0),
                item(1, "M_SVOL", Some(Sound), b's'),
            ],
            draw: DrawFn::Options,
            x: 60,
            y: 37,
            last_on: 0,
        },
        // ReadDef1
        Menu {
            numitems: 1,
            prev: Some(MenuId::Main),
            items: vec![item(1, "", Some(ReadThis2), 0)],
            draw: DrawFn::ReadThis1,
            x: 280,
            y: 185,
            last_on: 0,
        },
        // ReadDef2
        Menu {
            numitems: 1,
            prev: Some(MenuId::ReadThis1),
            items: vec![item(1, "", Some(FinishReadThis), 0)],
            draw: DrawFn::ReadThis2,
            x: 330,
            y: 175,
            last_on: 0,
        },
        // SoundDef
        Menu {
            numitems: 4,
            prev: Some(MenuId::Options),
            items: vec![
                item(2, "M_SFXVOL", Some(SfxVol), b's'),
                item(-1, "", None, 0),
                item(2, "M_MUSVOL", Some(MusicVol), b'm'),
                item(-1, "", None, 0),
            ],
            draw: DrawFn::Sound,
            x: 80,
            y: 64,
            last_on: 0,
        },
        // LoadDef
        Menu {
            numitems: 6,
            prev: Some(MenuId::Main),
            items: slots(LoadSelect),
            draw: DrawFn::Load,
            x: 80,
            y: 54,
            last_on: 0,
        },
        // SaveDef
        Menu {
            numitems: 6,
            prev: Some(MenuId::Main),
            items: slots(SaveSelect),
            draw: DrawFn::Save,
            x: 80,
            y: 54,
            last_on: 0,
        },
    ]
}

/// What the menu needs from the game for one call.
pub struct MenuCtx<'a> {
    pub players: &'a mut [Player],
    pub wad: &'a mut WadFiles,
    pub video: &'a mut VVideo,
    pub renderer: &'a mut Renderer,
    /// Out: a palette the caller must `I_SetPalette` (the gamma key).
    pub palette: Option<Vec<u8>>,
}

/// The file-scope statics of `m_menu.c`.
struct MenuState {
    menuactive: bool,
    /// `gamemode`, as `M_Init` saw it (constant for a run).
    gamemode: GameMode,
    menus: Vec<Menu>,
    current: MenuId,
    /// Menu item skull is on.
    item_on: usize,
    /// Skull animation counter.
    skull_anim_counter: i32,
    /// Which skull to draw.
    which_skull: usize,

    /// `screenSize` (`screenblocks - 3`).
    screen_size: i32,
    quick_save_slot: i32,
    message_to_print: bool,
    message_string: String,
    message_routine: Option<MsgResponse>,
    message_last_menu_active: bool,
    message_needs_input: bool,

    save_string_enter: bool,
    /// Which slot to save in.
    save_slot: usize,
    /// Which char we're editing.
    save_char_index: usize,
    save_old_string: String,
    savegamestrings: Vec<String>,
    inhelpscreens: bool,
    /// Episode chosen in the episode menu (`epi`).
    epi: i32,

    // M_Responder's statics
    joywait: i32,
    mousewait: i32,
    mousey: i32,
    lasty: i32,
    mousex: i32,
    lastx: i32,
}

thread_local! {
    static MENU: RefCell<Option<MenuState>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut MenuState) -> R) -> R {
    MENU.with(|s| f(s.borrow_mut().as_mut().expect("M_Init has not run")))
}

/// Whether [`m_init`] has run on this thread.
pub fn m_is_initialized() -> bool {
    MENU.with(|s| s.borrow().is_some())
}

/// Drops the menu state (tests, shutdown).
pub fn m_shutdown() {
    MENU.with(|s| *s.borrow_mut() = None);
}

/// `inhelpscreens` (`d_main` redraws the status bar after a help
/// screen goes away).
pub fn m_inhelpscreens() -> bool {
    with(|s| s.inhelpscreens)
}

/// The menu currently showing and the item the skull is on.
pub fn m_current() -> (MenuId, usize) {
    with(|s| (s.current, s.item_on))
}

/// Port of `M_Init`: resets the menu and adapts it to the game mode.
pub fn m_init() {
    m_init_for(doomstat::state().gamemode);
}

/// [`m_init`] for an explicit game mode.
pub fn m_init_for(gamemode: GameMode) {
    let mut menus = build_menus();

    match gamemode {
        GameMode::Commercial => {
            // This is used because DOOM 2 had only one HELP page. I
            // use CREDIT as second page now, but kept this hack for
            // educational purposes (the original's own comment).
            let main = &mut menus[MenuId::Main as usize];
            main.items[4] = main.items[5].clone(); // MainMenu[readthis] = MainMenu[quitdoom]
            main.numitems -= 1;
            main.y += 8;
            menus[MenuId::NewGame as usize].prev = Some(MenuId::Main);
            let read1 = &mut menus[MenuId::ReadThis1 as usize];
            read1.draw = DrawFn::ReadThis1;
            read1.x = 330;
            read1.y = 165;
            read1.items[0].routine = Some(Action::FinishReadThis);
        }
        GameMode::Shareware | GameMode::Registered => {
            // We need to remove the fourth episode.
            menus[MenuId::Episode as usize].numitems -= 1;
        }
        _ => {}
    }

    doomstat::state_mut().menuactive = false;
    let screenblocks = doomstat::state().screenblocks;
    MENU.with(|s| {
        let item_on = menus[MenuId::Main as usize].last_on;
        *s.borrow_mut() = Some(MenuState {
            menuactive: false,
            gamemode,
            menus,
            current: MenuId::Main,
            item_on,
            skull_anim_counter: 10,
            which_skull: 0,
            screen_size: screenblocks - 3,
            quick_save_slot: -1,
            message_to_print: false,
            message_string: String::new(),
            message_routine: None,
            message_last_menu_active: false,
            message_needs_input: false,
            save_string_enter: false,
            save_slot: 0,
            save_char_index: 0,
            save_old_string: String::new(),
            savegamestrings: vec![String::new(); 10],
            inhelpscreens: false,
            epi: 0,
            joywait: 0,
            mousewait: 0,
            mousey: 0,
            lasty: 0,
            mousex: 0,
            lastx: 0,
        });
    });
}

fn menu_gamemode() -> GameMode {
    with(|s| s.gamemode)
}

/// `menuactive`. The menu's own copy is authoritative (it is
/// per-thread, which keeps parallel unit tests apart); every change is
/// published to `doomstat::menuactive`, which is what the rest of the
/// game (`G_Ticker`'s pause check, `D_Display`) reads.
fn menuactive() -> bool {
    with(|s| s.menuactive)
}

fn set_menuactive(on: bool) {
    with(|s| s.menuactive = on);
    doomstat::state_mut().menuactive = on;
}

fn sound(sfx: Sfx) {
    s_start_sound(None, sfx);
}

fn player_message(ctx: &mut MenuCtx, message: &'static str) {
    let console = doomstat::state().consoleplayer as usize;
    if let Some(p) = ctx.players.get_mut(console) {
        p.message = Some(message);
    }
}

// ---------------------------------------------------------------------
// Save/load slots
// ---------------------------------------------------------------------

fn savegame_path(slot: usize) -> String {
    format!("{}{}.dsg", msg::SAVEGAMENAME, slot)
}

/// Port of `M_ReadSaveStrings`: the descriptions stored in the save
/// files (the first `SAVESTRINGSIZE` bytes), or "empty slot".
pub fn m_read_save_strings() {
    with(|s| {
        for i in 0..6 {
            match std::fs::read(savegame_path(i)) {
                Ok(bytes) => {
                    let head = &bytes[..bytes.len().min(SAVESTRINGSIZE)];
                    let end = head.iter().position(|&b| b == 0).unwrap_or(head.len());
                    s.savegamestrings[i] = String::from_utf8_lossy(&head[..end]).into_owned();
                    s.menus[MenuId::Load as usize].items[i].status = 1;
                }
                Err(_) => {
                    s.savegamestrings[i] = msg::EMPTYSTRING.to_string();
                    s.menus[MenuId::Load as usize].items[i].status = 0;
                }
            }
        }
    });
}

/// Port of `M_LoadSelect`. User wants to load this game.
fn m_load_select(choice: usize) {
    g_load_game(&savegame_path(choice));
    m_clear_menus();
}

/// Port of `M_LoadGame`.
fn m_load_game(ctx: &mut MenuCtx) {
    if doomstat::state().netgame {
        m_start_message(msg::LOADNET, None, false);
        return;
    }
    m_setup_next_menu(MenuId::Load);
    m_read_save_strings();
    let _ = ctx;
}

/// Port of `M_DoSave`. Selected from DOOM menu.
fn m_do_save(slot: usize) {
    let desc = with(|s| s.savegamestrings[slot].clone());
    g_save_game(slot as i32, &desc);
    m_clear_menus();

    // PICK QUICKSAVE SLOT YET?
    with(|s| {
        if s.quick_save_slot == -2 {
            s.quick_save_slot = slot as i32;
        }
    });
}

/// Port of `M_SaveSelect`. User wants to save. Start string input for
/// `M_Responder`.
fn m_save_select(choice: usize) {
    with(|s| {
        s.save_string_enter = true;
        s.save_slot = choice;
        s.save_old_string = s.savegamestrings[choice].clone();
        if s.savegamestrings[choice] == msg::EMPTYSTRING {
            s.savegamestrings[choice].clear();
        }
        s.save_char_index = s.savegamestrings[choice].len();
    });
}

/// Port of `M_SaveGame`. Selected from DOOM menu.
fn m_save_game() {
    let st = doomstat::state();
    if !st.usergame {
        m_start_message(msg::SAVEDEAD, None, false);
        return;
    }
    if st.gamestate != GameState::Level {
        return;
    }
    m_setup_next_menu(MenuId::Save);
    m_read_save_strings();
}

/// Port of `M_QuickSave`. M_QuickSave; M_QuickSaveResponse.
pub fn m_quick_save() {
    let st = doomstat::state();
    if !st.usergame {
        sound(Sfx::SfxOof);
        return;
    }
    if st.gamestate != GameState::Level {
        return;
    }
    if with(|s| s.quick_save_slot) < 0 {
        m_start_control_panel();
        m_read_save_strings();
        m_setup_next_menu(MenuId::Save);
        with(|s| s.quick_save_slot = -2); // means to pick a slot now
        return;
    }
    let text =
        with(|s| msg::QSPROMPT.replace("%s", &s.savegamestrings[s.quick_save_slot as usize]));
    m_start_message(&text, Some(MsgResponse::QuickSave), true);
}

/// Port of `M_QuickLoad`.
pub fn m_quick_load() {
    if doomstat::state().netgame {
        m_start_message(msg::QLOADNET, None, false);
        return;
    }
    if with(|s| s.quick_save_slot) < 0 {
        m_start_message(msg::QSAVESPOT, None, false);
        return;
    }
    let text =
        with(|s| msg::QLPROMPT.replace("%s", &s.savegamestrings[s.quick_save_slot as usize]));
    m_start_message(&text, Some(MsgResponse::QuickLoad), true);
}

// ---------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------

fn m_new_game() {
    if doomstat::state().netgame && !doomstat::state().demoplayback {
        m_start_message(msg::NEWGAME, None, false);
        return;
    }

    if menu_gamemode() == GameMode::Commercial {
        m_setup_next_menu(MenuId::NewGame);
    } else {
        m_setup_next_menu(MenuId::Episode);
    }
}

fn skill_from(choice: usize) -> Skill {
    match choice {
        0 => Skill::Baby,
        1 => Skill::Easy,
        2 => Skill::Medium,
        3 => Skill::Hard,
        _ => Skill::Nightmare,
    }
}

fn m_choose_skill(choice: usize) {
    if choice == 4 {
        // nightmare
        m_start_message(msg::NIGHTMARE, Some(MsgResponse::VerifyNightmare), true);
        return;
    }
    let epi = with(|s| s.epi);
    g_defered_init_new(skill_from(choice), epi + 1, 1);
    m_clear_menus();
}

fn m_episode(choice: usize) {
    let gamemode = menu_gamemode();
    if gamemode == GameMode::Shareware && choice != 0 {
        m_start_message(msg::SWSTRING, None, false);
        m_setup_next_menu(MenuId::ReadThis1);
        return;
    }

    // Yet another hack...
    let mut choice = choice;
    if gamemode == GameMode::Registered && choice > 2 {
        eprintln!("M_Episode: 4th episode requires UltimateDOOM");
        choice = 0;
    }

    with(|s| s.epi = choice as i32);
    m_setup_next_menu(MenuId::NewGame);
}

fn m_change_messages(ctx: &mut MenuCtx) {
    let st = doomstat::state_mut();
    st.show_messages = 1 - st.show_messages;
    let on = st.show_messages != 0;
    player_message(ctx, if on { msg::MSGON } else { msg::MSGOFF });
    if crate::hu_stuff::hu_is_initialized() {
        hu_set_message_dontfuckwithme(true);
    }
}

fn m_end_game() {
    let st = doomstat::state();
    if !st.usergame {
        sound(Sfx::SfxOof);
        return;
    }
    if st.netgame {
        m_start_message(msg::NETEND, None, false);
        return;
    }
    m_start_message(msg::ENDGAME, Some(MsgResponse::EndGame), true);
}

/// The sounds `M_QuitResponse` picks from (`quitsounds[]`).
pub const QUITSOUNDS: [Sfx; 8] = [
    Sfx::SfxPldeth,
    Sfx::SfxDmpain,
    Sfx::SfxPopain,
    Sfx::SfxSlop,
    Sfx::SfxTelept,
    Sfx::SfxPosit1,
    Sfx::SfxPosit3,
    Sfx::SfxSgtatk,
];
/// DOOM II's (`quitsounds2[]`).
pub const QUITSOUNDS2: [Sfx; 8] = [
    Sfx::SfxVilact,
    Sfx::SfxGetpow,
    Sfx::SfxBoscub,
    Sfx::SfxSlop,
    Sfx::SfxSkeswg,
    Sfx::SfxKntdth,
    Sfx::SfxBspact,
    Sfx::SfxSgtatk,
];

fn m_quit_doom() {
    let st = doomstat::state();
    let quit_msg = if st.language != crate::doomdef::Language::English {
        ENDMSG[0].unwrap_or("")
    } else {
        // (gametic % (NUM_QUITMESSAGES-2)) + 1; the original array's two
        // missing commas can leave a NULL here — see `dstrings`.
        let i = (st.gametic.rem_euclid(msg::NUM_QUITMESSAGES as i32 - 2)) as usize + 1;
        ENDMSG[i].unwrap_or("")
    };
    let text = format!("{}\n\n{}", quit_msg, msg::DOSY);
    m_start_message(&text, Some(MsgResponse::Quit), true);
}

fn m_change_sensitivity(choice: usize) {
    let st = doomstat::state_mut();
    match choice {
        0 => {
            if st.mouse_sensitivity != 0 {
                st.mouse_sensitivity -= 1;
            }
        }
        1 => {
            if st.mouse_sensitivity < 9 {
                st.mouse_sensitivity += 1;
            }
        }
        _ => {}
    }
}

fn m_change_detail() {
    let st = doomstat::state_mut();
    st.detail_level = 1 - st.detail_level;

    // FIXME - does not work. Remove anyway?
    eprintln!("M_ChangeDetail: low detail mode n.a.");
}

fn m_size_display(choice: usize, ctx: &mut MenuCtx) {
    let ss = with(|s| s.screen_size);
    let st = doomstat::state_mut();
    match choice {
        0 => {
            if ss > 0 {
                st.screenblocks -= 1;
                with(|s| s.screen_size -= 1);
            }
        }
        1 => {
            if ss < 8 {
                st.screenblocks += 1;
                with(|s| s.screen_size += 1);
            }
        }
        _ => {}
    }
    let (blocks, detail) = (st.screenblocks, st.detail_level);
    ctx.renderer.r_set_view_size(blocks, detail);
}

fn m_sfx_vol(choice: usize) {
    let mut v = doomstat::state().snd_sfx_volume;
    match choice {
        0 => {
            if v != 0 {
                v -= 1;
            }
        }
        1 => {
            if v < 15 {
                v += 1;
            }
        }
        _ => {}
    }
    crate::s_sound::s_set_sfx_volume(v);
}

fn m_music_vol(choice: usize) {
    let mut v = doomstat::state().snd_music_volume;
    match choice {
        0 => {
            if v != 0 {
                v -= 1;
            }
        }
        1 => {
            if v < 15 {
                v += 1;
            }
        }
        _ => {}
    }
    crate::s_sound::s_set_music_volume(v);
}

fn call_action(action: Action, choice: usize, ctx: &mut MenuCtx) {
    match action {
        Action::NewGame => m_new_game(),
        Action::Episode => m_episode(choice),
        Action::ChooseSkill => m_choose_skill(choice),
        Action::LoadGame => m_load_game(ctx),
        Action::SaveGame => m_save_game(),
        Action::Options => m_setup_next_menu(MenuId::Options),
        Action::EndGame => m_end_game(),
        Action::ReadThis => m_setup_next_menu(MenuId::ReadThis1),
        Action::ReadThis2 => m_setup_next_menu(MenuId::ReadThis2),
        Action::QuitDoom => m_quit_doom(),
        Action::ChangeMessages => m_change_messages(ctx),
        Action::ChangeSensitivity => m_change_sensitivity(choice),
        Action::SfxVol => m_sfx_vol(choice),
        Action::MusicVol => m_music_vol(choice),
        Action::ChangeDetail => m_change_detail(),
        Action::SizeDisplay => m_size_display(choice, ctx),
        Action::Sound => m_setup_next_menu(MenuId::Sound),
        Action::FinishReadThis => m_setup_next_menu(MenuId::Main),
        Action::LoadSelect => m_load_select(choice),
        Action::SaveSelect => m_save_select(choice),
    }
}

/// The answer to a message (`messageRoutine(ch)`). `ch` is the key.
fn call_response(response: MsgResponse, ch: i32) {
    let yes = ch == b'y' as i32;
    match response {
        MsgResponse::QuickSave => {
            if yes {
                let slot = with(|s| s.quick_save_slot) as usize;
                m_do_save(slot);
                sound(Sfx::SfxSwtchx);
            }
        }
        MsgResponse::QuickLoad => {
            if yes {
                let slot = with(|s| s.quick_save_slot) as usize;
                m_load_select(slot);
                sound(Sfx::SfxSwtchx);
            }
        }
        MsgResponse::VerifyNightmare => {
            if !yes {
                return;
            }
            let epi = with(|s| s.epi);
            g_defered_init_new(Skill::Nightmare, epi + 1, 1);
            m_clear_menus();
        }
        MsgResponse::EndGame => {
            if !yes {
                return;
            }
            with(|s| {
                let cur = s.current as usize;
                s.menus[cur].last_on = s.item_on;
            });
            m_clear_menus();
            crate::d_main::d_start_title();
        }
        MsgResponse::Quit => {
            if !yes {
                return;
            }
            let st = doomstat::state();
            if !st.netgame {
                let i = ((st.gametic >> 2) & 7) as usize;
                if menu_gamemode() == GameMode::Commercial {
                    sound(QUITSOUNDS2[i]);
                } else {
                    sound(QUITSOUNDS[i]);
                }
            }
            // I_Quit: the main loop sees this, lets the quit sound
            // start, and exits.
            doomstat::state_mut().quit_requested = true;
        }
    }
}

// ---------------------------------------------------------------------
// Messages and menu bookkeeping
// ---------------------------------------------------------------------

/// Port of `M_StartMessage`. Display a message box (the original's
/// own comment: "Draw a message and wait for a keypress"); the
/// response is asked `y`/`n`/space/escape if `input`.
pub fn m_start_message(string: &str, routine: Option<MsgResponse>, input: bool) {
    with(|s| {
        s.message_last_menu_active = s.menuactive;
        s.message_to_print = true;
        s.message_string = string.to_string();
        s.message_routine = routine;
        s.message_needs_input = input;
    });
    set_menuactive(true);
}

/// Port of `M_StopMessage`.
pub fn m_stop_message() {
    let last = with(|s| {
        s.message_to_print = false;
        s.message_last_menu_active
    });
    set_menuactive(last);
}

/// Port of `M_SetupNextMenu`.
fn m_setup_next_menu(menu: MenuId) {
    with(|s| {
        s.current = menu;
        s.item_on = s.menus[menu as usize].last_on;
    });
}

/// Port of `M_StartControlPanel`. Called by the main loop and the
/// responder to open the menu.
pub fn m_start_control_panel() {
    // intro might call this repeatedly (the original's own comment)
    if menuactive() {
        return;
    }
    set_menuactive(true);
    with(|s| {
        s.current = MenuId::Main; // JDC
        s.item_on = s.menus[MenuId::Main as usize].last_on; // JDC
    });
}

/// Port of `M_ClearMenus`.
pub fn m_clear_menus() {
    set_menuactive(false);
}

/// Port of `M_Ticker`: animates the skull.
pub fn m_ticker() {
    with(|s| {
        s.skull_anim_counter -= 1;
        if s.skull_anim_counter <= 0 {
            s.which_skull ^= 1;
            s.skull_anim_counter = 8;
        }
    });
}

// ---------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------

/// Port of `M_StringWidth`. Find string width from hu_font chars.
pub fn m_string_width(string: &str) -> i32 {
    let font = hu_font();
    string
        .bytes()
        .map(|b| {
            let c = b.to_ascii_uppercase() as i32 - HU_FONTSTART as i32;
            if c < 0 || c >= HU_FONTSIZE as i32 {
                4
            } else {
                patch_width(&font[c as usize])
            }
        })
        .sum()
}

/// Port of `M_StringHeight`. Find string height from hu_font chars.
pub fn m_string_height(string: &str) -> i32 {
    let height = patch_height(&hu_font()[0]);
    height + string.bytes().filter(|&b| b == b'\n').count() as i32 * height
}

/// Port of `M_WriteText`. Write a string using the hu_font.
pub fn m_write_text(v: &mut VVideo, x: i32, y: i32, string: &str) {
    let font = hu_font();
    let (mut cx, mut cy) = (x, y);

    for b in string.bytes() {
        if b == b'\n' {
            cx = x;
            cy += 12;
            continue;
        }

        let c = b.to_ascii_uppercase() as i32 - HU_FONTSTART as i32;
        if c < 0 || c >= HU_FONTSIZE as i32 {
            cx += 4;
            continue;
        }

        let w = patch_width(&font[c as usize]);
        if cx + w > SCREENWIDTH {
            break;
        }
        v.v_draw_patch_direct(cx, cy, 0, &font[c as usize]);
        cx += w;
    }
}

// ---------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------

fn draw_named(v: &mut VVideo, wad: &mut WadFiles, x: i32, y: i32, name: &str) {
    let patch = wad.cache_lump_name(name, PurgeTag::Cache).to_vec();
    v.v_draw_patch_direct(x, y, 0, &patch);
}

/// Port of `M_DrawThermo`. Draw a `thermometer` bar.
fn m_draw_thermo(
    v: &mut VVideo,
    wad: &mut WadFiles,
    x: i32,
    y: i32,
    therm_width: i32,
    therm_dot: i32,
) {
    let mut xx = x;
    draw_named(v, wad, xx, y, "M_THERML");
    xx += 8;
    for _ in 0..therm_width {
        draw_named(v, wad, xx, y, "M_THERMM");
        xx += 8;
    }
    draw_named(v, wad, xx, y, "M_THERMR");
    draw_named(v, wad, (x + 8) + therm_dot * 8, y, "M_THERMO");
}

/// Port of `M_DrawSaveLoadBorder`.
fn m_draw_save_load_border(v: &mut VVideo, wad: &mut WadFiles, x: i32, y: i32) {
    draw_named(v, wad, x - 8, y + 7, "M_LSLEFT");
    let mut x = x;
    for _ in 0..24 {
        draw_named(v, wad, x, y + 7, "M_LSCNTR");
        x += 8;
    }
    draw_named(v, wad, x, y + 7, "M_LSRGHT");
}

fn run_draw(f: DrawFn, v: &mut VVideo, wad: &mut WadFiles) {
    let (menu_x, menu_y) = with(|s| {
        let m = &s.menus[s.current as usize];
        (m.x, m.y)
    });
    let gamemode = menu_gamemode();
    match f {
        DrawFn::Main => draw_named(v, wad, 94, 2, "M_DOOM"),
        DrawFn::ReadThis1 => {
            with(|s| s.inhelpscreens = true);
            match gamemode {
                GameMode::Commercial => draw_named(v, wad, 0, 0, "HELP"),
                GameMode::Shareware | GameMode::Registered | GameMode::Retail => {
                    draw_named(v, wad, 0, 0, "HELP1")
                }
                _ => {}
            }
        }
        DrawFn::ReadThis2 => {
            with(|s| s.inhelpscreens = true);
            match gamemode {
                GameMode::Retail | GameMode::Commercial => draw_named(v, wad, 0, 0, "CREDIT"),
                GameMode::Shareware | GameMode::Registered => draw_named(v, wad, 0, 0, "HELP2"),
                _ => {}
            }
        }
        DrawFn::NewGame => {
            draw_named(v, wad, 96, 14, "M_NEWG");
            draw_named(v, wad, 54, 38, "M_SKILL");
        }
        DrawFn::Episode => draw_named(v, wad, 54, 38, "M_EPISOD"),
        DrawFn::Options => {
            let st = doomstat::state();
            let (detail, msgs, mouse) = (st.detail_level, st.show_messages, st.mouse_sensitivity);
            let screen_size = with(|s| s.screen_size);
            draw_named(v, wad, 108, 15, "M_OPTTTL");
            draw_named(
                v,
                wad,
                menu_x + 175,
                menu_y + LINEHEIGHT * 2,
                ["M_GDHIGH", "M_GDLOW"][detail as usize],
            );
            draw_named(
                v,
                wad,
                menu_x + 120,
                menu_y + LINEHEIGHT,
                ["M_MSGOFF", "M_MSGON"][msgs as usize],
            );
            m_draw_thermo(v, wad, menu_x, menu_y + LINEHEIGHT * (5 + 1), 10, mouse);
            m_draw_thermo(
                v,
                wad,
                menu_x,
                menu_y + LINEHEIGHT * (3 + 1),
                9,
                screen_size,
            );
        }
        DrawFn::Sound => {
            let st = doomstat::state();
            let (sfx, music) = (st.snd_sfx_volume, st.snd_music_volume);
            draw_named(v, wad, 60, 38, "M_SVOL");
            m_draw_thermo(v, wad, menu_x, menu_y + LINEHEIGHT, 16, sfx);
            m_draw_thermo(v, wad, menu_x, menu_y + LINEHEIGHT * 3, 16, music);
        }
        DrawFn::Load | DrawFn::Save => {
            let saving = f == DrawFn::Save;
            draw_named(v, wad, 72, 28, if saving { "M_SAVEG" } else { "M_LOADG" });
            let strings = with(|s| s.savegamestrings.clone());
            for (i, text) in strings.iter().take(6).enumerate() {
                m_draw_save_load_border(v, wad, menu_x, menu_y + LINEHEIGHT * i as i32);
                m_write_text(v, menu_x, menu_y + LINEHEIGHT * i as i32, text);
            }
            if saving && with(|s| s.save_string_enter) {
                let (slot, w) =
                    with(|s| (s.save_slot, m_string_width(&s.savegamestrings[s.save_slot])));
                m_write_text(v, menu_x + w, menu_y + LINEHEIGHT * slot as i32, "_");
            }
        }
    }
}

/// Port of `M_Drawer`. Called after the view has been rendered, but
/// before it has been blitted (the original's own comment).
pub fn m_drawer(v: &mut VVideo, wad: &mut WadFiles) {
    with(|s| s.inhelpscreens = false);

    // Horiz. & Vertically center string and print it.
    if with(|s| s.message_to_print) {
        let text = with(|s| s.message_string.clone());
        let lh = patch_height(&hu_font()[0]);
        let mut y = 100 - m_string_height(&text) / 2;
        for line in text.split('\n') {
            let x = 160 - m_string_width(line) / 2;
            m_write_text(v, x, y, line);
            y += lh;
        }
        return;
    }

    if !menuactive() {
        return;
    }

    let (draw, x, y0, max, item_on, which_skull, names) = with(|s| {
        let m = &s.menus[s.current as usize];
        (
            m.draw,
            m.x,
            m.y,
            m.numitems,
            s.item_on,
            s.which_skull,
            m.items.iter().map(|i| i.name).collect::<Vec<_>>(),
        )
    });

    // call Draw routine
    run_draw(draw, v, wad);

    // DRAW MENU
    let mut y = y0;
    for name in names.iter().take(max) {
        if !name.is_empty() {
            draw_named(v, wad, x, y, name);
        }
        y += LINEHEIGHT;
    }

    // DRAW SKULL
    draw_named(
        v,
        wad,
        x + SKULLXOFF,
        y0 - 5 + item_on as i32 * LINEHEIGHT,
        ["M_SKULL1", "M_SKULL2"][which_skull],
    );
}

// ---------------------------------------------------------------------
// Responder
// ---------------------------------------------------------------------

/// Port of `M_Responder`. Handles menu input; returns true if the
/// event was eaten.
pub fn m_responder(ev: &Event, ctx: &mut MenuCtx) -> bool {
    let mut ch: i32 = -1;
    let now = i_get_time();

    if ev.event_type == EvType::Joystick && with(|s| s.joywait) < now {
        if ev.data3 == -1 {
            ch = KEY_UPARROW;
            with(|s| s.joywait = now + 5);
        } else if ev.data3 == 1 {
            ch = KEY_DOWNARROW;
            with(|s| s.joywait = now + 5);
        }

        if ev.data2 == -1 {
            ch = KEY_LEFTARROW;
            with(|s| s.joywait = now + 2);
        } else if ev.data2 == 1 {
            ch = KEY_RIGHTARROW;
            with(|s| s.joywait = now + 2);
        }

        if ev.data1 & 1 != 0 {
            ch = KEY_ENTER;
            with(|s| s.joywait = now + 5);
        }
        if ev.data1 & 2 != 0 {
            ch = KEY_BACKSPACE;
            with(|s| s.joywait = now + 5);
        }
    } else if ev.event_type == EvType::Mouse && with(|s| s.mousewait) < now {
        with(|s| {
            s.mousey += ev.data3;
            if s.mousey < s.lasty - 30 {
                ch = KEY_DOWNARROW;
                s.mousewait = now + 5;
                s.lasty -= 30;
                s.mousey = s.lasty;
            } else if s.mousey > s.lasty + 30 {
                ch = KEY_UPARROW;
                s.mousewait = now + 5;
                s.lasty += 30;
                s.mousey = s.lasty;
            }

            s.mousex += ev.data2;
            if s.mousex < s.lastx - 30 {
                ch = KEY_LEFTARROW;
                s.mousewait = now + 5;
                s.lastx -= 30;
                s.mousex = s.lastx;
            } else if s.mousex > s.lastx + 30 {
                ch = KEY_RIGHTARROW;
                s.mousewait = now + 5;
                s.lastx += 30;
                s.mousex = s.lastx;
            }

            if ev.data1 & 1 != 0 {
                ch = KEY_ENTER;
                s.mousewait = now + 15;
            }
            if ev.data1 & 2 != 0 {
                ch = KEY_BACKSPACE;
                s.mousewait = now + 15;
            }
        });
    } else if ev.event_type == EvType::KeyDown {
        ch = ev.data1;
    }

    if ch == -1 {
        return false;
    }

    // Save Game string input
    if with(|s| s.save_string_enter) {
        with(|s| match ch {
            KEY_BACKSPACE => {
                if s.save_char_index > 0 {
                    s.save_char_index -= 1;
                    s.savegamestrings[s.save_slot].truncate(s.save_char_index);
                }
            }
            KEY_ESCAPE => {
                s.save_string_enter = false;
                s.savegamestrings[s.save_slot] = s.save_old_string.clone();
            }
            KEY_ENTER => {
                s.save_string_enter = false;
                if !s.savegamestrings[s.save_slot].is_empty() {
                    // (M_DoSave re-enters the menu state: run it after
                    // this borrow ends.)
                    PENDING_SAVE.with(|p| p.set(Some(s.save_slot)));
                }
            }
            _ => {
                let ch = (ch as u8 as char).to_ascii_uppercase() as i32;
                if ch != 32
                    && (ch - (HU_FONTSTART as i32) < 0
                        || ch - HU_FONTSTART as i32 >= HU_FONTSIZE as i32)
                {
                    return;
                }
                let width = m_string_width(&s.savegamestrings[s.save_slot]);
                if (32..=127).contains(&ch)
                    && s.save_char_index < SAVESTRINGSIZE - 1
                    && width < (SAVESTRINGSIZE as i32 - 2) * 8
                {
                    s.savegamestrings[s.save_slot].push(ch as u8 as char);
                    s.save_char_index += 1;
                }
            }
        });
        if let Some(slot) = PENDING_SAVE.with(|p| p.take()) {
            m_do_save(slot);
        }
        return true;
    }

    // Take care of any messages that need input
    if with(|s| s.message_to_print) {
        if with(|s| s.message_needs_input)
            && !(ch == b' ' as i32 || ch == b'n' as i32 || ch == b'y' as i32 || ch == KEY_ESCAPE)
        {
            return false;
        }

        let (last, routine) = with(|s| {
            s.message_to_print = false;
            (s.message_last_menu_active, s.message_routine)
        });
        set_menuactive(last);
        if let Some(r) = routine {
            call_response(r, ch);
        }

        set_menuactive(false);
        sound(Sfx::SfxSwtchx);
        return true;
    }

    if doomstat::state().devparm && ch == KEY_F1 {
        // G_ScreenShot (immediately, see the module docs)
        let palette = ctx.wad.cache_lump_name("PLAYPAL", PurgeTag::Cache).to_vec();
        let console = doomstat::state().consoleplayer as usize;
        if let Some(p) = ctx.players.get_mut(console) {
            m_screenshot(ctx.video, &palette, p);
        }
        return true;
    }

    // F-Keys
    if !menuactive() {
        match ch {
            // Screen size down
            KEY_MINUS => {
                if doomstat::state().automapactive
                    || crate::hu_stuff::hu_is_initialized() && crate::hu_stuff::hu_chat_on()
                {
                    return false;
                }
                m_size_display(0, ctx);
                sound(Sfx::SfxStnmov);
                return true;
            }
            // Screen size up
            KEY_EQUALS => {
                if doomstat::state().automapactive
                    || crate::hu_stuff::hu_is_initialized() && crate::hu_stuff::hu_chat_on()
                {
                    return false;
                }
                m_size_display(1, ctx);
                sound(Sfx::SfxStnmov);
                return true;
            }
            // Help key
            KEY_F1 => {
                m_start_control_panel();
                with(|s| {
                    s.current = if s.gamemode == GameMode::Retail {
                        MenuId::ReadThis2
                    } else {
                        MenuId::ReadThis1
                    };
                    s.item_on = 0;
                });
                sound(Sfx::SfxSwtchn);
                return true;
            }
            // Save
            KEY_F2 => {
                m_start_control_panel();
                sound(Sfx::SfxSwtchn);
                m_save_game();
                return true;
            }
            // Load
            KEY_F3 => {
                m_start_control_panel();
                sound(Sfx::SfxSwtchn);
                m_load_game(ctx);
                return true;
            }
            // Sound Volume
            KEY_F4 => {
                m_start_control_panel();
                with(|s| {
                    s.current = MenuId::Sound;
                    s.item_on = 0;
                });
                sound(Sfx::SfxSwtchn);
                return true;
            }
            // Detail toggle
            KEY_F5 => {
                m_change_detail();
                sound(Sfx::SfxSwtchn);
                return true;
            }
            // Quicksave
            KEY_F6 => {
                sound(Sfx::SfxSwtchn);
                m_quick_save();
                return true;
            }
            // End game
            KEY_F7 => {
                sound(Sfx::SfxSwtchn);
                m_end_game();
                return true;
            }
            // Toggle messages
            KEY_F8 => {
                m_change_messages(ctx);
                sound(Sfx::SfxSwtchn);
                return true;
            }
            // Quickload
            KEY_F9 => {
                sound(Sfx::SfxSwtchn);
                m_quick_load();
                return true;
            }
            // Quit DOOM
            KEY_F10 => {
                sound(Sfx::SfxSwtchn);
                m_quit_doom();
                return true;
            }
            // gamma toggle
            KEY_F11 => {
                let g = &mut ctx.video.usegamma;
                *g += 1;
                if *g > 4 {
                    *g = 0;
                }
                let gamma = *g as usize;
                player_message(
                    ctx,
                    [
                        msg::GAMMALVL0,
                        msg::GAMMALVL1,
                        msg::GAMMALVL2,
                        msg::GAMMALVL3,
                        msg::GAMMALVL4,
                    ][gamma],
                );
                ctx.palette =
                    Some(ctx.wad.cache_lump_name("PLAYPAL", PurgeTag::Cache)[..768].to_vec());
                return true;
            }
            _ => {}
        }
    }

    // Pop-up menu?
    if !menuactive() {
        if ch == KEY_ESCAPE {
            m_start_control_panel();
            sound(Sfx::SfxSwtchn);
            return true;
        }
        return false;
    }

    // Keys usable within menu
    match ch {
        KEY_DOWNARROW => {
            loop {
                with(|s| {
                    let n = s.menus[s.current as usize].numitems;
                    if s.item_on + 1 > n - 1 {
                        s.item_on = 0;
                    } else {
                        s.item_on += 1;
                    }
                });
                sound(Sfx::SfxPstop);
                if with(|s| s.menus[s.current as usize].items[s.item_on].status) != -1 {
                    break;
                }
            }
            true
        }
        KEY_UPARROW => {
            loop {
                with(|s| {
                    if s.item_on == 0 {
                        s.item_on = s.menus[s.current as usize].numitems - 1;
                    } else {
                        s.item_on -= 1;
                    }
                });
                sound(Sfx::SfxPstop);
                if with(|s| s.menus[s.current as usize].items[s.item_on].status) != -1 {
                    break;
                }
            }
            true
        }
        KEY_LEFTARROW | KEY_RIGHTARROW => {
            let it = with(|s| s.menus[s.current as usize].items[s.item_on].clone());
            if it.routine.is_some() && it.status == 2 {
                sound(Sfx::SfxStnmov);
                call_action(it.routine.unwrap(), (ch == KEY_RIGHTARROW) as usize, ctx);
            }
            true
        }
        KEY_ENTER => {
            let (it, item_on) = with(|s| {
                let cur = s.current as usize;
                (s.menus[cur].items[s.item_on].clone(), s.item_on)
            });
            if it.routine.is_some() && it.status != 0 {
                with(|s| {
                    let cur = s.current as usize;
                    s.menus[cur].last_on = item_on;
                });
                if it.status == 2 {
                    call_action(it.routine.unwrap(), 1, ctx); // right arrow
                    sound(Sfx::SfxStnmov);
                } else {
                    call_action(it.routine.unwrap(), item_on, ctx);
                    sound(Sfx::SfxPistol);
                }
            }
            true
        }
        KEY_ESCAPE => {
            with(|s| {
                let cur = s.current as usize;
                s.menus[cur].last_on = s.item_on;
            });
            m_clear_menus();
            sound(Sfx::SfxSwtchx);
            true
        }
        KEY_BACKSPACE => {
            with(|s| {
                let cur = s.current as usize;
                s.menus[cur].last_on = s.item_on;
                if let Some(prev) = s.menus[cur].prev {
                    s.current = prev;
                    s.item_on = s.menus[prev as usize].last_on;
                    sound(Sfx::SfxSwtchn);
                }
            });
            true
        }
        _ => {
            // (alpha keys) look for the next item with that key
            let found = with(|s| {
                let m = &s.menus[s.current as usize];
                let after =
                    (s.item_on + 1..m.numitems).find(|&i| m.items[i].alpha_key as i32 == ch);
                let upto = (0..=s.item_on).find(|&i| m.items[i].alpha_key as i32 == ch);
                after.or(upto)
            });
            if let Some(i) = found {
                with(|s| s.item_on = i);
                sound(Sfx::SfxPstop);
                return true;
            }
            false
        }
    }
}

thread_local! {
    /// `M_DoSave`'s slot, deferred out of the borrow `m_responder`'s
    /// save-name editor holds.
    static PENDING_SAVE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        wad: WadFiles,
        video: VVideo,
        renderer: Renderer,
        players: Vec<Player>,
    }

    fn fixture(mode: GameMode) -> Option<Fixture> {
        let path = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut wad = WadFiles::new();
        wad.init_file(path);
        let renderer = Renderer::r_init(&mut wad, 10, 0);
        crate::hu_stuff::hu_shutdown();
        crate::hu_stuff::hu_init(&mut wad);
        m_init_for(mode);
        let mut thinkers = crate::p_tick::Thinkers::new();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj(crate::r_defs::Mobj::blank(
                crate::info::MobjType::MtPlayer,
            )),
        );
        Some(Fixture {
            wad,
            video: VVideo::new(),
            renderer,
            players: vec![Player::for_test(mo)],
        })
    }

    impl Fixture {
        fn press(&mut self, key: i32) -> bool {
            let ev = Event {
                event_type: EvType::KeyDown,
                data1: key,
                data2: 0,
                data3: 0,
            };
            let mut ctx = MenuCtx {
                players: &mut self.players,
                wad: &mut self.wad,
                video: &mut self.video,
                renderer: &mut self.renderer,
                palette: None,
            };
            m_responder(&ev, &mut ctx)
        }

        fn type_str(&mut self, s: &str) {
            for c in s.bytes() {
                self.press(c as i32);
            }
        }
    }

    #[test]
    fn menu_layout_follows_the_game_mode() {
        let Some(_f) = fixture(GameMode::Commercial) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        with(|s| {
            let main = &s.menus[MenuId::Main as usize];
            assert_eq!(main.numitems, 5, "DOOM II has no read-this entry");
            assert_eq!(main.items[4].name, "M_QUITG");
            assert_eq!(main.y, 72);
            assert_eq!(s.menus[MenuId::NewGame as usize].prev, Some(MenuId::Main));
            assert_eq!(
                s.menus[MenuId::ReadThis1 as usize].items[0].routine,
                Some(Action::FinishReadThis)
            );
        });
        m_init_for(GameMode::Shareware);
        with(|s| {
            assert_eq!(s.menus[MenuId::Main as usize].numitems, 6);
            assert_eq!(s.menus[MenuId::Episode as usize].numitems, 3);
        });
        m_init_for(GameMode::Retail);
        with(|s| assert_eq!(s.menus[MenuId::Episode as usize].numitems, 4));
        assert!(!menuactive());
    }

    #[test]
    fn escape_opens_and_closes_and_other_keys_pass_through_when_closed() {
        let Some(mut f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        assert!(!f.press(b'x' as i32), "closed menu ignores plain keys");
        assert!(!f.press(KEY_DOWNARROW));
        assert!(f.press(KEY_ESCAPE));
        assert!(menuactive());
        assert_eq!(m_current(), (MenuId::Main, 0));
        assert!(f.press(KEY_ESCAPE));
        assert!(!menuactive());
    }

    #[test]
    fn arrows_wrap_and_skip_spacers() {
        let Some(mut f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.press(KEY_ESCAPE);
        f.press(KEY_UPARROW); // wraps to the last item (Quit)
        assert_eq!(m_current(), (MenuId::Main, 5));
        f.press(KEY_DOWNARROW);
        assert_eq!(m_current(), (MenuId::Main, 0));

        // Options: End Game, Messages, Detail, Screen Size, [spacer],
        // Mouse Sens, [spacer], Sound Volume.
        with(|s| {
            s.current = MenuId::Options;
            s.item_on = 3;
        });
        f.press(KEY_DOWNARROW);
        assert_eq!(m_current().1, 5, "the spacer at 4 is skipped");
        f.press(KEY_DOWNARROW);
        assert_eq!(m_current().1, 7, "and the one at 6");
        f.press(KEY_DOWNARROW);
        assert_eq!(m_current().1, 0, "wraps");
        f.press(KEY_UPARROW);
        assert_eq!(m_current().1, 7);
    }

    #[test]
    fn alpha_keys_jump_to_the_next_matching_item() {
        let Some(mut f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.press(KEY_ESCAPE);
        assert!(f.press(b'q' as i32));
        assert_eq!(m_current().1, 5);
        assert!(f.press(b'n' as i32));
        assert_eq!(m_current().1, 0, "searches after, then from the top");
        assert!(!f.press(b'z' as i32), "no such key: not eaten");

        // Episode menu (Ultimate DOOM has all four): 't' is on items 1
        // and 3; pressing it cycles.
        m_init_for(GameMode::Retail);
        f.press(KEY_ESCAPE);
        with(|s| {
            s.current = MenuId::Episode;
            s.item_on = 0;
        });
        f.press(b't' as i32);
        assert_eq!(m_current().1, 1);
        f.press(b't' as i32);
        assert_eq!(m_current().1, 3);
        f.press(b't' as i32);
        assert_eq!(m_current().1, 1);
    }

    #[test]
    fn enter_walks_into_submenus_and_backspace_returns() {
        let Some(mut f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.press(KEY_ESCAPE);
        f.press(KEY_ENTER); // New Game -> episodes
        assert_eq!(m_current(), (MenuId::Episode, 0));
        f.press(KEY_ENTER); // Knee-deep -> skill (remembers "hurt me plenty")
        assert_eq!(m_current(), (MenuId::NewGame, 2));
        f.press(KEY_BACKSPACE);
        assert_eq!(m_current().0, MenuId::Episode);
        f.press(KEY_BACKSPACE);
        assert_eq!(m_current().0, MenuId::Main);
        f.press(KEY_BACKSPACE); // no previous menu: stays
        assert_eq!(m_current().0, MenuId::Main);
        assert!(menuactive());
    }

    #[test]
    fn shareware_refuses_the_other_episodes() {
        let Some(mut f) = fixture(GameMode::Shareware) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.press(KEY_ESCAPE);
        f.press(KEY_ENTER);
        f.press(KEY_DOWNARROW); // episode 2
        f.press(KEY_ENTER);
        with(|s| {
            assert!(s.message_to_print);
            assert_eq!(s.message_string, msg::SWSTRING);
            assert_eq!(s.current, MenuId::ReadThis1);
        });
        // any key dismisses a message that needs no answer
        assert!(f.press(b'x' as i32));
        with(|s| assert!(!s.message_to_print));
    }

    #[test]
    fn a_yes_no_message_only_takes_y_n_space_escape() {
        let Some(mut f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        f.press(KEY_F10);
        with(|s| {
            assert!(s.message_to_print && s.message_needs_input);
            assert!(s.message_string.ends_with(msg::DOSY));
        });
        assert!(menuactive());
        assert!(!f.press(b'x' as i32), "other keys aren't consumed");
        with(|s| assert!(s.message_to_print));
        assert!(f.press(b'n' as i32));
        with(|s| assert!(!s.message_to_print));
        assert!(!menuactive(), "the menu was closed before the message");
        assert!(!doomstat::state().quit_requested);

        f.press(KEY_F10);
        f.press(KEY_ESCAPE);
        assert!(!doomstat::state().quit_requested, "escape declines");
        // (`y` asks the main loop to quit: covered in the integration
        // test, since it sets a process-wide flag.)
    }

    #[test]
    fn save_and_load_open_their_menus_after_their_own_checks() {
        let Some(mut f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        // F3 (load): the slot menu opens
        f.press(KEY_F3);
        with(|s| assert_eq!(s.current, MenuId::Load));
        f.press(KEY_ESCAPE);

        // F2 (save) without a game in progress: the original's own
        // "you can't save if you aren't playing" comes first.
        f.press(KEY_F2);
        with(|s| assert_eq!(s.message_string, msg::SAVEDEAD));
        f.press(b' ' as i32);

        // F9 (quickload) with no quicksave slot chosen
        f.press(KEY_F9);
        with(|s| assert_eq!(s.message_string, msg::QSAVESPOT));
    }

    #[test]
    fn text_metrics_use_the_hud_font() {
        let Some(_f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let font = hu_font();
        let w = |c: u8| patch_width(&font[(c - HU_FONTSTART) as usize]);
        assert_eq!(m_string_width("ab"), w(b'A') + w(b'B'), "case-insensitive");
        assert_eq!(m_string_width("a b"), w(b'A') + 4 + w(b'B'), "space is 4");
        let h = patch_height(&font[0]);
        assert_eq!(m_string_height("one"), h);
        assert_eq!(m_string_height("one\ntwo\nthree"), 3 * h);
    }

    #[test]
    fn skull_blinks_every_eight_tics() {
        let Some(_f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let skull = || with(|s| s.which_skull);
        assert_eq!(skull(), 0);
        for _ in 0..9 {
            m_ticker();
        }
        assert_eq!(skull(), 0);
        m_ticker(); // counter starts at 10: flips on the 10th tic
        assert_eq!(skull(), 1);
        for _ in 0..8 {
            m_ticker();
        }
        assert_eq!(skull(), 0);
    }

    #[test]
    fn drawer_paints_the_main_menu_and_the_message_box() {
        let Some(mut f) = fixture(GameMode::Registered) else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut v = VVideo::new();
        m_drawer(&mut v, &mut f.wad);
        assert!(v.screens[0].iter().all(|&b| b == 0), "closed: nothing");

        f.press(KEY_ESCAPE);
        m_drawer(&mut v, &mut f.wad);
        assert!(v.screens[0].iter().any(|&b| b != 0));

        // a message replaces the menu and is centred on screen
        let mut v2 = VVideo::new();
        f.press(KEY_ESCAPE);
        m_start_message("hello\nworld", None, false);
        m_drawer(&mut v2, &mut f.wad);
        let sw = SCREENWIDTH as usize;
        let lit: Vec<usize> = (0..v2.screens[0].len())
            .filter(|&i| v2.screens[0][i] != 0)
            .collect();
        assert!(!lit.is_empty());
        let (first, last) = (lit[0] / sw, lit[lit.len() - 1] / sw);
        assert!((80..120).contains(&first) && (80..130).contains(&last));
        m_stop_message();
    }
}
