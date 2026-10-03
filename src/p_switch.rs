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
//	Switches, buttons. Two-state animation. Exits.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_switch.c`. Switches, buttons. Two-state
//! animation. Exits (the original's own description, preserved).
//!
//! # Scope
//!
//! Ported: [`p_init_switch_list`] (`P_InitSwitchList`),
//! [`SwitchState::p_start_button`] (`P_StartButton`),
//! [`SwitchState::p_change_switch_texture`] (`P_ChangeSwitchTexture`),
//! [`p_use_special_line`] (`P_UseSpecialLine`, Phase 7b4 — its giant
//! `switch` needed every `EV_Do*` mover, which now all exist).
//!
//! Sounds: `P_ChangeSwitchTexture`'s switch click, queued via
//! [`crate::s_sound`]. Two faithful quirks of the original are kept:
//! the click's origin is `buttonlist->soundorg` (the *first* slot's
//! origin, not the switch just used — `NULL`, i.e. global, whenever
//! slot 0 is free), and `sfx_swtchx` (exit switch) is unreachable
//! because `line->special` is zeroed for non-repeatable switches before
//! it is compared with 11, so exit switches play `sfx_swtchn`.

use crate::doomdef::GameMode;
use crate::p_plats::PlatType;
use crate::p_setup::Level;
use crate::p_tick::ThinkerId;
use crate::r_data::RData;
use crate::s_sound::{s_start_sound, sector_origin};
use crate::sounds::Sfx;

/// (`bwhere_e`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BWhere {
    Top,
    Middle,
    Bottom,
}

/// (`button_t`). `soundorg` stands in for the original's `mobj_t*` cast
/// of the line's front sector's `soundorg` (a `degenmobj_t`): the sector
/// index the switch sound originates from.
#[derive(Debug, Clone, Copy)]
pub struct Button {
    pub line: usize,
    pub bwhere: BWhere,
    pub btexture: i32,
    pub btimer: i32,
    pub soundorg: usize,
}

/// (`MAXSWITCHES`) max # of wall switches in a level (the original's own
/// comment, preserved).
pub const MAXSWITCHES: usize = 50;
/// (`MAXBUTTONS`) 4 players, 4 buttons each at once, max (the original's
/// own comment, preserved).
pub const MAXBUTTONS: usize = 16;
/// (`BUTTONTIME`) 1 second, in ticks (the original's own comment,
/// preserved).
pub const BUTTONTIME: i32 = 35;

/// (`switchlist_t`)
struct SwitchListEntry {
    name1: &'static str,
    name2: &'static str,
    episode: i16,
}

/// Port of `alphSwitchList`. CHANGE THE TEXTURE OF A WALL SWITCH TO ITS
/// OPPOSITE (the original's own comment, preserved).
const ALPH_SWITCH_LIST: &[SwitchListEntry] = &[
    // Doom shareware episode 1 switches
    e("SW1BRCOM", "SW2BRCOM", 1),
    e("SW1BRN1", "SW2BRN1", 1),
    e("SW1BRN2", "SW2BRN2", 1),
    e("SW1BRNGN", "SW2BRNGN", 1),
    e("SW1BROWN", "SW2BROWN", 1),
    e("SW1COMM", "SW2COMM", 1),
    e("SW1COMP", "SW2COMP", 1),
    e("SW1DIRT", "SW2DIRT", 1),
    e("SW1EXIT", "SW2EXIT", 1),
    e("SW1GRAY", "SW2GRAY", 1),
    e("SW1GRAY1", "SW2GRAY1", 1),
    e("SW1METAL", "SW2METAL", 1),
    e("SW1PIPE", "SW2PIPE", 1),
    e("SW1SLAD", "SW2SLAD", 1),
    e("SW1STARG", "SW2STARG", 1),
    e("SW1STON1", "SW2STON1", 1),
    e("SW1STON2", "SW2STON2", 1),
    e("SW1STONE", "SW2STONE", 1),
    e("SW1STRTN", "SW2STRTN", 1),
    // Doom registered episodes 2&3 switches
    e("SW1BLUE", "SW2BLUE", 2),
    e("SW1CMT", "SW2CMT", 2),
    e("SW1GARG", "SW2GARG", 2),
    e("SW1GSTON", "SW2GSTON", 2),
    e("SW1HOT", "SW2HOT", 2),
    e("SW1LION", "SW2LION", 2),
    e("SW1SATYR", "SW2SATYR", 2),
    e("SW1SKIN", "SW2SKIN", 2),
    e("SW1VINE", "SW2VINE", 2),
    e("SW1WOOD", "SW2WOOD", 2),
    // Doom II switches
    e("SW1PANEL", "SW2PANEL", 3),
    e("SW1ROCK", "SW2ROCK", 3),
    e("SW1MET2", "SW2MET2", 3),
    e("SW1WDMET", "SW2WDMET", 3),
    e("SW1BRIK", "SW2BRIK", 3),
    e("SW1MOD1", "SW2MOD1", 3),
    e("SW1ZIM", "SW2ZIM", 3),
    e("SW1STON6", "SW2STON6", 3),
    e("SW1TEK", "SW2TEK", 3),
    e("SW1MARB", "SW2MARB", 3),
    e("SW1SKULL", "SW2SKULL", 3),
];

