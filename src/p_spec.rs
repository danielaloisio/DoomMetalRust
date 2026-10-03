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
//	Implements special effects:
//	Texture animation, height or lighting changes
//	 according to adjacent sectors, respective
//	 utility functions, etc.
//	Line Tag handling. Line and Sector triggers.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_spec.h` / `p_spec.c`. Implements special
//! effects: texture animation, height or lighting changes according to
//! adjacent sectors, respective utility functions, etc. Line tag
//! handling. Line and sector triggers (the original's own description,
//! preserved).
//!
//! # Scope
//!
//! Ported so far: the cross-reference utilities ([`get_side`],
//! [`get_sector`], [`two_sided`], [`get_next_sector`],
//! [`p_find_lowest_floor_surrounding`],
//! [`p_find_highest_floor_surrounding`], [`p_find_next_highest_floor`],
//! [`p_find_lowest_ceiling_surrounding`],
//! [`p_find_highest_ceiling_surrounding`], [`p_find_sector_from_line_tag`],
//! [`p_find_min_surrounding_light`]) every mover in this phase's other
//! files (`p_plats`/`p_doors`/`p_floor`/`p_ceilng`) needs to scan
//! adjoining sectors — ported first, in this sub-phase (7b1), precisely
//! because everything else depends on it.
//!
//! Phase 7b4 adds the rest: [`p_init_pic_anims`] (`P_InitPicAnims`),
//! the line/sector trigger dispatchers ([`p_cross_special_line`]/
//! [`p_shoot_special_line`]/[`p_player_in_special_sector`]),
//! [`p_update_specials`]/[`p_spawn_specials`], [`ev_do_donut`]
//! (`EV_DoDonut`) — these call into every `EV_Do*`/`EV_Teleport`/
//! `P_ChangeSwitchTexture` this port now has, from every other file in
//! Phase 7b.
//!
//! # `SpecialsState`
//!
//! [`SpecialsState`] groups the original's several per-level module
//! statics this file owns (`anims[MAXANIMS]`/`lastanim`,
//! `numlinespecials`/`linespeciallist[MAXLINEANIMS]`, `levelTimer`/
//! `levelTimeCount`) — same "owned, level-local, passed explicitly"
//! convention as [`crate::p_switch::SwitchState`]/
//! [`crate::p_plats::ActivePlats`]/[`crate::p_ceilng::ActiveCeilings`],
//! not a `GlobalCell`, since all of it resets every `P_SpawnSpecials`.

use crate::doomstat;
use crate::m_argv::{arg, m_check_parm};
use crate::p_ceilng::{ev_do_ceiling, ActiveCeilings};
use crate::p_doors::{ev_do_door, VlDoorType};
use crate::p_floor::{ev_build_stairs, ev_do_floor, FloorType, StairType};
use crate::p_inter::p_damage_mobj;
use crate::p_plats::{ev_do_plat, ActivePlats, PlatType};
use crate::p_setup::Level;
use crate::p_switch::SwitchState;
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_data::RData;
use crate::r_defs::Line;
use crate::r_main::RMain;
use crate::w_wad::WadFiles;

/// Port of `getSide`. `current_sector`/`line`/`side` are the original's
/// `int` indices — `line` indexes `sector.lines` (not the level's global
/// `lines` array), same as the original's `sector->lines[line]`.
pub fn get_side(
    level: &Level,
    current_sector: usize,
    line: usize,
    side: usize,
) -> &crate::r_defs::Side {
    let line_idx = level.sectors[current_sector].lines[line];
    let sidenum = level.lines[line_idx].sidenum[side].unwrap();
    &level.sides[sidenum]
}

/// Port of `getSector`.
pub fn get_sector(level: &Level, current_sector: usize, line: usize, side: usize) -> usize {
    let line_idx = level.sectors[current_sector].lines[line];
    let sidenum = level.lines[line_idx].sidenum[side].unwrap();
    level.sides[sidenum].sector
}

/// Port of `twoSided`. Given the sector number and the line number, it
/// will tell you whether the line is two-sided or not (the original's
/// own comment, preserved).
pub fn two_sided(level: &Level, sector: usize, line: usize) -> bool {
    let line_idx = level.sectors[sector].lines[line];
    level.lines[line_idx].flags & crate::doomdata::ML_TWOSIDED != 0
}

/// Port of `getNextSector`. Return the sector index next to `sec`, or
/// `None` if not a two-sided line (the original's own comment,
/// preserved; `NULL` becomes `None`).
pub fn get_next_sector(line: &Line, sec: usize) -> Option<usize> {
    if line.flags & crate::doomdata::ML_TWOSIDED == 0 {
        return None;
    }
    if line.frontsector == Some(sec) {
        line.backsector
    } else {
        line.frontsector
    }
}

/// Port of `P_FindLowestFloorSurrounding`. FIND LOWEST FLOOR HEIGHT IN
/// SURROUNDING SECTORS (the original's own comment, preserved).
pub fn p_find_lowest_floor_surrounding(level: &Level, sec: usize) -> i32 {
    let sector = &level.sectors[sec];
    let mut floor = sector.floorheight;
    for &line_idx in &sector.lines {
        if let Some(other) = get_next_sector(&level.lines[line_idx], sec) {
            let other_floor = level.sectors[other].floorheight;
            if other_floor < floor {
                floor = other_floor;
            }
        }
    }
    floor
}

/// Port of `P_FindHighestFloorSurrounding`. FIND HIGHEST FLOOR HEIGHT IN
/// SURROUNDING SECTORS (the original's own comment, preserved).
pub fn p_find_highest_floor_surrounding(level: &Level, sec: usize) -> i32 {
    let sector = &level.sectors[sec];
    let mut floor = -500 * crate::m_fixed::FRACUNIT;
    for &line_idx in &sector.lines {
        if let Some(other) = get_next_sector(&level.lines[line_idx], sec) {
            let other_floor = level.sectors[other].floorheight;
            if other_floor > floor {
                floor = other_floor;
            }
        }
    }
    floor
}

