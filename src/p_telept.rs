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
//	Teleportation.
//
//-----------------------------------------------------------------------------

//! Rust port of `p_telept.c`. Teleportation (the original's own
//! description, preserved).
//!
//! Sounds: `sfx_telept` at source and destination, queued via
//! [`crate::s_sound`] with the fog mobj as origin.

use crate::info::MobjType;
use crate::p_map::p_teleport_move;
use crate::p_mobj::{p_spawn_mobj, SpawnZ};
use crate::p_setup::Level;
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_defs::mobj_flag;
use crate::r_defs::Line;
use crate::s_sound::{s_start_sound, SoundOrigin};
use crate::sounds::Sfx;
use crate::tables::{fine_cosine, ANGLETOFINESHIFT, FINESINE};

/// Port of `EV_Teleport`.
pub fn ev_teleport(
    thinkers: &mut Thinkers,
    level: &mut Level,
    players: &mut [crate::d_player::Player],
    rmain: &mut crate::r_main::RMain,
    line: &Line,
    side: i32,
    thing: ThinkerId,
) -> bool {
    // don't teleport missiles (the original's own comment, preserved).
    if thinkers.mobj(thing).unwrap().flags & mobj_flag::MISSILE != 0 {
        return false;
    }

    // Don't teleport if hit back of line, so you can get out of
    // teleporter (the original's own comment, preserved).
    if side == 1 {
        return false;
    }

    let tag = line.tag;
    for i in 0..level.sectors.len() {
        if level.sectors[i].tag != tag {
            continue;
        }

        // Find a MT_TELEPORTMAN mobj whose subsector belongs to sector
        // `i` — standing in for the original's walk of `thinkercap`
        // filtered to `P_MobjThinker`/`MT_TELEPORTMAN` (see
        // `p_tick`'s docs: `Thinkers::iter_mobjs` already gives every
        // live mobj, in list order, without needing a raw function-
        // pointer identity check).
        let landing = thinkers.iter_mobjs().find_map(|(id, m)| {
            if m.mobj_type != MobjType::MtTeleportman {
                return None;
            }
            let sector = level.subsectors[m.subsector?].sector;
            (sector == i).then_some((id, m.x, m.y, m.angle))
        });

        let Some((_landing_id, dest_x, dest_y, dest_angle)) = landing else {
            continue;
        };

        let (old_x, old_y, old_z) = {
            let t = thinkers.mobj(thing).unwrap();
            (t.x, t.y, t.z)
        };

        if !p_teleport_move(thinkers, level, players, rmain, thing, dest_x, dest_y) {
            return false;
        }

        let floorz = thinkers.mobj(thing).unwrap().floorz;
        {
            let t = thinkers.mobj_mut(thing).unwrap();
            t.z = floorz; // fixme: not needed? (the original's own comment, preserved)
        }

        // spawn teleport fog at source and destination (the original's
        // own comment, preserved).
        let fog_src = p_spawn_mobj(
            thinkers,
            level,
            old_x,
            old_y,
            SpawnZ::At(old_z),
            MobjType::MtTfog,
        );
        s_start_sound(Some(SoundOrigin::Mobj(fog_src)), Sfx::SfxTelept);

        let an = (dest_angle >> ANGLETOFINESHIFT) as usize;
        let fog_x = dest_x + 20 * fine_cosine(an);
        let fog_y = dest_y + 20 * FINESINE[an];
        let thing_z = thinkers.mobj(thing).unwrap().z;
        let fog_dst = p_spawn_mobj(
            thinkers,
            level,
            fog_x,
            fog_y,
            SpawnZ::At(thing_z),
            MobjType::MtTfog,
        );
        // emit sound, where? (the original's own comment, preserved)
        s_start_sound(Some(SoundOrigin::Mobj(fog_dst)), Sfx::SfxTelept);

        let is_player = thinkers.mobj(thing).unwrap().player.is_some();
        {
            let t = thinkers.mobj_mut(thing).unwrap();
            // don't move for a bit (the original's own comment,
            // preserved) — player-only, matching `if (thing->player)`.
            if is_player {
                t.reactiontime = 18;
            }
            t.angle = dest_angle;
            t.momx = 0;
            t.momy = 0;
            t.momz = 0;
        }

        return true;
    }
    false
}

