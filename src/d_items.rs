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
//
//-----------------------------------------------------------------------------

//! Rust port of `d_items.h` / `d_items.c`.
//!
//! Items: key cards, artifacts, weapon, ammunition.

use crate::doomdef::{AmmoType, NUMWEAPONS};
use crate::info::StateNum;

/// (`weaponinfo_t`) — weapon info: sprite frames, ammunition use.
#[derive(Debug, Clone, Copy)]
pub struct WeaponInfo {
    pub ammo: AmmoType,
    pub upstate: StateNum,
    pub downstate: StateNum,
    pub readystate: StateNum,
    pub atkstate: StateNum,
    pub flashstate: StateNum,
}

/// (`NUMWEAPONS`)
pub const NUM_WEAPONINFO: usize = NUMWEAPONS;

/// (`weaponinfo[NUMWEAPONS]`), transcribed from `d_items.c`. Small and
/// stable (9 entries, 6 fields each) — checked by hand against the
/// original field-by-field (see `weaponinfo_matches_original_entries`
/// below), unlike the ~4700-line generated `info.c` tables this
/// references (`tools/gen_info.py`, Phase 6a).
pub static WEAPONINFO: [WeaponInfo; NUMWEAPONS] = [
    // fist
    WeaponInfo {
        ammo: AmmoType::AmNoammo,
        upstate: StateNum::SPunchup,
        downstate: StateNum::SPunchdown,
        readystate: StateNum::SPunch,
        atkstate: StateNum::SPunch1,
        flashstate: StateNum::SNull,
    },
    // pistol
    WeaponInfo {
        ammo: AmmoType::AmClip,
        upstate: StateNum::SPistolup,
        downstate: StateNum::SPistoldown,
        readystate: StateNum::SPistol,
        atkstate: StateNum::SPistol1,
        flashstate: StateNum::SPistolflash,
    },
    // shotgun
    WeaponInfo {
        ammo: AmmoType::AmShell,
        upstate: StateNum::SSgunup,
        downstate: StateNum::SSgundown,
        readystate: StateNum::SSgun,
        atkstate: StateNum::SSgun1,
        flashstate: StateNum::SSgunflash1,
    },
    // chaingun
    WeaponInfo {
        ammo: AmmoType::AmClip,
        upstate: StateNum::SChainup,
        downstate: StateNum::SChaindown,
        readystate: StateNum::SChain,
        atkstate: StateNum::SChain1,
        flashstate: StateNum::SChainflash1,
    },
    // missile launcher
    WeaponInfo {
        ammo: AmmoType::AmMisl,
        upstate: StateNum::SMissileup,
        downstate: StateNum::SMissiledown,
        readystate: StateNum::SMissile,
        atkstate: StateNum::SMissile1,
        flashstate: StateNum::SMissileflash1,
    },
    // plasma rifle
    WeaponInfo {
        ammo: AmmoType::AmCell,
        upstate: StateNum::SPlasmaup,
        downstate: StateNum::SPlasmadown,
        readystate: StateNum::SPlasma,
        atkstate: StateNum::SPlasma1,
        flashstate: StateNum::SPlasmaflash1,
    },
    // bfg 9000
    WeaponInfo {
        ammo: AmmoType::AmCell,
        upstate: StateNum::SBfgup,
        downstate: StateNum::SBfgdown,
        readystate: StateNum::SBfg,
        atkstate: StateNum::SBfg1,
        flashstate: StateNum::SBfgflash1,
    },
    // chainsaw
    WeaponInfo {
        ammo: AmmoType::AmNoammo,
        upstate: StateNum::SSawup,
        downstate: StateNum::SSawdown,
        readystate: StateNum::SSaw,
        atkstate: StateNum::SSaw1,
        flashstate: StateNum::SNull,
    },
    // super shotgun
    WeaponInfo {
        ammo: AmmoType::AmShell,
        upstate: StateNum::SDsgunup,
        downstate: StateNum::SDsgundown,
        readystate: StateNum::SDsgun,
        atkstate: StateNum::SDsgun1,
        flashstate: StateNum::SDsgunflash1,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doomdef::WeaponType;

    #[test]
    fn num_weaponinfo_matches_numweapons() {
        assert_eq!(NUM_WEAPONINFO, 9);
    }

    /// Checked by hand, field by field, against `d_items.c`'s
    /// `weaponinfo[NUMWEAPONS]` initializer.
    #[test]
    fn weaponinfo_matches_original_entries() {
        let fist = WEAPONINFO[WeaponType::WpFist as usize];
        assert_eq!(fist.ammo, AmmoType::AmNoammo);
        assert_eq!(fist.upstate, StateNum::SPunchup);
        assert_eq!(fist.downstate, StateNum::SPunchdown);
        assert_eq!(fist.readystate, StateNum::SPunch);
        assert_eq!(fist.atkstate, StateNum::SPunch1);
        assert_eq!(fist.flashstate, StateNum::SNull);

        let chainsaw = WEAPONINFO[WeaponType::WpChainsaw as usize];
        assert_eq!(chainsaw.ammo, AmmoType::AmNoammo);
        assert_eq!(chainsaw.flashstate, StateNum::SNull);

        let bfg = WEAPONINFO[WeaponType::WpBfg as usize];
        assert_eq!(bfg.ammo, AmmoType::AmCell);
        assert_eq!(bfg.upstate, StateNum::SBfgup);
        assert_eq!(bfg.atkstate, StateNum::SBfg1);
        assert_eq!(bfg.flashstate, StateNum::SBfgflash1);

        let supershotgun = WEAPONINFO[WeaponType::WpSupershotgun as usize];
        assert_eq!(supershotgun.ammo, AmmoType::AmShell);
        assert_eq!(supershotgun.readystate, StateNum::SDsgun);
    }
}
