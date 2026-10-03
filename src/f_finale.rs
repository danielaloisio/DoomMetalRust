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
//	Game completion, final screen animation.
//
//-----------------------------------------------------------------------------

//! Rust port of `f_finale.h` / `f_finale.c`. Game completion, final screen
//! animation (the original's own description): the end-of-episode
//! text over a tiled flat (typed out letter by letter), the episode's
//! end picture (`HELP2`/`CREDIT`/`VICTORY2`/`ENDPIC`, or the scrolling
//! "The End" bunny picture for episode 3), and DOOM II's "cast call"
//! after MAP30.
//!
//! # State and flow
//!
//! `finalestage`/`finalecount`, the chosen flat/text and the whole
//! cast-call state are one [`FinaleState`] in a `thread_local!` behind
//! C-named free functions ([`f_start_finale`], [`f_responder`],
//! [`f_ticker`], [`f_drawer`], [`f_start_cast`]). What the original
//! does through globals goes to `doomstat` (`gameaction`, `gamestate`,
//! `automapactive`, `wipegamestate`).
//!
//! # Divergences
//!
//! * `c > HU_FONTSIZE` in the original's text loops is off by one (it
//!   would index the font one past its end for `` ` ``); such a
//!   character is treated as a space here.
//! * `finaleflat`/`finaletext` are left unset by the original for
//!   episodes/maps that have no finale (`default:` branches); here they
//!   fall back to the last valid values or the DOOM II text, and
//!   [`f_start_finale`] never runs for those in a real game.
//! * The cast drawer takes the sprite tables and `firstspritelump`
//!   as parameters (they are renderer state).

use std::cell::RefCell;

use crate::d_englsh as msg;
use crate::d_event::{EvType, Event, GameAction};
use crate::d_player::Player;
use crate::doomdef::{GameMode, MAXPLAYERS, SCREENHEIGHT, SCREENWIDTH};
use crate::doomstat;
use crate::hu_stuff::{hu_font, HU_FONTSIZE, HU_FONTSTART};
use crate::info::{MobjType, StateNum, MOBJINFO, STATES};
use crate::r_defs::SpriteDef;
use crate::s_sound::{s_change_music, s_start_music, s_start_sound};
use crate::sounds::{MusicEnum, Sfx};
use crate::v_video::{patch_width, VVideo};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// (`TEXTSPEED`) tics per character.
pub const TEXTSPEED: i32 = 3;
/// (`TEXTWAIT`) tics after the last character.
pub const TEXTWAIT: i32 = 250;

/// (`FF_FRAMEMASK`)
const FF_FRAMEMASK: i32 = 0x7fff;

/// (`castorder[]`) name and type of each cast member.
pub const CASTORDER: [(&str, MobjType); 17] = [
    (msg::CC_ZOMBIE, MobjType::MtPossessed),
    (msg::CC_SHOTGUN, MobjType::MtShotguy),
    (msg::CC_HEAVY, MobjType::MtChainguy),
    (msg::CC_IMP, MobjType::MtTroop),
    (msg::CC_DEMON, MobjType::MtSergeant),
    (msg::CC_LOST, MobjType::MtSkull),
    (msg::CC_CACO, MobjType::MtHead),
    (msg::CC_HELL, MobjType::MtKnight),
    (msg::CC_BARON, MobjType::MtBruiser),
    (msg::CC_ARACH, MobjType::MtBaby),
    (msg::CC_PAIN, MobjType::MtPain),
    (msg::CC_REVEN, MobjType::MtUndead),
    (msg::CC_MANCU, MobjType::MtFatso),
    (msg::CC_ARCH, MobjType::MtVile),
    (msg::CC_SPIDER, MobjType::MtSpider),
    (msg::CC_CYBER, MobjType::MtCyborg),
    (msg::CC_HERO, MobjType::MtPlayer),
];

/// The file-scope statics of `f_finale.c`.
struct FinaleState {
    finalestage: i32,
    finalecount: i32,
    finaletext: &'static str,
    finaleflat: &'static str,

    // Cast call
    castnum: usize,
    casttics: i32,
    caststate: StateNum,
    castdeath: bool,
    castframes: i32,
    castonmelee: i32,
    castattacking: bool,

    /// `F_BunnyScroll`'s `static int laststage`.
    laststage: i32,
}