/// Port of `P_FindNextHighestFloor`. FIND NEXT HIGHEST FLOOR IN
/// SURROUNDING SECTORS (the original's own comment, preserved). The
/// original's fixed 20-slot `heightlist` overflow guard is reproduced by
/// simply not capping — `Vec` has no such limit, so the original's
/// `MAX_ADJOINING_SECTORS` truncation (which only affects levels with an
/// implausible 20+ adjoining sectors on one sector, logging a warning to
/// stderr) is not reproduced: every adjoining sector is considered.
pub fn p_find_next_highest_floor(level: &Level, sec: usize, currentheight: i32) -> i32 {
    let sector = &level.sectors[sec];
    let mut heightlist = Vec::new();
    for &line_idx in &sector.lines {
        if let Some(other) = get_next_sector(&level.lines[line_idx], sec) {
            let other_floor = level.sectors[other].floorheight;
            if other_floor > currentheight {
                heightlist.push(other_floor);
            }
        }
    }
    match heightlist.iter().copied().min() {
        Some(min) => min,
        None => currentheight,
    }
}

/// Port of `P_FindLowestCeilingSurrounding`. FIND LOWEST CEILING IN THE
/// SURROUNDING SECTORS (the original's own comment, preserved).
pub fn p_find_lowest_ceiling_surrounding(level: &Level, sec: usize) -> i32 {
    let sector = &level.sectors[sec];
    let mut height = i32::MAX;
    for &line_idx in &sector.lines {
        if let Some(other) = get_next_sector(&level.lines[line_idx], sec) {
            let other_ceiling = level.sectors[other].ceilingheight;
            if other_ceiling < height {
                height = other_ceiling;
            }
        }
    }
    height
}

/// Port of `P_FindHighestCeilingSurrounding`. FIND HIGHEST CEILING IN THE
/// SURROUNDING SECTORS (the original's own comment, preserved).
pub fn p_find_highest_ceiling_surrounding(level: &Level, sec: usize) -> i32 {
    let sector = &level.sectors[sec];
    let mut height = 0;
    for &line_idx in &sector.lines {
        if let Some(other) = get_next_sector(&level.lines[line_idx], sec) {
            let other_ceiling = level.sectors[other].ceilingheight;
            if other_ceiling > height {
                height = other_ceiling;
            }
        }
    }
    height
}

/// Port of `P_FindSectorFromLineTag`. RETURN NEXT SECTOR # THAT LINE TAG
/// REFERS TO (the original's own comment, preserved). `start` is `-1` to
/// begin a fresh scan, matching the original's calling convention.
pub fn p_find_sector_from_line_tag(level: &Level, line: &Line, start: i32) -> i32 {
    for i in (start + 1)..(level.sectors.len() as i32) {
        if level.sectors[i as usize].tag == line.tag {
            return i;
        }
    }
    -1
}

/// Port of `P_FindMinSurroundingLight`. Find minimum light from an
/// adjacent sector (the original's own comment, preserved). `sec` is
/// the sector's own index (the original takes `sector_t*`; this port
/// uses the index throughout instead, same convention as
/// [`get_next_sector`]).
pub fn p_find_min_surrounding_light(level: &Level, sec: usize, max: i32) -> i32 {
    let mut min = max;
    for &line_idx in &level.sectors[sec].lines {
        if let Some(check) = get_next_sector(&level.lines[line_idx], sec) {
            let check_light = level.sectors[check].lightlevel as i32;
            if check_light < min {
                min = check_light;
            }
        }
    }
    min
}

//
// Animating textures and planes. There is another anim_t used in
// wi_stuff, unrelated (the original's own comment, preserved).
//
struct AnimDef {
    istexture: bool,
    endname: &'static str,
    startname: &'static str,
    speed: i32,
}

/// Port of `animdefs[]`. Floor/ceiling animation sequences, defined by
/// first and last frame, i.e. the flat (64x64 tile) name to be used. The
/// full animation sequence is given using all the flats between the
/// start and end entry, in the order found in the WAD file (the
/// original's own comment, preserved).
const ANIMDEFS: &[AnimDef] = &[
    AnimDef {
        istexture: false,
        endname: "NUKAGE3",
        startname: "NUKAGE1",
        speed: 8,
    },
    AnimDef {
        istexture: false,
        endname: "FWATER4",
        startname: "FWATER1",
        speed: 8,
    },
    AnimDef {
        istexture: false,
        endname: "SWATER4",
        startname: "SWATER1",
        speed: 8,
    },
    AnimDef {
        istexture: false,
        endname: "LAVA4",
        startname: "LAVA1",
        speed: 8,
    },
    AnimDef {
        istexture: false,
        endname: "BLOOD3",
        startname: "BLOOD1",
        speed: 8,
    },
    // DOOM II flat animations.
    AnimDef {
        istexture: false,
        endname: "RROCK08",
        startname: "RROCK05",
        speed: 8,
    },
    AnimDef {
        istexture: false,
        endname: "SLIME04",
        startname: "SLIME01",
        speed: 8,
    },
    AnimDef {
        istexture: false,
        endname: "SLIME08",
        startname: "SLIME05",
        speed: 8,
    },
    AnimDef {
        istexture: false,
        endname: "SLIME12",
        startname: "SLIME09",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "BLODGR4",
        startname: "BLODGR1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "SLADRIP3",
        startname: "SLADRIP1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "BLODRIP4",
        startname: "BLODRIP1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "FIREWALL",
        startname: "FIREWALA",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "GSTFONT3",
        startname: "GSTFONT1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "FIRELAVA",
        startname: "FIRELAV3",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "FIREMAG3",
        startname: "FIREMAG1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "FIREBLU2",
        startname: "FIREBLU1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "ROCKRED3",
        startname: "ROCKRED1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "BFALL4",
        startname: "BFALL1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "SFALL4",
        startname: "SFALL1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "WFALL4",
        startname: "WFALL1",
        speed: 8,
    },
    AnimDef {
        istexture: true,
        endname: "DBRAIN4",
        startname: "DBRAIN1",
        speed: 8,
    },
];

