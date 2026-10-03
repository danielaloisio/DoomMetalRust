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
//	Handle Sector base lighting effects.
//	Muzzle flash?
//
//-----------------------------------------------------------------------------

//! Rust port of `p_lights.c`. Handle sector base lighting
//! effects. Muzzle flash? (the original's own description, preserved).
//!
//! # Representation
//!
//! Each of the four thinker structs (`fireflicker_t`/`lightflash_t`/
//! `strobe_t`/`glow_t`) becomes a plain struct ([`FireFlicker`]/
//! [`LightFlash`]/[`Strobe`]/[`Glow`]) holding just its own fields — no
//! embedded `thinker_t` header, since [`crate::p_tick::Thinkers`] already
//! supplies list linkage via [`crate::p_tick::ThinkerId`]/[`crate::p_tick::ThinkFn`]
//! (see that module's docs). `sector_t*` becomes a `usize` index into
//! [`crate::p_setup::Level::sectors`], same convention as every other
//! ported map struct.
//!
//! Not ported: `S_StartSound` calls — none exist in this file (lights
//! are silent).

use crate::m_random::p_random;
use crate::p_setup::Level;
use crate::p_spec::p_find_min_surrounding_light;
use crate::p_tick::{ThinkFn, ThinkerData, Thinkers};

pub const GLOWSPEED: i32 = 8;
pub const STROBEBRIGHT: i32 = 5;
pub const FASTDARK: i32 = 15;
pub const SLOWDARK: i32 = 35;

/// (`fireflicker_t`)
#[derive(Debug, Clone, Copy)]
pub struct FireFlicker {
    pub sector: usize,
    pub count: i32,
    pub maxlight: i32,
    pub minlight: i32,
}

/// (`lightflash_t`)
#[derive(Debug, Clone, Copy)]
pub struct LightFlash {
    pub sector: usize,
    pub count: i32,
    pub maxlight: i32,
    pub minlight: i32,
    pub maxtime: i32,
    pub mintime: i32,
}

/// (`strobe_t`)
#[derive(Debug, Clone, Copy)]
pub struct Strobe {
    pub sector: usize,
    pub count: i32,
    pub minlight: i32,
    pub maxlight: i32,
    pub darktime: i32,
    pub brighttime: i32,
}

/// (`glow_t`)
#[derive(Debug, Clone, Copy)]
pub struct Glow {
    pub sector: usize,
    pub minlight: i32,
    pub maxlight: i32,
    pub direction: i32,
}

/// Port of `T_FireFlicker`.
pub fn t_fire_flicker(thinkers: &mut Thinkers, level: &mut Level, id: crate::p_tick::ThinkerId) {
    let flick = thinkers.fire_flicker_mut(id).unwrap();
    flick.count -= 1;
    if flick.count != 0 {
        return;
    }

    let amount = (p_random() & 3) * 16;
    let sector = &mut level.sectors[flick.sector];
    if sector.lightlevel as i32 - amount < flick.minlight {
        sector.lightlevel = flick.minlight as i16;
    } else {
        sector.lightlevel = (flick.maxlight - amount) as i16;
    }

    flick.count = 4;
}

/// Port of `P_SpawnFireFlicker`.
pub fn p_spawn_fire_flicker(thinkers: &mut Thinkers, level: &mut Level, sector: usize) {
    // Note that we are resetting sector attributes. Nothing special
    // about it during gameplay (the original's own comment, preserved).
    level.sectors[sector].special = 0;

    let lightlevel = level.sectors[sector].lightlevel as i32;
    let minlight = p_find_min_surrounding_light(level, sector, lightlevel) + 16;

    let flick = FireFlicker {
        sector,
        maxlight: lightlevel,
        minlight,
        count: 4,
    };
    thinkers.add_thinker(ThinkFn::FireFlicker, ThinkerData::FireFlicker(flick));
}

/// Port of `T_LightFlash`. Do flashing lights (the original's own
/// comment, preserved).
pub fn t_light_flash(thinkers: &mut Thinkers, level: &mut Level, id: crate::p_tick::ThinkerId) {
    let flash = thinkers.light_flash_mut(id).unwrap();
    flash.count -= 1;
    if flash.count != 0 {
        return;
    }

    let sector = &mut level.sectors[flash.sector];
    if sector.lightlevel as i32 == flash.maxlight {
        sector.lightlevel = flash.minlight as i16;
        flash.count = (p_random() & flash.mintime) + 1;
    } else {
        sector.lightlevel = flash.maxlight as i16;
        flash.count = (p_random() & flash.maxtime) + 1;
    }
}