impl FinaleState {
    fn new() -> Self {
        FinaleState {
            finalestage: 0,
            finalecount: 0,
            finaletext: msg::C1TEXT,
            finaleflat: "F_SKY1",
            castnum: 0,
            casttics: 0,
            caststate: StateNum::SNull,
            castdeath: false,
            castframes: 0,
            castonmelee: 0,
            castattacking: false,
            laststage: 0,
        }
    }
}

thread_local! {
    static FINALE: RefCell<FinaleState> = RefCell::new(FinaleState::new());
}

fn with<R>(f: impl FnOnce(&mut FinaleState) -> R) -> R {
    FINALE.with(|s| f(&mut s.borrow_mut()))
}

/// `finalestage` (0 = text, 1 = end picture, 2 = cast call).
pub fn f_finalestage() -> i32 {
    with(|s| s.finalestage)
}

/// `finalecount`.
pub fn f_finalecount() -> i32 {
    with(|s| s.finalecount)
}

/// Port of `F_StartFinale`. Picks the flat, text and music for the
/// episode/map just finished and switches the game to `GS_FINALE`.
pub fn f_start_finale(wad: &mut WadFiles) {
    let st = doomstat::state_mut();
    st.gameaction = GameAction::Nothing;
    st.gamestate = crate::doomdef::GameState::Finale;
    st.automapactive = false;
    let (gamemode, episode, map) = (st.gamemode, st.gameepisode, st.gamemap);

    // Okay - IWAD dependend stuff.
    // This has been changed severly, and some stuff might have changed
    // in the process (the original's own comment).
    let (flat, text): (Option<&'static str>, Option<&'static str>) = match gamemode {
        // DOOM 1 - E1, E3 or E4, but each nine missions
        GameMode::Shareware | GameMode::Registered | GameMode::Retail => {
            s_change_music(wad, MusicEnum::MusVictor as i32, true);
            match episode {
                1 => (Some("FLOOR4_8"), Some(msg::E1TEXT)),
                2 => (Some("SFLR6_1"), Some(msg::E2TEXT)),
                3 => (Some("MFLR8_4"), Some(msg::E3TEXT)),
                4 => (Some("MFLR8_3"), Some(msg::E4TEXT)),
                _ => (None, None),
            }
        }
        // DOOM II and missions packs with E1, M34
        GameMode::Commercial => {
            s_change_music(wad, MusicEnum::MusReadM as i32, true);
            match map {
                6 => (Some("SLIME16"), Some(msg::C1TEXT)),
                11 => (Some("RROCK14"), Some(msg::C2TEXT)),
                20 => (Some("RROCK07"), Some(msg::C3TEXT)),
                30 => (Some("RROCK17"), Some(msg::C4TEXT)),
                15 => (Some("RROCK13"), Some(msg::C5TEXT)),
                31 => (Some("RROCK19"), Some(msg::C6TEXT)),
                _ => (None, None),
            }
        }
        // Indeterminate.
        _ => {
            s_change_music(wad, MusicEnum::MusReadM as i32, true);
            (Some("F_SKY1"), Some(msg::C1TEXT)) // Not used anywhere else. FIXME - other text, music?
        }
    };

    with(|s| {
        if let Some(f) = flat {
            s.finaleflat = f;
        }
        if let Some(t) = text {
            s.finaletext = t;
        }
        s.finalestage = 0;
        s.finalecount = 0;
    });
}

/// Port of `F_Responder`.
pub fn f_responder(event: &Event) -> bool {
    if with(|s| s.finalestage) == 2 {
        return f_cast_responder(event);
    }
    false
}

/// Port of `F_Ticker`. `players` is the game's player array (any
/// button press moves DOOM II's finale on); `wad` is for the music.
pub fn f_ticker(players: &[Player], wad: &mut WadFiles) {
    let st = doomstat::state();
    let (gamemode, gamemap, gameepisode) = (st.gamemode, st.gamemap, st.gameepisode);

    // check for skipping
    if gamemode == GameMode::Commercial && with(|s| s.finalecount) > 50 {
        // go on to the next level
        let pressed = players
            .iter()
            .take(MAXPLAYERS as usize)
            .any(|p| p.cmd.buttons != 0);
        if pressed {
            if gamemap == 30 {
                f_start_cast(wad);
            } else {
                doomstat::state_mut().gameaction = GameAction::WorldDone;
            }
        }
    }

    // advance animation
    with(|s| s.finalecount += 1);

    if with(|s| s.finalestage) == 2 {
        f_cast_ticker();
        return;
    }

    if gamemode == GameMode::Commercial {
        return;
    }

    // (the C `strlen`)
    let (stage, count, len) = with(|s| (s.finalestage, s.finalecount, s.finaletext.len() as i32));
    if stage == 0 && count > len * TEXTSPEED + TEXTWAIT {
        with(|s| {
            s.finalecount = 0;
            s.finalestage = 1;
        });
        doomstat::state_mut().wipegamestate = None; // force a wipe
        if gameepisode == 3 {
            s_start_music(wad, MusicEnum::MusBunny as i32);
        }
    }
}