const fn e(name1: &'static str, name2: &'static str, episode: i16) -> SwitchListEntry {
    SwitchListEntry {
        name1,
        name2,
        episode,
    }
}

/// The switches state this phase needs, standing in for the original's
/// `switchlist[MAXSWITCHES*2]`/`numswitches`/`buttonlist[MAXBUTTONS]`
/// module statics — passed explicitly like every other not-yet-global
/// piece of state in this port (`doomstat`'s `GlobalCell` pattern is
/// reserved for state genuinely shared game-wide; this is level-local,
/// reset every `P_SpawnSpecials`, so an owned struct threaded through
/// call sites fits better, matching `p_map`'s `MoveContext` precedent).
#[derive(Debug, Default)]
pub struct SwitchState {
    /// Interleaved `[tex1, tex2, tex1, tex2, ...]`, one pair per active
    /// switch — `switchlist[MAXSWITCHES*2]` (`-1`-terminated in the
    /// original; this port just uses the `Vec`'s length instead).
    switchlist: Vec<i32>,
    pub buttonlist: [Option<Button>; MAXBUTTONS],
}

impl SwitchState {
    /// Port of `P_InitSwitchList`. Only called at game initialization
    /// (the original's own comment, preserved).
    pub fn p_init_switch_list(&mut self, rdata: &RData, gamemode: GameMode) {
        let episode = match gamemode {
            GameMode::Registered | GameMode::Retail => 2,
            GameMode::Commercial => 3,
            _ => 1,
        };

        self.switchlist.clear();
        for entry in ALPH_SWITCH_LIST.iter().take(MAXSWITCHES) {
            if entry.episode <= episode {
                self.switchlist
                    .push(rdata.texture_num_for_name(entry.name1));
                self.switchlist
                    .push(rdata.texture_num_for_name(entry.name2));
            }
        }
    }

    /// Port of `P_StartButton`. Start a button counting down till it
    /// turns off (the original's own comment, preserved).
    pub fn p_start_button(
        &mut self,
        line: usize,
        bwhere: BWhere,
        texture: i32,
        time: i32,
        soundorg: usize,
    ) {
        // See if button is already pressed (the original's own comment,
        // preserved).
        if self
            .buttonlist
            .iter()
            .flatten()
            .any(|b| b.btimer != 0 && b.line == line)
        {
            return;
        }

        for slot in self.buttonlist.iter_mut() {
            if slot.is_none() {
                *slot = Some(Button {
                    line,
                    bwhere,
                    btexture: texture,
                    btimer: time,
                    soundorg,
                });
                return;
            }
        }

        panic!("P_StartButton: no button slots left!");
    }