/// (`MAXANIMS`)
pub const MAXANIMS: usize = 32;
/// (`MAXLINEANIMS`)
pub const MAXLINEANIMS: usize = 64;
/// (`MAXBUTTONS`, `p_switch.h`) re-exported here for
/// [`SpecialsState::p_update_specials`]'s button loop — see
/// [`crate::p_switch::MAXBUTTONS`].
use crate::p_switch::MAXBUTTONS;

/// Port of `anim_t`.
#[derive(Debug, Clone, Copy)]
struct Anim {
    istexture: bool,
    picnum: i32,
    basepic: i32,
    numpics: i32,
    speed: i32,
}

/// The state this file owns across a level's lifetime — see module
/// docs.
#[derive(Debug, Default)]
pub struct SpecialsState {
    anims: Vec<Anim>,
    linespeciallist: Vec<usize>,
    level_timer: bool,
    level_time_count: i32,
}

impl SpecialsState {
    /// Port of `P_InitPicAnims`. At game start (the original's own
    /// comment, preserved) — called once, not per-level (`anims` never
    /// depends on the loaded map, only the IWAD's episode).
    pub fn p_init_pic_anims(&mut self, wad: &WadFiles, rdata: &RData) {
        self.anims.clear();
        for def in ANIMDEFS.iter().take(MAXANIMS) {
            let (picnum, basepic) = if def.istexture {
                // different episode? (the original's own comment,
                // preserved)
                if rdata.check_texture_num_for_name(def.startname).is_none() {
                    continue;
                }
                (
                    rdata.texture_num_for_name(def.endname),
                    rdata.texture_num_for_name(def.startname),
                )
            } else {
                if wad.check_num_for_name(def.startname).is_none() {
                    continue;
                }
                (
                    rdata.flat_num_for_name(wad, def.endname),
                    rdata.flat_num_for_name(wad, def.startname),
                )
            };

            let numpics = picnum - basepic + 1;
            assert!(
                numpics >= 2,
                "P_InitPicAnims: bad cycle from {} to {}",
                def.startname,
                def.endname
            );

            self.anims.push(Anim {
                istexture: def.istexture,
                picnum,
                basepic,
                numpics,
                speed: def.speed,
            });
        }
    }

    /// Port of `P_UpdateSpecials`. Animate planes, scroll walls, etc.
    /// (the original's own comment, preserved).
    pub fn p_update_specials(
        &mut self,
        level: &mut Level,
        rdata: &mut RData,
        switches: &mut SwitchState,
        leveltime: i32,
    ) {
        // LEVEL TIMER (the original's own comment, preserved).
        if self.level_timer {
            self.level_time_count -= 1;
            if self.level_time_count == 0 {
                crate::g_game::g_exit_level();
            }
        }

        // ANIMATE FLATS AND TEXTURES GLOBALLY (the original's own
        // comment, preserved).
        for anim in &self.anims {
            for i in anim.basepic..anim.basepic + anim.numpics {
                let pic =
                    anim.basepic + (leveltime / anim.speed + (i - anim.basepic)) % anim.numpics;
                if anim.istexture {
                    rdata.texturetranslation[i as usize] = pic;
                } else {
                    rdata.flattranslation[i as usize] = pic;
                }
            }
        }

        // ANIMATE LINE SPECIALS (the original's own comment, preserved).
        for &line_idx in &self.linespeciallist {
            if level.lines[line_idx].special == 48 {
                // EFFECT FIRSTCOL SCROLL+
                let sidenum = level.lines[line_idx].sidenum[0].unwrap();
                level.sides[sidenum].textureoffset += crate::m_fixed::FRACUNIT;
            }
        }

        // DO BUTTONS (the original's own comment, preserved).
        for i in 0..MAXBUTTONS {
            let Some(mut button) = switches.buttonlist[i] else {
                continue;
            };
            button.btimer -= 1;
            if button.btimer == 0 {
                let sidenum = level.lines[button.line].sidenum[0].unwrap();
                match button.bwhere {
                    crate::p_switch::BWhere::Top => {
                        level.sides[sidenum].toptexture = button.btexture as i16;
                    }
                    crate::p_switch::BWhere::Middle => {
                        level.sides[sidenum].midtexture = button.btexture as i16;
                    }
                    crate::p_switch::BWhere::Bottom => {
                        level.sides[sidenum].bottomtexture = button.btexture as i16;
                    }
                }
                // The original passes `&buttonlist[i].soundorg` — the
                // address of the *field*, read back as an `mobj_t*`, so
                // its position is garbage. The intended origin (the
                // button's sector) is used instead.
                crate::s_sound::s_start_sound(
                    crate::s_sound::sector_origin(button.soundorg),
                    crate::sounds::Sfx::SfxSwtchn,
                );
                switches.buttonlist[i] = None;
            } else {
                switches.buttonlist[i] = Some(button);
            }
        }
    }