/// Port of `F_TextWrite`. Tiles the flat over the screen and types
/// the text out on it.
fn f_text_write(v: &mut VVideo, wad: &mut WadFiles) {
    let (flat, text, finalecount) = with(|s| (s.finaleflat, s.finaletext, s.finalecount));

    // erase the entire screen to a tiled background
    let src = wad.cache_lump_name(flat, PurgeTag::Cache).to_vec();
    let sw = SCREENWIDTH as usize;
    let mut dest = 0usize;
    for y in 0..SCREENHEIGHT as usize {
        let row = &src[(y & 63) << 6..((y & 63) << 6) + 64];
        for _ in 0..sw / 64 {
            v.screens[0][dest..dest + 64].copy_from_slice(row);
            dest += 64;
        }
        if sw & 63 != 0 {
            v.screens[0][dest..dest + (sw & 63)].copy_from_slice(&row[..sw & 63]);
            dest += sw & 63;
        }
    }

    v.v_mark_rect(0, 0, SCREENWIDTH, SCREENHEIGHT);

    // draw some of the text onto the screen
    let font = hu_font();
    let (mut cx, mut cy) = (10, 10);
    let mut count = (finalecount - 10) / TEXTSPEED;
    if count < 0 {
        count = 0;
    }
    for ch in text.bytes() {
        if count == 0 {
            break;
        }
        count -= 1;

        if ch == b'\n' {
            cx = 10;
            cy += 11;
            continue;
        }

        let c = ch.to_ascii_uppercase() as i32 - HU_FONTSTART as i32;
        if c < 0 || c >= HU_FONTSIZE as i32 {
            cx += 4;
            continue;
        }

        let w = patch_width(&font[c as usize]);
        if cx + w > SCREENWIDTH {
            break;
        }
        v.v_draw_patch(cx, cy, 0, &font[c as usize]);
        cx += w;
    }
}

/// The sound `F_CastTicker` plays when entering `st` (0 = none).
fn cast_sound(st: StateNum) -> Option<Sfx> {
    use StateNum::*;
    Some(match st {
        SPlayAtk1 => Sfx::SfxDshtgn,
        SPossAtk2 => Sfx::SfxPistol,
        SSposAtk2 => Sfx::SfxShotgn,
        SVileAtk2 => Sfx::SfxVilatk,
        SSkelFist2 => Sfx::SfxSkeswg,
        SSkelFist4 => Sfx::SfxSkepch,
        SSkelMiss2 => Sfx::SfxSkeatk,
        SFattAtk8 | SFattAtk5 | SFattAtk2 => Sfx::SfxFirsht,
        SCposAtk2 | SCposAtk3 | SCposAtk4 => Sfx::SfxShotgn,
        STrooAtk3 => Sfx::SfxClaw,
        SSargAtk2 => Sfx::SfxSgtatk,
        SBossAtk2 | SBos2Atk2 | SHeadAtk2 => Sfx::SfxFirsht,
        SSkullAtk2 => Sfx::SfxSklatk,
        SSpidAtk2 | SSpidAtk3 => Sfx::SfxShotgn,
        SBspiAtk2 => Sfx::SfxPlasma,
        SCyberAtk2 | SCyberAtk4 | SCyberAtk6 => Sfx::SfxRlaunc,
        SPainAtk3 => Sfx::SfxSklatk,
        _ => return None,
    })
}

/// Port of `F_StartCast`.
pub fn f_start_cast(wad: &mut WadFiles) {
    doomstat::state_mut().wipegamestate = None; // force a screen wipe
    with(|s| {
        s.castnum = 0;
        s.caststate = MOBJINFO[CASTORDER[s.castnum].1 as usize].seestate;
        s.casttics = STATES[s.caststate as usize].tics;
        s.castdeath = false;
        s.finalestage = 2;
        s.castframes = 0;
        s.castonmelee = 0;
        s.castattacking = false;
    });
    s_change_music(wad, MusicEnum::MusEvil as i32, true);
}

