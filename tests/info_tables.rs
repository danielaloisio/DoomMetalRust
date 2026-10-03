//! Spot checks of the generated `info.rs` tables (Phase 6a) against
//! values read off the original `info.c`, compiled and dumped with a C
//! harness when the tables were first generated (every entry of every
//! table matched then; these keep a few of them pinned).
//!
//! Kept outside `src/info.rs` so regenerating that file with
//! `tools/gen_info.py` doesn't drop them.

use doommetal_rust::info::*;
use doommetal_rust::m_fixed::FRACUNIT;
use doommetal_rust::r_defs::mobj_flag;
use doommetal_rust::sounds::Sfx;

#[test]
fn table_sizes_match_the_original() {
    assert_eq!(NUMSPRITES, 138);
    assert_eq!(NUMSTATES, 967);
    assert_eq!(NUMMOBJTYPES, 137);
    assert_eq!(SPRNAMES.len(), NUMSPRITES);
    assert_eq!(StateNum::STech2lamp4 as usize + 1, NUMSTATES);
}

#[test]
fn enum_values_index_their_tables() {
    assert_eq!(SPRNAMES[SpriteNum::SprTroo as usize], "TROO");
    assert_eq!(SPRNAMES[SpriteNum::SprTlp2 as usize], "TLP2");
    assert_eq!(StateNum::SPlay as usize, 149);
    assert_eq!(MobjType::MtTroop as usize, 11);
}

#[test]
fn states_match_original_entries() {
    // {SPR_PLAY,0,-1,{NULL},S_NULL,0,0}, // S_PLAY
    let play = STATES[StateNum::SPlay as usize];
    assert_eq!(play.sprite, SpriteNum::SprPlay);
    assert_eq!((play.frame, play.tics), (0, -1));
    assert_eq!(play.action, StateAction::None);
    assert_eq!(play.nextstate, StateNum::SNull);

    // {SPR_PISG,0,1,{A_WeaponReady},S_PISTOL,0,0}, // S_PISTOL
    let pistol = STATES[StateNum::SPistol as usize];
    assert_eq!(pistol.sprite, SpriteNum::SprPisg);
    assert_eq!(pistol.action, StateAction::AWeaponReady);
    assert_eq!(pistol.nextstate, StateNum::SPistol);

    // Full-bright frames keep FF_FULLBRIGHT (0x8000) in `frame`:
    // {SPR_COLU,32768,-1,{NULL},S_NULL,0,0}, // S_COLU
    assert_eq!(STATES[StateNum::SColu as usize].frame, 0x8000);
}

#[test]
fn mobjinfo_matches_original_entries() {
    let imp = MOBJINFO[MobjType::MtTroop as usize];
    assert_eq!(imp.doomednum, 3001);
    assert_eq!(imp.spawnstate, StateNum::STrooStnd);
    assert_eq!(imp.spawnhealth, 60);
    assert_eq!(imp.seesound, Sfx::SfxBgsit1);
    assert_eq!(imp.attacksound, Sfx::SfxNone);
    assert_eq!(imp.painchance, 200);
    assert_eq!(imp.speed, 8);
    assert_eq!(imp.radius, 20 * FRACUNIT);
    assert_eq!(imp.height, 56 * FRACUNIT);
    assert_eq!(
        imp.flags,
        mobj_flag::SOLID | mobj_flag::SHOOTABLE | mobj_flag::COUNTKILL
    );
    assert_eq!(imp.raisestate, StateNum::STrooRaise1);

    let player = MOBJINFO[MobjType::MtPlayer as usize];
    assert_eq!(player.doomednum, -1);
    assert_eq!(player.spawnstate, StateNum::SPlay);
    assert_eq!(player.radius, 16 * FRACUNIT);
    assert_eq!(player.flags, 33557510);
}

#[test]
fn placeable_things_have_unique_editor_numbers() {
    let mut seen = std::collections::HashSet::new();
    for info in MOBJINFO.iter().filter(|m| m.doomednum != -1) {
        assert!(
            seen.insert(info.doomednum),
            "duplicate doomednum {}",
            info.doomednum
        );
    }
}