/// Sets `player.viewz` after a successful [`ev_teleport`] — the
/// original's `if (thing->player) thing->player->viewz = thing->z +
/// thing->player->viewheight;`, split out because it needs `&mut
/// Player` (this port threads `players` separately from `Thinkers`,
/// same convention as every other Phase 7 mover) while [`ev_teleport`]
/// itself only takes `Thinkers`/`Level`. Callers that know `thing` is
/// the player's own mobj call this right after a `true` return from
/// [`ev_teleport`]; a no-op otherwise (mirrors the original's `if
/// (thing->player)` guard).
pub fn ev_teleport_fixup_player_viewz(
    thinkers: &Thinkers,
    player: &mut crate::d_player::Player,
    thing: ThinkerId,
) {
    let mobj = thinkers.mobj(thing).unwrap();
    if mobj.player.is_some() {
        player.viewz = mobj.z + player.viewheight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p_mobj::p_spawn_mobj;
    use crate::r_defs::Sector;
    use crate::r_defs::Subsector;

    /// Sector 0 is `x <= 100` (untagged, the thing's starting side);
    /// sector 1 is `x > 100` (tag 1, the teleport destination) — see the
    /// node below.
    fn small_level_with_two_sectors() -> Level {
        let mut level = Level::default();
        level.sectors = vec![
            Sector {
                floorheight: 0,
                ceilingheight: 512 * crate::m_fixed::FRACUNIT,
                tag: 0,
                ..Default::default()
            },
            Sector {
                floorheight: 0,
                ceilingheight: 512 * crate::m_fixed::FRACUNIT,
                tag: 1,
                ..Default::default()
            },
        ];
        level.subsectors = vec![
            Subsector {
                sector: 0,
                ..Default::default()
            },
            Subsector {
                sector: 1,
                ..Default::default()
            },
        ];
        // A single axis-aligned partition node at x=100 so
        // `RMain::point_in_subsector` actually routes points to the
        // right subsector — `p_spawn_mobj`/`p_teleport_move` both
        // re-derive `subsector` from `x`/`y` via `p_set_thing_position`,
        // so a level with no BSP at all (`nodes` empty, which always
        // resolves to subsector 0) can't exercise "thing lands in a
        // specific other sector" the way this test needs to.
        level.nodes = vec![crate::r_defs::Node {
            x: 100 * crate::m_fixed::FRACUNIT,
            y: 0,
            dx: 0,
            dy: crate::m_fixed::FRACUNIT, // dy > 0
            bbox: [[0; 4]; 2],
            // dx == 0, dy > 0: point_on_side returns 1 when x <= node.x,
            // 0 when x > node.x (see `RMain::point_on_side`'s dx==0
            // branch) — children[side] picks the subsector, so side 1
            // (x <= 100) must be subsector 0 and side 0 (x > 100) must
            // be subsector 1 to route as this test's comments describe.
            children: [
                crate::doomdata::NF_SUBSECTOR | 1, // x > 100 -> subsector 1
                crate::doomdata::NF_SUBSECTOR,     // x <= 100 -> subsector 0
            ],
        }];
        level
    }

    #[test]
    fn ev_teleport_moves_thing_to_the_teleportman_landing() {
        let mut level = small_level_with_two_sectors();
        let mut thinkers = Thinkers::new();

        // MT_TELEPORTMAN sitting in sector 1 (destination) — x=5000 is
        // past the x=100 partition, so it lands in subsector 1.
        let landing = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            5000 * crate::m_fixed::FRACUNIT,
            6000 * crate::m_fixed::FRACUNIT,
            SpawnZ::OnFloor,
            MobjType::MtTeleportman,
        );
        assert_eq!(
            thinkers.mobj(landing).unwrap().subsector,
            Some(1),
            "sanity check: the teleportman should land in the tagged sector's subsector"
        );

        // thing starts at x=0, on the other side of the partition (subsector 0).
        let thing = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            0,
            0,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );

        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = crate::r_main::RMain::new();
        let ok = ev_teleport(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            &line,
            0,
            thing,
        );

        assert!(ok);
        let t = thinkers.mobj(thing).unwrap();
        assert_eq!(t.x, 5000 * crate::m_fixed::FRACUNIT);
        assert_eq!(t.y, 6000 * crate::m_fixed::FRACUNIT);
        assert_eq!(t.momx, 0);
        assert_eq!(t.momy, 0);
        assert_eq!(t.momz, 0);
    }

    #[test]
    fn ev_teleport_rejects_missiles() {
        let mut level = small_level_with_two_sectors();
        let mut thinkers = Thinkers::new();
        let thing = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            0,
            0,
            SpawnZ::OnFloor,
            MobjType::MtRocket,
        );
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = crate::r_main::RMain::new();
        assert!(!ev_teleport(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            &line,
            0,
            thing
        ));
    }

    #[test]
    fn ev_teleport_rejects_hitting_the_back_side() {
        let mut level = small_level_with_two_sectors();
        let mut thinkers = Thinkers::new();
        let thing = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            0,
            0,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = crate::r_main::RMain::new();
        assert!(!ev_teleport(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            &line,
            1,
            thing
        ));
    }

    #[test]
    fn ev_teleport_fails_when_no_teleportman_in_tagged_sector() {
        let mut level = small_level_with_two_sectors();
        let mut thinkers = Thinkers::new();
        let thing = p_spawn_mobj(
            &mut thinkers,
            &mut level,
            0,
            0,
            SpawnZ::OnFloor,
            MobjType::MtPlayer,
        );
        let line = Line {
            tag: 1,
            ..Default::default()
        };

        // no MT_TELEPORTMAN spawned anywhere
        let mut players: Vec<crate::d_player::Player> = Vec::new();
        let mut rmain = crate::r_main::RMain::new();
        assert!(!ev_teleport(
            &mut thinkers,
            &mut level,
            &mut players,
            &mut rmain,
            &line,
            0,
            thing
        ));
    }
}