/// Port of `F_CastTicker`.
fn f_cast_ticker() {
    with(|s| {
        s.casttics -= 1;
        if s.casttics > 0 {
            return; // not time to change state yet
        }

        let info = &MOBJINFO[CASTORDER[s.castnum].1 as usize];
        let mut stop_attack = false;

        let cur = STATES[s.caststate as usize];
        if cur.tics == -1 || cur.nextstate == StateNum::SNull {
            // switch from deathstate to next monster
            s.castnum += 1;
            s.castdeath = false;
            if s.castnum == CASTORDER.len() {
                s.castnum = 0;
            }
            let info = &MOBJINFO[CASTORDER[s.castnum].1 as usize];
            if info.seesound != Sfx::SfxNone {
                s_start_sound(None, info.seesound);
            }
            s.caststate = info.seestate;
            s.castframes = 0;
        } else if s.caststate == StateNum::SPlayAtk1 {
            // Oh, gross hack! (the original's own comment)
            stop_attack = true;
        } else {
            // just advance to next state in animation
            let st = cur.nextstate;
            s.caststate = st;
            s.castframes += 1;

            // sound hacks....
            if let Some(sfx) = cast_sound(st) {
                s_start_sound(None, sfx);
            }
        }

        if !stop_attack {
            let info = &MOBJINFO[CASTORDER[s.castnum].1 as usize];
            if s.castframes == 12 {
                // go into attack frame
                s.castattacking = true;
                if s.castonmelee != 0 {
                    s.caststate = info.meleestate;
                } else {
                    s.caststate = info.missilestate;
                }
                s.castonmelee ^= 1;
                if s.caststate == StateNum::SNull {
                    if s.castonmelee != 0 {
                        s.caststate = info.meleestate;
                    } else {
                        s.caststate = info.missilestate;
                    }
                }
            }

            if s.castattacking && (s.castframes == 24 || s.caststate == info.seestate) {
                stop_attack = true;
            }
        }

        if stop_attack {
            s.castattacking = false;
            s.castframes = 0;
            s.caststate = info.seestate;
        }

        s.casttics = STATES[s.caststate as usize].tics;
        if s.casttics == -1 {
            s.casttics = 15;
        }
    });
}

/// Port of `F_CastResponder`.
fn f_cast_responder(ev: &Event) -> bool {
    if ev.event_type != EvType::KeyDown {
        return false;
    }

    with(|s| {
        if s.castdeath {
            return true; // already in dying frames
        }

        // go into death frame
        s.castdeath = true;
        let info = &MOBJINFO[CASTORDER[s.castnum].1 as usize];
        s.caststate = info.deathstate;
        s.casttics = STATES[s.caststate as usize].tics;
        s.castframes = 0;
        s.castattacking = false;
        if info.deathsound != Sfx::SfxNone {
            s_start_sound(None, info.deathsound);
        }

        true
    })
}

/// Port of `F_CastPrint`. Prints `text` centred at the bottom.
fn f_cast_print(v: &mut VVideo, text: &str) {
    let font = hu_font();

    // find width
    let mut width = 0;
    for ch in text.bytes() {
        let c = ch.to_ascii_uppercase() as i32 - HU_FONTSTART as i32;
        if c < 0 || c >= HU_FONTSIZE as i32 {
            width += 4;
            continue;
        }
        width += patch_width(&font[c as usize]);
    }

    // draw it
    let mut cx = 160 - width / 2;
    for ch in text.bytes() {
        let c = ch.to_ascii_uppercase() as i32 - HU_FONTSTART as i32;
        if c < 0 || c >= HU_FONTSIZE as i32 {
            cx += 4;
            continue;
        }

        let w = patch_width(&font[c as usize]);
        v.v_draw_patch(cx, 180, 0, &font[c as usize]);
        cx += w;
    }
}