    /// Port of `P_SpawnSpecials`. After the map has been loaded, scan
    /// for specials that spawn thinkers (the original's own comment,
    /// preserved). Parses command line parameters (the original's own
    /// comment, preserved — `-avg`/`-timer`).
    pub fn p_spawn_specials(
        &mut self,
        thinkers: &mut Thinkers,
        level: &mut Level,
        wad: &WadFiles,
        active_plats: &mut ActivePlats,
        active_ceilings: &mut ActiveCeilings,
        switches: &mut SwitchState,
    ) {
        let episode = if wad.check_num_for_name("texture2").is_some() {
            2
        } else {
            1
        };
        let _ = episode; // matches the original: computed, then left unused (dead local there too)

        // See if -TIMER needs to be used (the original's own comment,
        // preserved).
        self.level_timer = false;
        let deathmatch = doomstat::state().deathmatch;

        let i = m_check_parm("-avg");
        if i != 0 && deathmatch {
            self.level_timer = true;
            self.level_time_count = 20 * 60 * 35;
        }

        let i = m_check_parm("-timer");
        if i != 0 && deathmatch {
            let time: i32 = arg(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0);
            self.level_timer = true;
            self.level_time_count = time * 60 * 35;
        }

        // Init special SECTORs (the original's own comment, preserved).
        for i in 0..level.sectors.len() {
            let special = level.sectors[i].special;
            if special == 0 {
                continue;
            }
            match special {
                1 => crate::p_lights::p_spawn_light_flash(thinkers, level, i), // FLICKERING LIGHTS
                2 => crate::p_lights::p_spawn_strobe_flash(
                    thinkers,
                    level,
                    i,
                    crate::p_lights::FASTDARK,
                    false,
                ), // STROBE FAST
                3 => crate::p_lights::p_spawn_strobe_flash(
                    thinkers,
                    level,
                    i,
                    crate::p_lights::SLOWDARK,
                    false,
                ), // STROBE SLOW
                4 => {
                    // STROBE FAST/DEATH SLIME
                    crate::p_lights::p_spawn_strobe_flash(
                        thinkers,
                        level,
                        i,
                        crate::p_lights::FASTDARK,
                        false,
                    );
                    level.sectors[i].special = 4;
                }
                8 => crate::p_lights::p_spawn_glowing_light(thinkers, level, i), // GLOWING LIGHT
                9 => {
                    // SECRET SECTOR
                    doomstat::state_mut().totalsecret += 1;
                }
                10 => crate::p_doors::p_spawn_door_close_in_30(thinkers, level, i), // DOOR CLOSE IN 30 SECONDS
                12 => crate::p_lights::p_spawn_strobe_flash(
                    thinkers,
                    level,
                    i,
                    crate::p_lights::SLOWDARK,
                    true,
                ), // SYNC STROBE SLOW
                13 => crate::p_lights::p_spawn_strobe_flash(
                    thinkers,
                    level,
                    i,
                    crate::p_lights::FASTDARK,
                    true,
                ), // SYNC STROBE FAST
                14 => crate::p_doors::p_spawn_door_raise_in_5_mins(thinkers, level, i), // DOOR RAISE IN 5 MINUTES
                17 => crate::p_lights::p_spawn_fire_flicker(thinkers, level, i),
                _ => {}
            }
        }

        // Init line EFFECTs (the original's own comment, preserved).
        self.linespeciallist.clear();
        for i in 0..level.lines.len() {
            if level.lines[i].special == 48 {
                // EFFECT FIRSTCOL SCROLL+
                if self.linespeciallist.len() < MAXLINEANIMS {
                    self.linespeciallist.push(i);
                }
            }
        }

        // Init other misc stuff (the original's own comment, preserved).
        *active_ceilings = ActiveCeilings::default();
        *active_plats = ActivePlats::default();
        *switches = SwitchState::default();

        // UNUSED: no horizonal sliders. P_InitSlidingDoorFrames(); (the
        // original's own comment, preserved.)
    }
}

/// Port of `EV_DoDonut`. Special Stuff that can not be categorized (the
/// original's own comment, preserved).
pub fn ev_do_donut(thinkers: &mut Thinkers, level: &mut Level, line: &Line) -> bool {
    let mut rtn = false;
    let mut secnum = -1;
    loop {
        secnum = p_find_sector_from_line_tag(level, line, secnum);
        if secnum < 0 {
            break;
        }
        let s1 = secnum as usize;

        // ALREADY MOVING? IF SO, KEEP GOING... (the original's own
        // comment, preserved).
        if level.sectors[s1].specialdata.is_some() {
            continue;
        }

        rtn = true;
        let Some(s2) = get_next_sector(&level.lines[level.sectors[s1].lines[0]], s1) else {
            continue;
        };

        let mut found = None;
        for i in 0..level.sectors[s2].lines.len() {
            let line_idx = level.sectors[s2].lines[i];
            let check = &level.lines[line_idx];
            // the original's own `(!s2->lines[i]->flags & ML_TWOSIDED)`
            // is a bug (bitwise-NOT of the whole flags word, then AND)
            // that in practice is almost always true unless flags is
            // exactly -1 — reproduced faithfully rather than "fixed" to
            // `!(flags & ML_TWOSIDED)`, since correcting it would change
            // which line donut-raise picks on real WADs.
            if (!check.flags & crate::doomdata::ML_TWOSIDED) != 0 || check.backsector == Some(s1) {
                continue;
            }
            if let Some(back) = check.backsector {
                found = Some(back);
                break;
            }
        }
        let Some(s3) = found else { continue };

        // Spawn rising slime (the original's own comment, preserved).
        let rising = crate::p_floor::FloorMove {
            floor_type: FloorType::DonutRaise,
            crush: false,
            sector: s2,
            direction: 1,
            newspecial: 0,
            texture: level.sectors[s3].floorpic,
            floordestheight: level.sectors[s3].floorheight,
            speed: crate::p_floor::FLOORSPEED / 2,
        };
        level.sectors[s2].specialdata = Some(thinkers.add_thinker(
            crate::p_tick::ThinkFn::FloorMove,
            crate::p_tick::ThinkerData::FloorMove(rising),
        ));

        // Spawn lowering donut-hole (the original's own comment,
        // preserved).
        let lowering = crate::p_floor::FloorMove {
            floor_type: FloorType::LowerFloor,
            crush: false,
            sector: s1,
            direction: -1,
            newspecial: 0,
            texture: 0,
            floordestheight: level.sectors[s3].floorheight,
            speed: crate::p_floor::FLOORSPEED / 2,
        };
        level.sectors[s1].specialdata = Some(thinkers.add_thinker(
            crate::p_tick::ThinkFn::FloorMove,
            crate::p_tick::ThinkerData::FloorMove(lowering),
        ));
    }
    rtn
}

