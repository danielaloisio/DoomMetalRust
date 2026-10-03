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
//  Sky rendering. The DOOM sky is a texture map like any
//  wall, wrapping around. A 1024 columns equal 360 degrees.
//  The default sky map is 256 columns and repeats 4 times
//  on a 320 screen?
//
//-----------------------------------------------------------------------------

//! Rust port of `r_sky.h` / `r_sky.c`. Sky rendering. The DOOM sky is a
//! texture map like any wall, wrapping around. A 1024 columns equal 360
//! degrees. The default sky map is 256 columns and repeats 4 times on a
//! 320 screen? (the original's own comment, preserved).
//!
//! # Scope
//!
//! The original itself is nearly a stub: `skyflatnum` is declared but
//! never assigned here (the commented-out `R_FlatNumForName` call is
//! dead code in the source as shipped — `doomstat`/`p_setup` are
//! expected to set `skyflatnum` elsewhere), and `skytexture` is never
//! set in this file either (`R_ExecuteSetViewSize`/`G_InitNew` do that
//! in the original, both Phase 6). Only [`r_init_sky_map`]
//! (`R_InitSkyMap`) exists as real logic here, faithfully reproducing
//! that it's the *only* thing this file actually does.
//!
//! The three globals themselves are bundled as [`Sky`], which the
//! renderer passes to whoever reads them (`r_bsp`/`r_segs`/`r_plane`).
//! Setting them per level is `G_DoLoadLevel`'s job (Phase 6).

use crate::m_fixed::{Fixed, FRACUNIT};

/// (`SKYFLATNAME`) — flat lump name marking a sky ceiling/floor.
pub const SKYFLATNAME: &str = "F_SKY1";

/// (`ANGLETOSKYSHIFT`) — the sky map is 256*128*4 maps (the original's
/// own comment, preserved).
pub const ANGLETOSKYSHIFT: i32 = 22;

/// (`skyflatnum`/`skytexture`/`skytexturemid`) — see module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sky {
    /// Flat number of [`SKYFLATNAME`]; `-1` (no flat) until a level
    /// sets it.
    pub skyflatnum: i32,
    /// Texture number of the sky texture drawn where the sky flat is.
    pub skytexture: i32,
    pub skytexturemid: Fixed,
}

impl Default for Sky {
    fn default() -> Self {
        Sky {
            skyflatnum: -1,
            skytexture: 0,
            skytexturemid: r_init_sky_map(),
        }
    }
}

/// Port of `R_InitSkyMap`. Called whenever the view size changes (the
/// original's own comment, preserved). Returns `skytexturemid` directly
/// — this is the entirety of the original function's real effect (see
/// module docs), so there's no state worth bundling into a struct for
/// it.
pub fn r_init_sky_map() -> Fixed {
    100 * FRACUNIT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r_init_sky_map_matches_original_constant() {
        assert_eq!(r_init_sky_map(), 100 * FRACUNIT);
    }
}