/// Port of `F_CastDrawer`.
fn f_cast_drawer(v: &mut VVideo, wad: &mut WadFiles, sprites: &[SpriteDef], firstspritelump: i32) {
    // erase the entire screen to a background
    let bossback = wad.cache_lump_name("BOSSBACK", PurgeTag::Cache).to_vec();
    v.v_draw_patch(0, 0, 0, &bossback);

    let (castnum, caststate) = with(|s| (s.castnum, s.caststate));
    f_cast_print(v, CASTORDER[castnum].0);

    // draw the current frame in the middle of the screen
    let state = STATES[caststate as usize];
    let sprdef = &sprites[state.sprite as usize];
    let sprframe = &sprdef.spriteframes[(state.frame & FF_FRAMEMASK) as usize];
    let lump = sprframe.lump[0];
    let flip = sprframe.flip[0] != 0;

    let patch = wad
        .cache_lump_num((lump as i32 + firstspritelump) as usize, PurgeTag::Cache)
        .to_vec();
    if flip {
        v.v_draw_patch_flipped(160, 170, 0, &patch);
    } else {
        v.v_draw_patch(160, 170, 0, &patch);
    }
}

/// Port of `F_DrawPatchCol`. Draws column `col` of `patch` at screen
/// column `x` (unclipped, straight into `screens[0]`).
fn f_draw_patch_col(v: &mut VVideo, x: i32, patch: &[u8], col: i32) {
    let sw = SCREENWIDTH as usize;
    let ofs_at = 8 + 4 * col as usize;
    let mut column = i32::from_le_bytes(patch[ofs_at..ofs_at + 4].try_into().unwrap()) as usize;
    while patch[column] != 0xff {
        let topdelta = patch[column] as usize;
        let count = patch[column + 1] as usize;
        let mut dest = topdelta * sw + x as usize;
        for &b in &patch[column + 3..column + 3 + count] {
            v.screens[0][dest] = b;
            dest += sw;
        }
        column += count + 4;
    }
}

/// Port of `F_BunnyScroll`. The scrolling "The End" picture of
/// episode 3.
fn f_bunny_scroll(v: &mut VVideo, wad: &mut WadFiles) {
    let p1 = wad.cache_lump_name("PFUB2", PurgeTag::Level).to_vec();
    let p2 = wad.cache_lump_name("PFUB1", PurgeTag::Level).to_vec();

    v.v_mark_rect(0, 0, SCREENWIDTH, SCREENHEIGHT);

    let finalecount = with(|s| s.finalecount);
    let scrolled = (320 - (finalecount - 230) / 2).clamp(0, 320);

    for x in 0..SCREENWIDTH {
        if x + scrolled < 320 {
            f_draw_patch_col(v, x, &p1, x + scrolled);
        } else {
            f_draw_patch_col(v, x, &p2, x + scrolled - 320);
        }
    }

    if finalecount < 1130 {
        return;
    }
    if finalecount < 1180 {
        let end0 = wad.cache_lump_name("END0", PurgeTag::Cache).to_vec();
        v.v_draw_patch(
            (SCREENWIDTH - 13 * 8) / 2,
            (SCREENHEIGHT - 8 * 8) / 2,
            0,
            &end0,
        );
        with(|s| s.laststage = 0);
        return;
    }

    let mut stage = (finalecount - 1180) / 5;
    if stage > 6 {
        stage = 6;
    }
    let laststage = with(|s| s.laststage);
    if stage > laststage {
        s_start_sound(None, Sfx::SfxPistol);
        with(|s| s.laststage = stage);
    }

    let name = format!("END{stage}");
    let patch = wad.cache_lump_name(&name, PurgeTag::Cache).to_vec();
    v.v_draw_patch(
        (SCREENWIDTH - 13 * 8) / 2,
        (SCREENHEIGHT - 8 * 8) / 2,
        0,
        &patch,
    );
}