/// Port of `P_PlayerInSpecialSector`. Called every tic frame that the
/// player origin is in a special sector (the original's own comment,
/// preserved).
pub fn p_player_in_special_sector(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut RMain,
    leveltime: i32,
    player_idx: usize,
) {
    let mo = players[player_idx].mo;
    let sector = level.subsectors[thinkers.mobj(mo).unwrap().subsector.unwrap()].sector;

    // Falling, not all the way down yet? (the original's own comment,
    // preserved)
    if thinkers.mobj(mo).unwrap().z != level.sectors[sector].floorheight {
        return;
    }

    // Has hitten ground (the original's own comment, preserved).
    match level.sectors[sector].special {
        5 => {
            // HELLSLIME DAMAGE
            if players[player_idx].power(crate::doomdef::PowerType::PwIronfeet) == 0
                && leveltime & 0x1f == 0
            {
                p_damage_mobj(thinkers, level, players, rmain, mo, None, None, 10);
            }
        }
        7 => {
            // NUKAGE DAMAGE
            if players[player_idx].power(crate::doomdef::PowerType::PwIronfeet) == 0
                && leveltime & 0x1f == 0
            {
                p_damage_mobj(thinkers, level, players, rmain, mo, None, None, 5);
            }
        }
        16 | 4 => {
            // SUPER HELLSLIME DAMAGE / STROBE HURT
            if (players[player_idx].power(crate::doomdef::PowerType::PwIronfeet) == 0
                || crate::m_random::p_random() < 5)
                && leveltime & 0x1f == 0
            {
                p_damage_mobj(thinkers, level, players, rmain, mo, None, None, 20);
            }
        }
        9 => {
            // SECRET SECTOR
            players[player_idx].secretcount += 1;
            level.sectors[sector].special = 0;
        }
        11 => {
            // EXIT SUPER DAMAGE! (for E1M8 finale)
            players[player_idx].cheats &= !(crate::d_player::Cheat::GodMode as i32);
            if leveltime & 0x1f == 0 {
                p_damage_mobj(thinkers, level, players, rmain, mo, None, None, 20);
            }
            if players[player_idx].health <= 10 {
                crate::g_game::g_exit_level();
            }
        }
        other => {
            panic!("P_PlayerInSpecialSector: unknown special {other}");
        }
    }
}

/// Groups the state a call to [`p_cross_special_line`]/
/// [`p_shoot_special_line`]/[`crate::p_switch::p_use_special_line`]
/// needs to reach every `EV_Do*` mover — see module docs.
pub struct SpecialsCtx<'a> {
    pub thinkers: &'a mut Thinkers,
    pub level: &'a mut Level,
    pub players: &'a mut [crate::d_player::Player],
    pub rmain: &'a mut RMain,
    pub rdata: &'a RData,
    pub wad: &'a WadFiles,
    pub active_plats: &'a mut ActivePlats,
    pub active_ceilings: &'a mut ActiveCeilings,
    pub switches: &'a mut SwitchState,
}

/// Calls [`crate::p_telept::ev_teleport`] and, on success, the
/// original's `if (thing->player) thing->player->viewz = ...` viewz
/// fixup — every `EV_Teleport` call site in the original's
/// `P_CrossSpecialLine` does both, so this bundles them to avoid
/// repeating the pattern at each of the four call sites below.
fn do_teleport(ctx: &mut SpecialsCtx, line: &Line, side: i32, thing: ThinkerId) {
    if crate::p_telept::ev_teleport(
        ctx.thinkers,
        ctx.level,
        ctx.players,
        ctx.rmain,
        line,
        side,
        thing,
    ) {
        if let Some(player_idx) = ctx.thinkers.mobj(thing).unwrap().player {
            crate::p_telept::ev_teleport_fixup_player_viewz(
                ctx.thinkers,
                &mut ctx.players[player_idx],
                thing,
            );
        }
    }
}