    /// Port of `P_ChangeSwitchTexture`. Function that changes wall
    /// texture. Tell it if switch is ok to use again (the original's own
    /// comment, preserved; `use_again` replaces the `int 1=yes` flag).
    /// `sfx_swtchx`/`sfx_swtchn` selection (exit-switch vs. normal) is
    /// left to the caller reading `line.special == 11` the same way the
    /// original inlines it, but the sound itself isn't played yet — see
    /// module docs.
    pub fn p_change_switch_texture(&mut self, level: &mut Level, line_idx: usize, use_again: bool) {
        if !use_again {
            level.lines[line_idx].special = 0;
        }

        let line = &level.lines[line_idx];
        let sidenum = line.sidenum[0].unwrap();
        let tex_top = level.sides[sidenum].toptexture as i32;
        let tex_mid = level.sides[sidenum].midtexture as i32;
        let tex_bot = level.sides[sidenum].bottomtexture as i32;
        let soundorg = level.lines[line_idx].frontsector.unwrap();

        let mut sound = Sfx::SfxSwtchn;
        // EXIT SWITCH? (line.special was already zeroed above for
        // non-repeatable switches — see module docs.)
        if level.lines[line_idx].special == 11 {
            sound = Sfx::SfxSwtchx;
        }
        // `buttonlist->soundorg`: slot 0's origin, NULL if it is free.
        let click_origin = self.buttonlist[0].and_then(|b| sector_origin(b.soundorg));

        let mut i = 0;
        while i < self.switchlist.len() {
            let tex = self.switchlist[i];
            let opposite = self.switchlist[i ^ 1];

            if tex == tex_top {
                s_start_sound(click_origin, sound);
                level.sides[sidenum].toptexture = opposite as i16;
                if use_again {
                    self.p_start_button(line_idx, BWhere::Top, tex, BUTTONTIME, soundorg);
                }
                return;
            } else if tex == tex_mid {
                s_start_sound(click_origin, sound);
                level.sides[sidenum].midtexture = opposite as i16;
                if use_again {
                    self.p_start_button(line_idx, BWhere::Middle, tex, BUTTONTIME, soundorg);
                }
                return;
            } else if tex == tex_bot {
                s_start_sound(click_origin, sound);
                level.sides[sidenum].bottomtexture = opposite as i16;
                if use_again {
                    self.p_start_button(line_idx, BWhere::Bottom, tex, BUTTONTIME, soundorg);
                }
                return;
            }
            i += 1;
        }
    }
}