/// Port of `P_SpawnLightFlash`. After the map has been loaded, scan each
/// sector for specials that spawn thinkers (the original's own comment,
/// preserved).
pub fn p_spawn_light_flash(thinkers: &mut Thinkers, level: &mut Level, sector: usize) {
    // nothing special about it during gameplay (the original's own
    // comment, preserved).
    level.sectors[sector].special = 0;

    let maxlight = level.sectors[sector].lightlevel as i32;
    let minlight = p_find_min_surrounding_light(level, sector, maxlight);
    let maxtime = 64;
    let flash = LightFlash {
        sector,
        maxlight,
        minlight,
        maxtime,
        mintime: 7,
        count: (p_random() & maxtime) + 1,
    };
    thinkers.add_thinker(ThinkFn::LightFlash, ThinkerData::LightFlash(flash));
}

/// Port of `T_StrobeFlash`.
pub fn t_strobe_flash(thinkers: &mut Thinkers, level: &mut Level, id: crate::p_tick::ThinkerId) {
    let flash = thinkers.strobe_mut(id).unwrap();
    flash.count -= 1;
    if flash.count != 0 {
        return;
    }

    let sector = &mut level.sectors[flash.sector];
    if sector.lightlevel as i32 == flash.minlight {
        sector.lightlevel = flash.maxlight as i16;
        flash.count = flash.brighttime;
    } else {
        sector.lightlevel = flash.minlight as i16;
        flash.count = flash.darktime;
    }
}

/// Port of `P_SpawnStrobeFlash`. After the map has been loaded, scan
/// each sector for specials that spawn thinkers (the original's own
/// comment, preserved).
pub fn p_spawn_strobe_flash(
    thinkers: &mut Thinkers,
    level: &mut Level,
    sector: usize,
    fast_or_slow: i32,
    in_sync: bool,
) {
    let maxlight = level.sectors[sector].lightlevel as i32;
    let mut minlight = p_find_min_surrounding_light(level, sector, maxlight);
    if minlight == maxlight {
        minlight = 0;
    }

    // nothing special about it during gameplay (the original's own
    // comment, preserved).
    level.sectors[sector].special = 0;

    let count = if !in_sync { (p_random() & 7) + 1 } else { 1 };

    let flash = Strobe {
        sector,
        darktime: fast_or_slow,
        brighttime: STROBEBRIGHT,
        maxlight,
        minlight,
        count,
    };
    thinkers.add_thinker(ThinkFn::StrobeFlash, ThinkerData::StrobeFlash(flash));
}

/// Port of `EV_StartLightStrobing`. Start strobing lights (usually from
/// a trigger) (the original's own comment, preserved).
pub fn ev_start_light_strobing(
    thinkers: &mut Thinkers,
    level: &mut Level,
    line: &crate::r_defs::Line,
) {
    let mut secnum = -1;
    loop {
        secnum = crate::p_spec::p_find_sector_from_line_tag(level, line, secnum);
        if secnum < 0 {
            break;
        }
        let sec = secnum as usize;
        if level.sectors[sec].specialdata.is_some() {
            continue;
        }
        p_spawn_strobe_flash(thinkers, level, sec, SLOWDARK, false);
    }
}

/// Port of `EV_TurnTagLightsOff`. TURN LINE'S TAG LIGHTS OFF (the
/// original's own comment, preserved).
pub fn ev_turn_tag_lights_off(level: &mut Level, line: &crate::r_defs::Line) {
    for j in 0..level.sectors.len() {
        if level.sectors[j].tag != line.tag {
            continue;
        }
        let mut min = level.sectors[j].lightlevel as i32;
        for i in 0..level.sectors[j].lines.len() {
            let line_idx = level.sectors[j].lines[i];
            if let Some(tsec) = crate::p_spec::get_next_sector(&level.lines[line_idx], j) {
                let light = level.sectors[tsec].lightlevel as i32;
                if light < min {
                    min = light;
                }
            }
        }
        level.sectors[j].lightlevel = min as i16;
    }
}

/// Port of `EV_LightTurnOn`. TURN LINE'S TAG LIGHTS ON (the original's
/// own comment, preserved).
pub fn ev_light_turn_on(level: &mut Level, line: &crate::r_defs::Line, bright: i32) {
    for i in 0..level.sectors.len() {
        if level.sectors[i].tag != line.tag {
            continue;
        }

        // bright = 0 means to search for highest light level
        // surrounding sector (the original's own comment, preserved).
        let mut bright = bright;
        if bright == 0 {
            for j in 0..level.sectors[i].lines.len() {
                let line_idx = level.sectors[i].lines[j];
                if let Some(temp) = crate::p_spec::get_next_sector(&level.lines[line_idx], i) {
                    let light = level.sectors[temp].lightlevel as i32;
                    if light > bright {
                        bright = light;
                    }
                }
            }
        }
        level.sectors[i].lightlevel = bright as i16;
    }
}