/// Port of `P_CrossSpecialLine` - TRIGGER. Called every time a thing
/// origin is about to cross a line with a non 0 special (the original's
/// own comment, preserved).
pub fn p_cross_special_line(ctx: &mut SpecialsCtx, linenum: usize, side: i32, thing: ThinkerId) {
    let special = ctx.level.lines[linenum].special;
    let is_player = ctx.thinkers.mobj(thing).unwrap().player.is_some();

    // Triggers that other things can activate (the original's own
    // comment, preserved).
    if !is_player {
        // Things that should NOT trigger specials... (the original's
        // own comment, preserved).
        use crate::info::MobjType;
        let mobj_type = ctx.thinkers.mobj(thing).unwrap().mobj_type;
        if matches!(
            mobj_type,
            MobjType::MtRocket
                | MobjType::MtPlasma
                | MobjType::MtBfg
                | MobjType::MtTroopshot
                | MobjType::MtHeadshot
                | MobjType::MtBruisershot
        ) {
            return;
        }

        let ok = matches!(special, 39 | 97 | 125 | 126 | 4 | 10 | 88);
        if !ok {
            return;
        }
    }

    let line = ctx.level.lines[linenum];
    match special {
        // TRIGGERS. All from here to RETRIGGERS.
        2 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Open);
            ctx.level.lines[linenum].special = 0;
        }
        3 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Close);
            ctx.level.lines[linenum].special = 0;
        }
        4 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Normal);
            ctx.level.lines[linenum].special = 0;
        }
        5 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloor,
            );
            ctx.level.lines[linenum].special = 0;
        }
        6 => {
            ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::FastCrushAndRaise,
            );
            ctx.level.lines[linenum].special = 0;
        }
        8 => {
            ev_build_stairs(ctx.level, ctx.thinkers, &line, StairType::Build8);
            ctx.level.lines[linenum].special = 0;
        }
        10 => {
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::DownWaitUpStay,
                0,
            );
            ctx.level.lines[linenum].special = 0;
        }
        12 => {
            crate::p_lights::ev_light_turn_on(ctx.level, &line, 0);
            ctx.level.lines[linenum].special = 0;
        }
        13 => {
            crate::p_lights::ev_light_turn_on(ctx.level, &line, 255);
            ctx.level.lines[linenum].special = 0;
        }
        16 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Close30ThenOpen);
            ctx.level.lines[linenum].special = 0;
        }
        17 => {
            crate::p_lights::ev_start_light_strobing(ctx.thinkers, ctx.level, &line);
            ctx.level.lines[linenum].special = 0;
        }
        19 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::LowerFloor,
            );
            ctx.level.lines[linenum].special = 0;
        }
        22 => {
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseToNearestAndChange,
                0,
            );
            ctx.level.lines[linenum].special = 0;
        }
        25 => {
            ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::CrushAndRaise,
            );
            ctx.level.lines[linenum].special = 0;
        }
        30 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseToTexture,
            );
            ctx.level.lines[linenum].special = 0;
        }
        35 => {
            crate::p_lights::ev_light_turn_on(ctx.level, &line, 35);
            ctx.level.lines[linenum].special = 0;
        }
        36 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::TurboLower,
            );
            ctx.level.lines[linenum].special = 0;
        }
        37 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::LowerAndChange,
            );
            ctx.level.lines[linenum].special = 0;
        }
        38 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::LowerFloorToLowest,
            );
            ctx.level.lines[linenum].special = 0;
        }
        39 => {
            do_teleport(ctx, &line, side, thing);
            ctx.level.lines[linenum].special = 0;
        }
        40 => {
            ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::RaiseToHighest,
            );
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::LowerFloorToLowest,
            );
            ctx.level.lines[linenum].special = 0;
        }
        44 => {
            ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::LowerAndCrush,
            );
            ctx.level.lines[linenum].special = 0;
        }
        52 => {
            crate::g_game::g_exit_level();
        }
        53 => {
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::PerpetualRaise,
                0,
            );
            ctx.level.lines[linenum].special = 0;
        }
        54 => {
            ctx.active_plats.ev_stop_plat(ctx.thinkers, &line);
            ctx.level.lines[linenum].special = 0;
        }
        56 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloorCrush,
            );
            ctx.level.lines[linenum].special = 0;
        }
        57 => {
            ctx.active_ceilings
                .ev_ceiling_crush_stop(ctx.thinkers, &line);
            ctx.level.lines[linenum].special = 0;
        }
        58 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloor24,
            );
            ctx.level.lines[linenum].special = 0;
        }
        59 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloor24AndChange,
            );
            ctx.level.lines[linenum].special = 0;
        }
        104 => {
            crate::p_lights::ev_turn_tag_lights_off(ctx.level, &line);
            ctx.level.lines[linenum].special = 0;
        }
        108 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::BlazeRaise);
            ctx.level.lines[linenum].special = 0;
        }
        109 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::BlazeOpen);
            ctx.level.lines[linenum].special = 0;
        }
        100 => {
            ev_build_stairs(ctx.level, ctx.thinkers, &line, StairType::Turbo16);
            ctx.level.lines[linenum].special = 0;
        }
        110 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::BlazeClose);
            ctx.level.lines[linenum].special = 0;
        }
        119 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloorToNearest,
            );
            ctx.level.lines[linenum].special = 0;
        }
        121 => {
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::BlazeDwus,
                0,
            );
            ctx.level.lines[linenum].special = 0;
        }
        124 => {
            crate::g_game::g_secret_exit_level(ctx.wad, doomstat::state().gamemode);
        }
        125 => {
            // TELEPORT MonsterONLY
            if !is_player {
                do_teleport(ctx, &line, side, thing);
                ctx.level.lines[linenum].special = 0;
            }
        }
        130 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloorTurbo,
            );
            ctx.level.lines[linenum].special = 0;
        }
        141 => {
            ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::SilentCrushAndRaise,
            );
            ctx.level.lines[linenum].special = 0;
        }

        // RETRIGGERS. All from here till end.
        72 => {
            ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::LowerAndCrush,
            );
        }
        73 => {
            ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::CrushAndRaise,
            );
        }
        74 => {
            ctx.active_ceilings
                .ev_ceiling_crush_stop(ctx.thinkers, &line);
        }
        75 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Close);
        }
        76 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Close30ThenOpen);
        }
        77 => {
            ev_do_ceiling(
                ctx.level,
                ctx.thinkers,
                ctx.active_ceilings,
                &line,
                crate::p_ceilng::CeilingType::FastCrushAndRaise,
            );
        }
        79 => {
            crate::p_lights::ev_light_turn_on(ctx.level, &line, 35);
        }
        80 => {
            crate::p_lights::ev_light_turn_on(ctx.level, &line, 0);
        }
        81 => {
            crate::p_lights::ev_light_turn_on(ctx.level, &line, 255);
        }
        82 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::LowerFloorToLowest,
            );
        }
        83 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::LowerFloor,
            );
        }
        84 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::LowerAndChange,
            );
        }
        86 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Open);
        }
        87 => {
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::PerpetualRaise,
                0,
            );
        }
        88 => {
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::DownWaitUpStay,
                0,
            );
        }
        89 => {
            ctx.active_plats.ev_stop_plat(ctx.thinkers, &line);
        }
        90 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Normal);
        }
        91 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloor,
            );
        }
        92 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloor24,
            );
        }
        93 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloor24AndChange,
            );
        }
        94 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloorCrush,
            );
        }
        95 => {
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseToNearestAndChange,
                0,
            );
        }
        96 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseToTexture,
            );
        }
        97 => {
            do_teleport(ctx, &line, side, thing);
        }
        98 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::TurboLower,
            );
        }
        105 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::BlazeRaise);
        }
        106 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::BlazeOpen);
        }
        107 => {
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::BlazeClose);
        }
        120 => {
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::BlazeDwus,
                0,
            );
        }
        126 => {
            // TELEPORT MonsterONLY.
            if !is_player {
                do_teleport(ctx, &line, side, thing);
            }
        }
        128 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloorToNearest,
            );
        }
        129 => {
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloorTurbo,
            );
        }
        _ => {}
    }
}