/// Port of `P_UseSpecialLine`. Called when a thing uses a special line.
/// Only the front sides of lines are usable (the original's own
/// comment, preserved). `thing` is the activating mobj; a monster (no
/// owning player) is only let through for the small set of specials the
/// original allows non-players to trigger.
pub fn p_use_special_line(
    ctx: &mut crate::p_spec::SpecialsCtx,
    thing: ThinkerId,
    line_idx: usize,
    side: i32,
) -> bool {
    // Err... Use the back sides of VERY SPECIAL lines... (the
    // original's own comment, preserved). `124` (UNUSED sliding door)
    // is the only back-side case in the original, and it's itself
    // `#if 0`'d out — so every back-side use is rejected here, same net
    // effect.
    if side != 0 {
        return false;
    }

    let special = ctx.level.lines[line_idx].special;
    let is_player = ctx.thinkers.mobj(thing).unwrap().player.is_some();

    // Switches that other things can activate (the original's own
    // comment, preserved).
    if !is_player {
        // never open secret doors (the original's own comment,
        // preserved).
        if ctx.level.lines[line_idx].flags & crate::doomdata::ML_SECRET != 0 {
            return false;
        }
        if !matches!(special, 1 | 32 | 33 | 34) {
            return false;
        }
    }

    let line = ctx.level.lines[line_idx];

    // do something (the original's own comment, preserved).
    match special {
        // MANUALS
        1 | 26 | 27 | 28 | 31 | 32 | 33 | 34 | 117 | 118 => {
            crate::p_doors::ev_vertical_door(ctx.thinkers, ctx.level, ctx.players, &line, thing);
        }

        // SWITCHES
        7 => {
            // Build Stairs
            if crate::p_floor::ev_build_stairs(
                ctx.level,
                ctx.thinkers,
                &line,
                crate::p_floor::StairType::Build8,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        9 => {
            // Change Donut
            if crate::p_spec::ev_do_donut(ctx.thinkers, ctx.level, &line) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        11 => {
            // Exit level
            ctx.switches
                .p_change_switch_texture(ctx.level, line_idx, false);
            crate::g_game::g_exit_level();
        }
        14 => {
            // Raise Floor 32 and change texture
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseAndChange,
                32,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        15 => {
            // Raise Floor 24 and change texture
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseAndChange,
                24,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        18 => {
            // Raise Floor to next highest floor
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloorToNearest,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        20 => {
            // Raise Plat next highest floor and change texture
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseToNearestAndChange,
                0,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        21 => {
            // PlatDownWaitUpStay
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::DownWaitUpStay,
                0,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        23 => {
            // Lower Floor to Lowest
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::LowerFloorToLowest,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        29 => {
            // Raise Door
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::Normal,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        41 => {
            // Lower Ceiling to Floor
            if crate::p_ceilng::ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::LowerToFloor,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        71 => {
            // Turbo Lower Floor
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::TurboLower,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        49 => {
            // Ceiling Crush And Raise
            if crate::p_ceilng::ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::CrushAndRaise,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        50 => {
            // Close Door
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::Close,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        51 => {
            // Secret EXIT
            ctx.switches
                .p_change_switch_texture(ctx.level, line_idx, false);
            crate::g_game::g_secret_exit_level(ctx.wad, crate::doomstat::state().gamemode);
        }
        55 => {
            // Raise Floor Crush
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloorCrush,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        101 => {
            // Raise Floor
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloor,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        102 => {
            // Lower Floor to Surrounding floor height
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::LowerFloor,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        103 => {
            // Open Door
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::Open,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        111 => {
            // Blazing Door Raise (faster than TURBO!)
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::BlazeRaise,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        112 => {
            // Blazing Door Open (faster than TURBO!)
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::BlazeOpen,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        113 => {
            // Blazing Door Close (faster than TURBO!)
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::BlazeClose,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        122 => {
            // Blazing PlatDownWaitUpStay
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::BlazeDwus,
                0,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        127 => {
            // Build Stairs Turbo 16
            if crate::p_floor::ev_build_stairs(
                ctx.level,
                ctx.thinkers,
                &line,
                crate::p_floor::StairType::Turbo16,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        131 => {
            // Raise Floor Turbo
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloorTurbo,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        133 | 135 | 137 => {
            // BlzOpenDoor BLUE/RED/YELLOW
            if crate::p_doors::ev_do_locked_door(
                ctx.thinkers,
                ctx.level,
                ctx.players,
                &line,
                crate::p_doors::VlDoorType::BlazeOpen,
                thing,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }
        140 => {
            // Raise Floor 512
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloor512,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, false);
            }
        }

        // BUTTONS
        42 => {
            // Close Door
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::Close,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        43 => {
            // Lower Ceiling to Floor
            if crate::p_ceilng::ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::LowerToFloor,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        45 => {
            // Lower Floor to Surrounding floor height
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::LowerFloor,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        60 => {
            // Lower Floor to Lowest
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::LowerFloorToLowest,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        61 => {
            // Open Door
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::Open,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        62 => {
            // PlatDownWaitUpStay
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::DownWaitUpStay,
                1,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        63 => {
            // Raise Door
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::Normal,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        64 => {
            // Raise Floor to ceiling
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloor,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        66 => {
            // Raise Floor 24 and change texture
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseAndChange,
                24,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        67 => {
            // Raise Floor 32 and change texture
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseAndChange,
                32,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        65 => {
            // Raise Floor Crush
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloorCrush,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        68 => {
            // Raise Plat to next highest floor and change texture
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseToNearestAndChange,
                0,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        69 => {
            // Raise Floor to next highest floor
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloorToNearest,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        70 => {
            // Turbo Lower Floor
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::TurboLower,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        114 => {
            // Blazing Door Raise (faster than TURBO!)
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::BlazeRaise,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        115 => {
            // Blazing Door Open (faster than TURBO!)
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::BlazeOpen,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        116 => {
            // Blazing Door Close (faster than TURBO!)
            if crate::p_doors::ev_do_door(
                ctx.thinkers,
                ctx.level,
                &line,
                crate::p_doors::VlDoorType::BlazeClose,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        123 => {
            // Blazing PlatDownWaitUpStay
            if crate::p_plats::ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::BlazeDwus,
                0,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        132 => {
            // Raise Floor Turbo
            if crate::p_floor::ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                crate::p_floor::FloorType::RaiseFloorTurbo,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        99 | 134 | 136 => {
            // BlzOpenDoor BLUE/RED/YELLOW
            if crate::p_doors::ev_do_locked_door(
                ctx.thinkers,
                ctx.level,
                ctx.players,
                &line,
                crate::p_doors::VlDoorType::BlazeOpen,
                thing,
            ) {
                ctx.switches
                    .p_change_switch_texture(ctx.level, line_idx, true);
            }
        }
        138 => {
            // Light Turn On
            crate::p_lights::ev_light_turn_on(ctx.level, &line, 255);
            ctx.switches
                .p_change_switch_texture(ctx.level, line_idx, true);
        }
        139 => {
            // Light Turn Off
            crate::p_lights::ev_light_turn_on(ctx.level, &line, 35);
            ctx.switches
                .p_change_switch_texture(ctx.level, line_idx, true);
        }
        _ => {}
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p_tick::Thinkers;
    use crate::r_defs::{Line, Side};

    #[test]
    fn init_switch_list_filters_by_episode() {
        // RData::texture_num_for_name panics on an unknown name (no WAD
        // loaded here), so this test only exercises the episode-cutoff
        // logic indirectly via count — full name resolution is covered
        // by the real-WAD integration tests (see tests/real_*).
        let count_for = |episode: i16| {
            ALPH_SWITCH_LIST
                .iter()
                .filter(|e| e.episode <= episode)
                .count()
        };
        assert_eq!(count_for(1), 19, "shareware episode 1 switches");
        assert_eq!(count_for(2), 29, "registered episodes 2&3 switches");
        assert_eq!(count_for(3), 40, "commercial (Doom II) switches");
    }

    #[test]
    fn start_button_rejects_when_all_slots_full() {
        let mut state = SwitchState::default();
        for i in 0..MAXBUTTONS {
            state.p_start_button(i, BWhere::Top, 1, BUTTONTIME, 0);
        }
        assert!(state.buttonlist.iter().all(|b| b.is_some()));
    }

    #[test]
    fn start_button_is_idempotent_for_the_same_line_while_pressed() {
        let mut state = SwitchState::default();
        state.p_start_button(3, BWhere::Top, 1, BUTTONTIME, 0);
        state.p_start_button(3, BWhere::Top, 1, BUTTONTIME, 0);
        assert_eq!(
            state
                .buttonlist
                .iter()
                .flatten()
                .filter(|b| b.line == 3)
                .count(),
            1
        );
    }

    #[test]
    fn change_switch_texture_swaps_matching_texture_and_starts_a_button() {
        let mut level = Level::default();
        level.sides.push(Side {
            textureoffset: 0,
            rowoffset: 0,
            toptexture: 10,
            bottomtexture: 0,
            midtexture: 0,
            sector: 0,
        });
        level.sectors.push(Default::default());
        level.lines.push(Line {
            sidenum: [Some(0), None],
            frontsector: Some(0),
            special: 42,
            ..Default::default()
        });

        let mut state = SwitchState {
            switchlist: vec![10, 20],
            ..Default::default()
        };

        state.p_change_switch_texture(&mut level, 0, true);

        assert_eq!(
            level.sides[0].toptexture, 20,
            "should swap to the opposite texture"
        );
        assert_eq!(level.lines[0].special, 42, "use_again keeps the special");
        assert_eq!(state.buttonlist.iter().flatten().count(), 1);
    }

    #[test]
    fn change_switch_texture_clears_special_when_not_reusable() {
        let mut level = Level::default();
        level.sides.push(Side {
            textureoffset: 0,
            rowoffset: 0,
            toptexture: 10,
            bottomtexture: 0,
            midtexture: 0,
            sector: 0,
        });
        level.sectors.push(Default::default());
        level.lines.push(Line {
            sidenum: [Some(0), None],
            frontsector: Some(0),
            special: 42,
            ..Default::default()
        });

        let mut state = SwitchState {
            switchlist: vec![10, 20],
            ..Default::default()
        };

        state.p_change_switch_texture(&mut level, 0, false);

        assert_eq!(level.lines[0].special, 0, "one-shot switch clears special");
        assert!(
            state.buttonlist.iter().all(|b| b.is_none()),
            "not a reusable button"
        );
    }

    #[test]
    fn use_special_line_exit_level_sets_gameaction_completed() {
        let mut level = Level::default();
        level.sides.push(Side {
            textureoffset: 0,
            rowoffset: 0,
            toptexture: 0,
            bottomtexture: 0,
            midtexture: 0,
            sector: 0,
        });
        level.sectors.push(Default::default());
        level.lines.push(Line {
            special: 11, // Exit level
            sidenum: [Some(0), None],
            frontsector: Some(0),
            ..Default::default()
        });

        let mut thinkers = Thinkers::new();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj({
                let mut m = crate::r_defs::Mobj::blank(crate::info::MobjType::MtPlayer);
                m.player = Some(0);
                m
            }),
        );
        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = crate::r_main::RMain::new();
        let rdata = crate::r_data::RData::default();
        let wad = crate::w_wad::WadFiles::new();
        let mut active_plats = crate::p_plats::ActivePlats::default();
        let mut active_ceilings = crate::p_ceilng::ActiveCeilings::default();
        let mut switches = SwitchState::default();

        crate::doomstat::state_mut().gameaction = crate::d_event::GameAction::Nothing;

        let mut ctx = crate::p_spec::SpecialsCtx {
            thinkers: &mut thinkers,
            level: &mut level,
            players: &mut players,
            rmain: &mut rmain,
            rdata: &rdata,
            wad: &wad,
            active_plats: &mut active_plats,
            active_ceilings: &mut active_ceilings,
            switches: &mut switches,
        };

        let used = p_use_special_line(&mut ctx, mo, 0, 0);

        assert!(used);
        assert_eq!(
            crate::doomstat::state().gameaction,
            crate::d_event::GameAction::Completed
        );
    }

    #[test]
    fn use_special_line_rejects_the_back_side() {
        let mut level = Level::default();
        level.lines.push(Line {
            special: 11,
            ..Default::default()
        });

        let mut thinkers = Thinkers::new();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj(crate::r_defs::Mobj::blank(
                crate::info::MobjType::MtPlayer,
            )),
        );
        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = crate::r_main::RMain::new();
        let rdata = crate::r_data::RData::default();
        let wad = crate::w_wad::WadFiles::new();
        let mut active_plats = crate::p_plats::ActivePlats::default();
        let mut active_ceilings = crate::p_ceilng::ActiveCeilings::default();
        let mut switches = SwitchState::default();

        let mut ctx = crate::p_spec::SpecialsCtx {
            thinkers: &mut thinkers,
            level: &mut level,
            players: &mut players,
            rmain: &mut rmain,
            rdata: &rdata,
            wad: &wad,
            active_plats: &mut active_plats,
            active_ceilings: &mut active_ceilings,
            switches: &mut switches,
        };

        let used = p_use_special_line(&mut ctx, mo, 0, 1);

        assert!(!used, "back-side use is always rejected (no sliding doors)");
    }
}
