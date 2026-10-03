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
// DESCRIPTION: Door animation code (opening/closing)
//
//-----------------------------------------------------------------------------

//! Rust port of `p_doors.c`. Door animation code (opening/
//! closing) (the original's own description, preserved).
//!
//! # Scope
//!
//! The `#if 0`'d out sliding-door code (`slidename_t`/`slideframe_t`/
//! `slidedoor_t`/`T_SlidingDoor`/`EV_SlidingDoor`/
//! `P_InitSlidingDoorFrames`/`P_FindSlidingDoorType` — "ABANDONED TO THE
//! MISTS OF TIME!!!", the original's own comment) is not ported, same as
//! the original doesn't compile it.
//!
//! # Representation
//!
//! [`VlDoor`] drops the embedded `thinker_t` header, same as every
//! other mover in this phase. `sector_t*` becomes a `usize` index.
//!
//! Sounds (`S_StartSound`) are queued via [`crate::s_sound`]; the
//! sector's `soundorg` is [`crate::s_sound::sector_origin`], a
//! `NULL` origin is `None`.

use crate::d_englsh as msg;
use crate::d_player::Player;
use crate::doomdef::Card;
use crate::m_fixed::{Fixed, FRACUNIT};
use crate::p_floor::{t_move_plane, PlaneKind, ResultE};
use crate::p_setup::Level;
use crate::p_spec::{p_find_lowest_ceiling_surrounding, p_find_sector_from_line_tag};
use crate::p_tick::{ThinkFn, ThinkerData, ThinkerId, Thinkers};
use crate::r_defs::Line;
use crate::r_main::RMain;
use crate::s_sound::{s_start_sound, sector_origin};
use crate::sounds::Sfx;

pub const VDOORSPEED: Fixed = FRACUNIT * 2;
pub const VDOORWAIT: i32 = 150;

/// (`vldoor_e`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VlDoorType {
    Normal,
    Close30ThenOpen,
    Close,
    Open,
    RaiseIn5Mins,
    BlazeRaise,
    BlazeOpen,
    BlazeClose,
}

/// (`vldoor_t`)
#[derive(Debug, Clone, Copy)]
pub struct VlDoor {
    pub door_type: VlDoorType,
    pub sector: usize,
    pub topheight: Fixed,
    pub speed: Fixed,

    /// 1 = up, 0 = waiting at top, -1 = down (the original's own
    /// comment, preserved). Also `2` = initial wait (`raiseIn5Mins`).
    pub direction: i32,

    /// tics to wait at the top (the original's own comment, preserved).
    pub topwait: i32,
    /// (keep in case a door going down is reset) when it reaches 0,
    /// start going down (the original's own comment, preserved).
    pub topcountdown: i32,
}