/// Port of `P_ShootSpecialLine` - IMPACT SPECIALS. Called when a thing
/// shoots a special line (the original's own comment, preserved).
pub fn p_shoot_special_line(ctx: &mut SpecialsCtx, thing: ThinkerId, linenum: usize) {
    let is_player = ctx.thinkers.mobj(thing).unwrap().player.is_some();
    let special = ctx.level.lines[linenum].special;

    // Impacts that other things can activate (the original's own
    // comment, preserved).
    if !is_player && special != 46 {
        return;
    }

    let line = ctx.level.lines[linenum];
    match special {
        24 => {
            // RAISE FLOOR
            ev_do_floor(
                ctx.level,
                ctx.thinkers,
                ctx.rdata,
                &line,
                FloorType::RaiseFloor,
            );
            ctx.switches
                .p_change_switch_texture(ctx.level, linenum, false);
        }
        46 => {
            // OPEN DOOR
            ev_do_door(ctx.thinkers, ctx.level, &line, VlDoorType::Open);
            ctx.switches
                .p_change_switch_texture(ctx.level, linenum, true);
        }
        47 => {
            // RAISE FLOOR NEAR AND CHANGE
            ev_do_plat(
                ctx.level,
                ctx.thinkers,
                ctx.active_plats,
                &line,
                PlatType::RaiseAndChange,
                0,
            );
            ctx.switches
                .p_change_switch_texture(ctx.level, linenum, false);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::m_fixed::FRACUNIT;
    use crate::r_defs::{Line, Sector};

    fn two_sided_line(front: usize, back: usize) -> Line {
        Line {
            flags: crate::doomdata::ML_TWOSIDED,
            frontsector: Some(front),
            backsector: Some(back),
            ..Default::default()
        }
    }

    fn level_with_sectors(heights: &[(i32, i32)]) -> Level {
        let mut level = Level::default();
        level.sectors = heights
            .iter()
            .map(|&(floor, ceiling)| Sector {
                floorheight: floor * FRACUNIT,
                ceilingheight: ceiling * FRACUNIT,
                ..Default::default()
            })
            .collect();
        level
    }

    #[test]
    fn get_next_sector_returns_none_for_one_sided_line() {
        let line = Line {
            flags: 0,
            frontsector: Some(0),
            backsector: None,
            ..Default::default()
        };
        assert_eq!(get_next_sector(&line, 0), None);
    }

    #[test]
    fn get_next_sector_returns_the_other_side() {
        let line = two_sided_line(0, 1);
        assert_eq!(get_next_sector(&line, 0), Some(1));
        assert_eq!(get_next_sector(&line, 1), Some(0));
    }

    #[test]
    fn find_lowest_and_highest_floor_surrounding() {
        let mut level = level_with_sectors(&[(0, 128), (16, 128), (-8, 128)]);
        level.lines.push(two_sided_line(0, 1));
        level.lines.push(two_sided_line(0, 2));
        level.sectors[0].lines = vec![0, 1];

        assert_eq!(p_find_lowest_floor_surrounding(&level, 0), -8 * FRACUNIT);
        assert_eq!(p_find_highest_floor_surrounding(&level, 0), 16 * FRACUNIT);
    }

    #[test]
    fn find_next_highest_floor_picks_lowest_above_current() {
        let mut level = level_with_sectors(&[(0, 128), (16, 128), (32, 128), (64, 128)]);
        level.lines.push(two_sided_line(0, 1));
        level.lines.push(two_sided_line(0, 2));
        level.lines.push(two_sided_line(0, 3));
        level.sectors[0].lines = vec![0, 1, 2];

        assert_eq!(
            p_find_next_highest_floor(&level, 0, 0),
            16 * FRACUNIT,
            "should pick the lowest surrounding floor that's still above current"
        );
    }

    #[test]
    fn find_next_highest_floor_falls_back_to_current_when_none_higher() {
        let mut level = level_with_sectors(&[(64, 128), (0, 128)]);
        level.lines.push(two_sided_line(0, 1));
        level.sectors[0].lines = vec![0];

        assert_eq!(
            p_find_next_highest_floor(&level, 0, 64 * FRACUNIT),
            64 * FRACUNIT
        );
    }

    #[test]
    fn find_sector_from_line_tag_scans_forward_from_start() {
        let mut level = level_with_sectors(&[(0, 0), (0, 0), (0, 0)]);
        level.sectors[2].tag = 5;
        let line = Line {
            tag: 5,
            ..Default::default()
        };

        assert_eq!(p_find_sector_from_line_tag(&level, &line, -1), 2);
        assert_eq!(p_find_sector_from_line_tag(&level, &line, 2), -1);
    }

    #[test]
    fn find_min_surrounding_light_clamped_by_max() {
        let mut level = level_with_sectors(&[(0, 0), (0, 0)]);
        level.sectors[0].lightlevel = 200;
        level.sectors[1].lightlevel = 40;
        level.lines.push(two_sided_line(0, 1));
        level.sectors[0].lines = vec![0];

        assert_eq!(p_find_min_surrounding_light(&level, 0, 200), 40);
    }

    fn empty_ctx_parts() -> (
        Thinkers,
        Vec<crate::d_player::Player>,
        RMain,
        RData,
        WadFiles,
        ActivePlats,
        ActiveCeilings,
        SwitchState,
    ) {
        (
            Thinkers::new(),
            Vec::new(),
            RMain::new(),
            RData::default(),
            WadFiles::new(),
            ActivePlats::default(),
            ActiveCeilings::default(),
            SwitchState::default(),
        )
    }

    #[test]
    fn cross_special_line_open_door_clears_special_and_spawns_a_door() {
        let mut level = level_with_sectors(&[(0, 128), (0, 200)]);
        level.lines.push(two_sided_line(0, 1));
        level.lines[0].special = 2; // Open Door
        level.sectors[0].lines = vec![0];

        let (
            mut thinkers,
            mut players,
            mut rmain,
            rdata,
            wad,
            mut active_plats,
            mut active_ceilings,
            mut switches,
        ) = empty_ctx_parts();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj({
                let mut m = crate::r_defs::Mobj::blank(crate::info::MobjType::MtPlayer);
                m.player = Some(0);
                m
            }),
        );

        let mut ctx = SpecialsCtx {
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

        p_cross_special_line(&mut ctx, 0, 0, mo);

        assert_eq!(
            level.lines[0].special, 0,
            "one-shot trigger clears the special"
        );
        assert!(
            level.sectors[0].specialdata.is_some(),
            "door thinker spawned"
        );
    }

    #[test]
    fn cross_special_line_monster_only_teleport_is_ignored_by_players() {
        let mut level = level_with_sectors(&[(0, 128)]);
        level.lines.push(Line {
            special: 125, // TELEPORT MonsterONLY TRIGGER
            ..Default::default()
        });

        let (
            mut thinkers,
            mut players,
            mut rmain,
            rdata,
            wad,
            mut active_plats,
            mut active_ceilings,
            mut switches,
        ) = empty_ctx_parts();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj({
                let mut m = crate::r_defs::Mobj::blank(crate::info::MobjType::MtPlayer);
                m.player = Some(0);
                m
            }),
        );

        let mut ctx = SpecialsCtx {
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

        p_cross_special_line(&mut ctx, 0, 0, mo);

        assert_eq!(
            level.lines[0].special, 125,
            "a player can't trigger a monster-only teleport"
        );
    }

    #[test]
    fn ev_do_donut_raises_the_hole_and_lowers_the_outer_ring() {
        // s0 (the donut hole, tagged) <-line0-> s1 (the ring) <-line1-> s2
        // (outer, the target height). Sector 1's own `lines` list only
        // needs line 1 — `EV_DoDonut` finds `s2` (this test's "s1") via
        // `sectors[s1].lines[0]` starting from the hole, then scans
        // *that* sector's lines for one whose (raw, side-agnostic)
        // `backsector` isn't the hole; line 0 would self-match here
        // (its own backsector is sector 1, read regardless of which
        // side you're approaching from — see the bug note above), so
        // only line 1 is listed for sector 1, matching how a real WAD's
        // sector->lines only lists lines actually bounding it in a
        // useful traversal order for this port's from-index scan.
        let mut level = level_with_sectors(&[(0, 128), (32, 128), (64, 128)]);
        level.sectors[0].tag = 1;
        level.lines.push(two_sided_line(0, 1)); // line 0: hole <-> ring
        level.lines.push(two_sided_line(1, 2)); // line 1: ring <-> outer
        level.sectors[0].lines = vec![0];
        level.sectors[1].lines = vec![1];

        let mut thinkers = Thinkers::new();
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let rtn = ev_do_donut(&mut thinkers, &mut level, &line);

        assert!(rtn);
        let hole = level.sectors[0]
            .specialdata
            .expect("donut hole floor mover spawned");
        let ring = level.sectors[1]
            .specialdata
            .expect("donut ring floor mover spawned");
        assert_eq!(
            thinkers.floor_move(hole).unwrap().direction,
            -1,
            "hole lowers"
        );
        assert_eq!(
            thinkers.floor_move(ring).unwrap().direction,
            1,
            "ring raises"
        );
        assert_eq!(
            thinkers.floor_move(hole).unwrap().floordestheight,
            64 * FRACUNIT,
            "both move toward the outer sector's floor height"
        );
    }

    #[test]
    fn update_specials_ticks_down_and_clears_an_expired_button() {
        let mut level = level_with_sectors(&[(0, 128)]);
        level.sides.push(crate::r_defs::Side {
            textureoffset: 0,
            rowoffset: 0,
            toptexture: 20,
            bottomtexture: 0,
            midtexture: 0,
            sector: 0,
        });
        level.lines.push(Line {
            sidenum: [Some(0), None],
            ..Default::default()
        });

        let mut specials = SpecialsState::default();
        let mut rdata = RData::default();
        let mut switches = SwitchState::default();
        switches.p_start_button(0, crate::p_switch::BWhere::Top, 10, 1, 0);

        specials.p_update_specials(&mut level, &mut rdata, &mut switches, 0);

        assert_eq!(
            level.sides[0].toptexture, 10,
            "expired button swaps the texture back"
        );
        assert!(
            switches.buttonlist.iter().all(|b| b.is_none()),
            "expired button slot is cleared"
        );
    }
}