/// Port of `F_Drawer`.
pub fn f_drawer(v: &mut VVideo, wad: &mut WadFiles, sprites: &[SpriteDef], firstspritelump: i32) {
    let stage = with(|s| s.finalestage);
    if stage == 2 {
        f_cast_drawer(v, wad, sprites, firstspritelump);
        return;
    }

    if stage == 0 {
        f_text_write(v, wad);
    } else {
        let st = doomstat::state();
        let name = match st.gameepisode {
            1 => Some(if st.gamemode == GameMode::Retail {
                "CREDIT"
            } else {
                "HELP2"
            }),
            2 => Some("VICTORY2"),
            3 => {
                f_bunny_scroll(v, wad);
                None
            }
            4 => Some("ENDPIC"),
            _ => None,
        };
        if let Some(name) = name {
            let patch = wad.cache_lump_name(name, PurgeTag::Cache).to_vec();
            v.v_draw_patch(0, 0, 0, &patch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_down() -> Event {
        Event {
            event_type: EvType::KeyDown,
            data1: b' ' as i32,
            data2: 0,
            data3: 0,
        }
    }

    fn wad() -> Option<WadFiles> {
        let path = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        let mut w = WadFiles::new();
        w.init_file(path);
        Some(w)
    }

    fn started_cast() -> WadFiles {
        // (the sound/music side is inert without `s_init`; the WAD is
        // only needed by the API)
        let mut w = wad().unwrap_or_default();
        with(|s| *s = FinaleState::new());
        f_start_cast(&mut w);
        w
    }

    #[test]
    fn cast_order_matches_the_original() {
        assert_eq!(CASTORDER.len(), 17);
        assert_eq!(CASTORDER[0], (msg::CC_ZOMBIE, MobjType::MtPossessed));
        assert_eq!(CASTORDER[16], (msg::CC_HERO, MobjType::MtPlayer));
    }

    #[test]
    fn attack_frames_have_their_own_sounds() {
        use StateNum::*;
        assert_eq!(cast_sound(SPossAtk2), Some(Sfx::SfxPistol));
        assert_eq!(cast_sound(SPlayAtk1), Some(Sfx::SfxDshtgn));
        assert_eq!(cast_sound(SFattAtk5), Some(Sfx::SfxFirsht));
        assert_eq!(cast_sound(SCyberAtk6), Some(Sfx::SfxRlaunc));
        assert_eq!(cast_sound(SPossRun1), None);
    }

    #[test]
    fn start_cast_begins_with_the_zombieman_walking() {
        started_cast();
        with(|s| {
            assert_eq!(s.finalestage, 2);
            assert_eq!(s.castnum, 0);
            assert_eq!(
                s.caststate,
                MOBJINFO[MobjType::MtPossessed as usize].seestate
            );
            assert_eq!(s.casttics, STATES[s.caststate as usize].tics);
            assert!(!s.castdeath && !s.castattacking);
        });
    }

    #[test]
    fn the_cast_walks_attacks_and_moves_on_when_killed() {
        started_cast();
        // the walk cycle plus an attack every 12 frames, back to walking
        let mut attacked = false;
        let mut seen_states = std::collections::HashSet::new();
        for _ in 0..600 {
            f_cast_ticker();
            with(|s| {
                seen_states.insert(s.caststate);
                if s.castattacking {
                    attacked = true;
                }
            });
        }
        assert!(attacked, "the zombieman shot at least once");
        assert!(
            seen_states.len() >= 6,
            "{} distinct frames",
            seen_states.len()
        );
        with(|s| assert_eq!(s.castnum, 0, "still the zombieman: nobody killed him"));

        // a key press kills it (once)
        assert!(f_cast_responder(&key_down()));
        let death = with(|s| {
            assert!(s.castdeath);
            s.caststate
        });
        assert_eq!(death, MOBJINFO[MobjType::MtPossessed as usize].deathstate);
        assert!(f_cast_responder(&key_down()), "already dying: still eaten");
        with(|s| assert_eq!(s.caststate, death, "no second death"));

        // the death animation plays out and the next monster walks in
        for _ in 0..400 {
            f_cast_ticker();
            if with(|s| s.castnum) == 1 {
                break;
            }
        }
        with(|s| {
            assert_eq!(s.castnum, 1, "the shotgun guy is next");
            assert!(!s.castdeath);
            assert_eq!(s.caststate, MOBJINFO[MobjType::MtShotguy as usize].seestate);
        });
    }

    #[test]
    fn the_cast_loops_back_to_the_first_monster_after_the_hero() {
        started_cast();
        let mut kills = 0;
        while kills < 17 {
            f_cast_responder(&key_down());
            kills += 1;
            let before = with(|s| s.castnum);
            for _ in 0..2000 {
                f_cast_ticker();
                if with(|s| s.castnum) != before {
                    break;
                }
            }
        }
        with(|s| assert_eq!(s.castnum, 0, "17 deaths later we are back at the zombieman"));
    }

    #[test]
    fn only_key_downs_reach_the_cast() {
        started_cast();
        let up = Event {
            event_type: EvType::KeyUp,
            ..key_down()
        };
        assert!(!f_cast_responder(&up));
        with(|s| assert!(!s.castdeath));
        // f_responder only forwards during the cast stage
        with(|s| s.finalestage = 0);
        assert!(!f_responder(&key_down()));
        with(|s| s.finalestage = 2);
        assert!(f_responder(&key_down()));
    }
}