/// Port of `T_Glow`.
pub fn t_glow(thinkers: &mut Thinkers, level: &mut Level, id: crate::p_tick::ThinkerId) {
    let g = thinkers.glow_mut(id).unwrap();
    let sector = &mut level.sectors[g.sector];
    match g.direction {
        -1 => {
            // DOWN
            sector.lightlevel -= GLOWSPEED as i16;
            if sector.lightlevel as i32 <= g.minlight {
                sector.lightlevel += GLOWSPEED as i16;
                g.direction = 1;
            }
        }
        1 => {
            // UP
            sector.lightlevel += GLOWSPEED as i16;
            if sector.lightlevel as i32 >= g.maxlight {
                sector.lightlevel -= GLOWSPEED as i16;
                g.direction = -1;
            }
        }
        _ => {}
    }
}

/// Port of `P_SpawnGlowingLight`.
pub fn p_spawn_glowing_light(thinkers: &mut Thinkers, level: &mut Level, sector: usize) {
    let maxlight = level.sectors[sector].lightlevel as i32;
    let minlight = p_find_min_surrounding_light(level, sector, maxlight);

    let g = Glow {
        sector,
        minlight,
        maxlight,
        direction: -1,
    };
    thinkers.add_thinker(ThinkFn::Glow, ThinkerData::Glow(g));

    level.sectors[sector].special = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p_setup::Level;
    use crate::r_defs::Sector;

    fn level_with_one_sector(lightlevel: i16) -> (Level, usize) {
        let mut level = Level::default();
        level.sectors.push(Sector {
            lightlevel,
            ..Default::default()
        });
        (level, 0)
    }

    #[test]
    fn fire_flicker_only_changes_light_when_count_hits_zero() {
        let (mut level, sector) = level_with_one_sector(200);
        let mut thinkers = Thinkers::new();
        let id = thinkers.add_thinker(
            ThinkFn::FireFlicker,
            ThinkerData::FireFlicker(FireFlicker {
                sector,
                count: 2,
                maxlight: 200,
                minlight: 100,
            }),
        );

        t_fire_flicker(&mut thinkers, &mut level, id);
        assert_eq!(
            level.sectors[sector].lightlevel, 200,
            "count hasn't hit zero yet, light must be untouched"
        );

        t_fire_flicker(&mut thinkers, &mut level, id);
        assert!(
            level.sectors[sector].lightlevel <= 200 && level.sectors[sector].lightlevel >= 100,
            "count hit zero, light should have been set to maxlight-amount clamped to minlight"
        );
        assert_eq!(
            thinkers.fire_flicker(id).unwrap().count,
            4,
            "count resets to 4"
        );
    }

    #[test]
    fn spawn_fire_flicker_resets_sector_special() {
        let (mut level, sector) = level_with_one_sector(200);
        level.sectors[sector].special = 17;
        let mut thinkers = Thinkers::new();
        p_spawn_fire_flicker(&mut thinkers, &mut level, sector);
        assert_eq!(level.sectors[sector].special, 0);
    }

    #[test]
    fn light_flash_toggles_between_min_and_max() {
        let (mut level, sector) = level_with_one_sector(200);
        let mut thinkers = Thinkers::new();
        p_spawn_light_flash(&mut thinkers, &mut level, sector);

        // Force count to 1 so the very next tick triggers a toggle.
        let id = {
            let mut found = None;
            thinkers.run_thinkers_dispatch(|thinkers, id, function| {
                if function == ThinkFn::LightFlash {
                    thinkers.light_flash_mut(id).unwrap().count = 1;
                    found = Some(id);
                }
            });
            found.unwrap()
        };

        t_light_flash(&mut thinkers, &mut level, id);
        assert_eq!(
            level.sectors[sector].lightlevel,
            thinkers.light_flash(id).unwrap().minlight as i16
        );
    }

    #[test]
    fn glow_reverses_direction_at_minlight() {
        let (mut level, sector) = level_with_one_sector(0);
        let mut thinkers = Thinkers::new();
        let g = Glow {
            sector,
            minlight: 0,
            maxlight: 200,
            direction: -1,
        };
        let id = thinkers.add_thinker(ThinkFn::Glow, ThinkerData::Glow(g));

        t_glow(&mut thinkers, &mut level, id);

        assert_eq!(
            thinkers.glow(id).unwrap().direction,
            1,
            "should reverse at minlight"
        );
        assert_eq!(level.sectors[sector].lightlevel, 0);
    }

    #[test]
    fn ev_turn_tag_lights_off_takes_dimmest_neighbor() {
        let mut level = Level::default();
        level.sectors.push(Sector {
            tag: 5,
            lightlevel: 200,
            lines: vec![0],
            ..Default::default()
        });
        level.sectors.push(Sector {
            lightlevel: 40,
            ..Default::default()
        });
        level.lines.push(crate::r_defs::Line {
            flags: crate::doomdata::ML_TWOSIDED,
            tag: 5,
            frontsector: Some(0),
            backsector: Some(1),
            ..Default::default()
        });

        let line = level.lines[0];
        ev_turn_tag_lights_off(&mut level, &line);
        assert_eq!(level.sectors[0].lightlevel, 40);
    }
}