/// Port of `T_VerticalDoor`.
#[allow(clippy::too_many_arguments)]
pub fn t_vertical_door(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    rmain: &mut RMain,
    validcount: i32,
    leveltime: i32,
    id: ThinkerId,
) {
    let door = *thinkers.vl_door(id).unwrap();

    match door.direction {
        0 => {
            // WAITING
            let d = thinkers.vl_door_mut(id).unwrap();
            d.topcountdown -= 1;
            if d.topcountdown == 0 {
                match door.door_type {
                    VlDoorType::BlazeRaise => {
                        d.direction = -1; // time to go back down
                        s_start_sound(sector_origin(door.sector), Sfx::SfxBdcls);
                    }
                    VlDoorType::Normal => {
                        d.direction = -1; // time to go back down
                        s_start_sound(sector_origin(door.sector), Sfx::SfxDorcls);
                    }
                    VlDoorType::Close30ThenOpen => {
                        d.direction = 1;
                        s_start_sound(sector_origin(door.sector), Sfx::SfxDoropn);
                    }
                    _ => {}
                }
            }
        }
        2 => {
            // INITIAL WAIT
            let d = thinkers.vl_door_mut(id).unwrap();
            d.topcountdown -= 1;
            if d.topcountdown == 0 && door.door_type == VlDoorType::RaiseIn5Mins {
                d.direction = 1;
                d.door_type = VlDoorType::Normal;
                s_start_sound(sector_origin(door.sector), Sfx::SfxDoropn);
            }
        }
        -1 => {
            // DOWN
            let dest = level.sectors[door.sector].floorheight;
            let res = t_move_plane(
                thinkers,
                level,
                players,
                rmain,
                validcount,
                leveltime,
                door.sector,
                door.speed,
                dest,
                false,
                PlaneKind::Ceiling,
                door.direction,
            );
            if res == ResultE::Pastdest {
                match door.door_type {
                    VlDoorType::BlazeRaise | VlDoorType::BlazeClose => {
                        level.sectors[door.sector].specialdata = None;
                        thinkers.remove(id); // unlink and free
                        s_start_sound(sector_origin(door.sector), Sfx::SfxBdcls);
                    }
                    VlDoorType::Normal | VlDoorType::Close => {
                        level.sectors[door.sector].specialdata = None;
                        thinkers.remove(id); // unlink and free
                    }
                    VlDoorType::Close30ThenOpen => {
                        let d = thinkers.vl_door_mut(id).unwrap();
                        d.direction = 0;
                        d.topcountdown = 35 * 30;
                    }
                    _ => {}
                }
            } else if res == ResultE::Crushed {
                match door.door_type {
                    VlDoorType::BlazeClose | VlDoorType::Close => {
                        // DO NOT GO BACK UP! (the original's own
                        // comment, preserved).
                    }
                    _ => {
                        thinkers.vl_door_mut(id).unwrap().direction = 1;
                        s_start_sound(sector_origin(door.sector), Sfx::SfxDoropn);
                    }
                }
            }
        }
        1 => {
            // UP
            let res = t_move_plane(
                thinkers,
                level,
                players,
                rmain,
                validcount,
                leveltime,
                door.sector,
                door.speed,
                door.topheight,
                false,
                PlaneKind::Ceiling,
                door.direction,
            );
            if res == ResultE::Pastdest {
                match door.door_type {
                    VlDoorType::BlazeRaise | VlDoorType::Normal => {
                        let d = thinkers.vl_door_mut(id).unwrap();
                        d.direction = 0; // wait at top
                        d.topcountdown = door.topwait;
                    }
                    VlDoorType::Close30ThenOpen | VlDoorType::BlazeOpen | VlDoorType::Open => {
                        level.sectors[door.sector].specialdata = None;
                        thinkers.remove(id); // unlink and free
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Port of `EV_DoLockedDoor`. Move a locked door up/down (the
/// original's own comment, preserved). `thing` must be the player's own
/// mobj id; `None` (no owning player) mirrors the original's `if (!p)
/// return 0`.
pub fn ev_do_locked_door(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    line: &Line,
    door_type: VlDoorType,
    thing: ThinkerId,
) -> bool {
    let Some(player_idx) = thinkers.mobj(thing).and_then(|m| m.player) else {
        return false;
    };
    let Some(player) = players.get_mut(player_idx) else {
        return false;
    };

    match line.special {
        99 | 133 => {
            // Blue Lock
            if !player.has_card(Card::ItBluecard) && !player.has_card(Card::ItBlueskull) {
                player.message = Some(msg::PD_BLUEO);
                s_start_sound(None, Sfx::SfxOof);
                return false;
            }
        }
        134 | 135 => {
            // Red Lock
            if !player.has_card(Card::ItRedcard) && !player.has_card(Card::ItRedskull) {
                player.message = Some(msg::PD_REDO);
                s_start_sound(None, Sfx::SfxOof);
                return false;
            }
        }
        136 | 137 => {
            // Yellow Lock
            if !player.has_card(Card::ItYellowcard) && !player.has_card(Card::ItYellowskull) {
                player.message = Some(msg::PD_YELLOWO);
                s_start_sound(None, Sfx::SfxOof);
                return false;
            }
        }
        _ => {}
    }

    ev_do_door(thinkers, level, line, door_type)
}

/// Port of `EV_DoDoor`.
pub fn ev_do_door(
    thinkers: &mut Thinkers,
    level: &mut Level,
    line: &Line,
    door_type: VlDoorType,
) -> bool {
    let mut rtn = false;
    let mut secnum = -1;
    loop {
        secnum = p_find_sector_from_line_tag(level, line, secnum);
        if secnum < 0 {
            break;
        }
        let sec = secnum as usize;

        if level.sectors[sec].specialdata.is_some() {
            continue;
        }

        rtn = true;
        let mut door = VlDoor {
            door_type,
            sector: sec,
            topheight: 0,
            speed: VDOORSPEED,
            direction: 0,
            topwait: VDOORWAIT,
            topcountdown: 0,
        };

        match door_type {
            VlDoorType::BlazeClose => {
                door.topheight = p_find_lowest_ceiling_surrounding(level, sec) - 4 * FRACUNIT;
                door.direction = -1;
                door.speed = VDOORSPEED * 4;
                s_start_sound(sector_origin(sec), Sfx::SfxBdcls);
            }
            VlDoorType::Close => {
                door.topheight = p_find_lowest_ceiling_surrounding(level, sec) - 4 * FRACUNIT;
                door.direction = -1;
                s_start_sound(sector_origin(sec), Sfx::SfxDorcls);
            }
            VlDoorType::Close30ThenOpen => {
                door.topheight = level.sectors[sec].ceilingheight;
                door.direction = -1;
                s_start_sound(sector_origin(sec), Sfx::SfxDorcls);
            }
            VlDoorType::BlazeRaise | VlDoorType::BlazeOpen => {
                door.direction = 1;
                door.topheight = p_find_lowest_ceiling_surrounding(level, sec) - 4 * FRACUNIT;
                door.speed = VDOORSPEED * 4;
                if door.topheight != level.sectors[sec].ceilingheight {
                    s_start_sound(sector_origin(sec), Sfx::SfxBdopn);
                }
            }
            VlDoorType::Normal | VlDoorType::Open => {
                door.direction = 1;
                door.topheight = p_find_lowest_ceiling_surrounding(level, sec) - 4 * FRACUNIT;
                if door.topheight != level.sectors[sec].ceilingheight {
                    s_start_sound(sector_origin(sec), Sfx::SfxDoropn);
                }
            }
            VlDoorType::RaiseIn5Mins => {}
        }

        level.sectors[sec].specialdata =
            Some(thinkers.add_thinker(ThinkFn::VerticalDoor, ThinkerData::VlDoor(door)));
    }
    rtn
}

/// Port of `EV_VerticalDoor`. Open a door manually, no tag value (the
/// original's own comment, preserved). `thing` is the activating mobj's
/// id (used to find its owning player, if any, for lock checks/the
/// "bad guys never close doors" rule).
pub fn ev_vertical_door(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [Player],
    line: &Line,
    thing: ThinkerId,
) {
    let side = 0; // only front sides can be used (the original's own comment, preserved)
    let player_idx = thinkers.mobj(thing).and_then(|m| m.player);

    match line.special {
        26 | 32 => {
            // Blue Lock
            let Some(player) = player_idx.and_then(|i| players.get_mut(i)) else {
                return;
            };
            if !player.has_card(Card::ItBluecard) && !player.has_card(Card::ItBlueskull) {
                player.message = Some(msg::PD_BLUEK);
                s_start_sound(None, Sfx::SfxOof);
                return;
            }
        }
        27 | 34 => {
            // Yellow Lock
            let Some(player) = player_idx.and_then(|i| players.get_mut(i)) else {
                return;
            };
            if !player.has_card(Card::ItYellowcard) && !player.has_card(Card::ItYellowskull) {
                player.message = Some(msg::PD_YELLOWK);
                s_start_sound(None, Sfx::SfxOof);
                return;
            }
        }
        28 | 33 => {
            // Red Lock
            let Some(player) = player_idx.and_then(|i| players.get_mut(i)) else {
                return;
            };
            if !player.has_card(Card::ItRedcard) && !player.has_card(Card::ItRedskull) {
                player.message = Some(msg::PD_REDK);
                s_start_sound(None, Sfx::SfxOof);
                return;
            }
        }
        _ => {}
    }

    // if the sector has an active thinker, use it (the original's own
    // comment, preserved).
    let sec = line.sidenum[side ^ 1]
        .map(|s| level.sides[s].sector)
        .unwrap();

    if let Some(existing) = level.sectors[sec].specialdata {
        if let Some(door) = thinkers.vl_door_mut(existing) {
            if matches!(line.special, 1 | 26 | 27 | 28 | 117) {
                if door.direction == -1 {
                    door.direction = 1; // go back up
                } else {
                    if player_idx.is_none() {
                        return; // JDC: bad guys never close doors
                    }
                    door.direction = -1; // start going down immediately
                }
            }
        }
        return;
    }

    // for proper sound (the original's own comment, preserved)
    match line.special {
        117 | 118 => {
            // BLAZING DOOR RAISE / OPEN
            s_start_sound(sector_origin(sec), Sfx::SfxBdopn);
        }
        // NORMAL DOOR SOUND (1, 31) and the LOCKED DOOR SOUND default
        // arm play the same sound.
        _ => s_start_sound(sector_origin(sec), Sfx::SfxDoropn),
    }

    // new door thinker (the original's own comment, preserved).
    let mut door = VlDoor {
        door_type: VlDoorType::Normal,
        sector: sec,
        topheight: 0,
        speed: VDOORSPEED,
        direction: 1,
        topwait: VDOORWAIT,
        topcountdown: 0,
    };

    match line.special {
        1 | 26 | 27 | 28 => door.door_type = VlDoorType::Normal,
        31..=34 => {
            door.door_type = VlDoorType::Open;
        }
        117 => {
            door.door_type = VlDoorType::BlazeRaise;
            door.speed = VDOORSPEED * 4;
        }
        118 => {
            door.door_type = VlDoorType::BlazeOpen;
            door.speed = VDOORSPEED * 4;
        }
        _ => {}
    }

    // find the top and bottom of the movement range (the original's
    // own comment, preserved).
    door.topheight = p_find_lowest_ceiling_surrounding(level, sec) - 4 * FRACUNIT;

    level.sectors[sec].specialdata =
        Some(thinkers.add_thinker(ThinkFn::VerticalDoor, ThinkerData::VlDoor(door)));

    // line->special = 0 for the one-shot open variants (31-34/118) is
    // done by the caller ([`crate::p_spec`]'s dispatcher, Phase 7b4),
    // matching every other one-shot special in this port.
}

/// Port of `P_SpawnDoorCloseIn30`. Spawn a door that closes after 30
/// seconds (the original's own comment, preserved).
pub fn p_spawn_door_close_in_30(thinkers: &mut Thinkers, level: &mut Level, sector: usize) {
    level.sectors[sector].special = 0;
    let door = VlDoor {
        door_type: VlDoorType::Normal,
        sector,
        topheight: 0,
        speed: VDOORSPEED,
        direction: 0,
        topwait: VDOORWAIT,
        topcountdown: 30 * 35,
    };
    level.sectors[sector].specialdata =
        Some(thinkers.add_thinker(ThinkFn::VerticalDoor, ThinkerData::VlDoor(door)));
}

/// Port of `P_SpawnDoorRaiseIn5Mins`. Spawn a door that opens after 5
/// minutes (the original's own comment, preserved). `secnum` is unused
/// by the original too (dead parameter, kept for signature fidelity).
pub fn p_spawn_door_raise_in_5_mins(thinkers: &mut Thinkers, level: &mut Level, sector: usize) {
    level.sectors[sector].special = 0;
    let door = VlDoor {
        door_type: VlDoorType::RaiseIn5Mins,
        sector,
        topheight: p_find_lowest_ceiling_surrounding(level, sector) - 4 * FRACUNIT,
        speed: VDOORSPEED,
        direction: 2,
        topwait: VDOORWAIT,
        topcountdown: 5 * 60 * 35,
    };
    level.sectors[sector].specialdata =
        Some(thinkers.add_thinker(ThinkFn::VerticalDoor, ThinkerData::VlDoor(door)));
}

impl VlDoorType {
    /// Every variant in declaration order (the original's enum values), so
    /// `ALL[v as usize] == v`.
    pub const ALL: [VlDoorType; 8] = [
        VlDoorType::Normal,
        VlDoorType::Close30ThenOpen,
        VlDoorType::Close,
        VlDoorType::Open,
        VlDoorType::RaiseIn5Mins,
        VlDoorType::BlazeRaise,
        VlDoorType::BlazeOpen,
        VlDoorType::BlazeClose,
    ];

    /// The safe inverse of `as usize` (used by the savegame reader).
    pub fn from_index(i: usize) -> Option<VlDoorType> {
        Self::ALL.get(i).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r_defs::Sector;

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
    fn ev_do_door_normal_targets_lowest_surrounding_ceiling_minus_4() {
        let mut level = level_with_sectors(&[(0, 64), (0, 100)]);
        level.sectors[0].tag = 1;
        level.lines.push(Line {
            flags: crate::doomdata::ML_TWOSIDED,
            frontsector: Some(0),
            backsector: Some(1),
            ..Default::default()
        });
        level.sectors[0].lines = vec![0];

        let mut thinkers = Thinkers::new();
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let rtn = ev_do_door(&mut thinkers, &mut level, &line, VlDoorType::Normal);

        assert!(rtn);
        let id = level.sectors[0].specialdata.expect("door spawned");
        let door = thinkers.vl_door(id).unwrap();
        assert_eq!(door.topheight, 100 * FRACUNIT - 4 * FRACUNIT);
        assert_eq!(door.direction, 1);
    }

    #[test]
    fn ev_do_locked_door_rejects_without_the_key() {
        let mut level = level_with_sectors(&[(0, 64)]);
        level.sectors[0].tag = 1;
        let mut thinkers = Thinkers::new();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj({
                let mut m = crate::r_defs::Mobj::blank(crate::info::MobjType::MtPlayer);
                m.player = Some(0);
                m
            }),
        );
        let mut players = vec![Player::for_test(mo)];
        let line = Line {
            tag: 1,
            special: 99,
            ..Default::default()
        };

        let rtn = ev_do_locked_door(
            &mut thinkers,
            &mut level,
            &mut players,
            &line,
            VlDoorType::Open,
            mo,
        );

        assert!(!rtn, "no blue key: should be rejected");
        assert_eq!(players[0].message, Some(msg::PD_BLUEO));
        assert!(level.sectors[0].specialdata.is_none());
    }

    #[test]
    fn ev_do_locked_door_succeeds_with_the_key() {
        let mut level = level_with_sectors(&[(0, 64)]);
        level.sectors[0].tag = 1;
        let mut thinkers = Thinkers::new();
        let mo = thinkers.add_thinker(
            crate::p_tick::ThinkFn::MobjThinker,
            crate::p_tick::ThinkerData::Mobj({
                let mut m = crate::r_defs::Mobj::blank(crate::info::MobjType::MtPlayer);
                m.player = Some(0);
                m
            }),
        );
        let mut players = vec![Player::for_test(mo)];
        players[0].cards[Card::ItBluecard as usize] = true;
        let line = Line {
            tag: 1,
            special: 99,
            ..Default::default()
        };

        let rtn = ev_do_locked_door(
            &mut thinkers,
            &mut level,
            &mut players,
            &line,
            VlDoorType::Open,
            mo,
        );

        assert!(rtn);
        assert!(level.sectors[0].specialdata.is_some());
    }

    #[test]
    fn t_vertical_door_up_waits_at_top_then_reverses() {
        let mut level = level_with_sectors(&[(0, 64)]);
        let mut thinkers = Thinkers::new();
        let mut rmain = RMain::new();
        let mut players: Vec<Player> = Vec::new();

        let door = VlDoor {
            door_type: VlDoorType::Normal,
            sector: 0,
            topheight: 10 * FRACUNIT,
            speed: 100 * FRACUNIT,
            direction: 1,
            topwait: VDOORWAIT,
            topcountdown: 0,
        };
        let id = thinkers.add_thinker(ThinkFn::VerticalDoor, ThinkerData::VlDoor(door));

        t_vertical_door(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            1,
            0,
            id,
        );

        assert_eq!(level.sectors[0].ceilingheight, 10 * FRACUNIT);
        let d = thinkers.vl_door(id).unwrap();
        assert_eq!(d.direction, 0, "should be waiting at top");
        assert_eq!(d.topcountdown, VDOORWAIT);
    }

    #[test]
    fn opening_a_door_queues_the_door_open_sound_at_its_sector() {
        crate::s_sound::test_init(3);
        let mut level = level_with_sectors(&[(0, 64), (0, 100)]);
        level.sectors[0].tag = 1;
        level.lines.push(Line {
            flags: crate::doomdata::ML_TWOSIDED,
            frontsector: Some(0),
            backsector: Some(1),
            ..Default::default()
        });
        level.sectors[0].lines = vec![0];
        let mut thinkers = Thinkers::new();
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        ev_do_door(&mut thinkers, &mut level, &line, VlDoorType::Normal);
        crate::s_sound::test_flush(&thinkers, &level);

        assert_eq!(
            crate::s_sound::test_playing(),
            vec![Sfx::SfxDoropn as usize]
        );
        crate::s_sound::s_shutdown();
    }
}
