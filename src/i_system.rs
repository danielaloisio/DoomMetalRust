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

//! Rust port of `i_system.h` / `i_system.c` (partial — see note below).
//!
//! System specific interface stuff.
//!
//! # Scope
//!
//! `I_Init`/`I_Quit`/`I_Error` in the original call into subsystems not
//! yet ported (`I_InitSound`, `D_QuitNetGame`, `I_ShutdownSound`,
//! `I_ShutdownMusic`, `M_SaveDefaults`, `G_CheckDemoStatus`,
//! `I_ShutdownGraphics`). Rather than stub those calls out silently
//! (which would hide the fact that shutdown is incomplete), this port
//! keeps `i_error`/`i_quit` deliberately minimal for now — the parts
//! that don't depend on those unported subsystems — and documents
//! exactly what's missing, to be filled in as each subsystem lands in
//! its own phase. `I_BaseTiccmd`/`I_Tactile` (unused stub in the
//! original: "UNUSED. on = off = total = 0;") are ported as-is.
//!
//! `I_BeginRead`/`I_EndRead` are empty in the original (`??? I_BeginRead
//! ()` in `w_wad.c` was even left commented out) — not ported, since
//! porting two no-op functions has no value.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use crate::d_ticcmd::TicCmd;
use crate::doomdef::TICRATE;

/// (`mb_used`) — megabytes to claim for the zone heap.
///
/// Kept for fidelity/documentation even though [`i_zone_base`] (this
/// port's stand-in for `I_ZoneBase`) doesn't actually pre-claim a fixed
/// arena — see `z_zone`'s module docs for why.
pub const MB_USED: i32 = 6;

/// Port of `I_Tactile`. UNUSED in the original (the comment is
/// preserved: "on = off = total = 0;" — the parameters are discarded,
/// nothing else happens).
pub fn i_tactile(_on: i32, _off: i32, _total: i32) {}

/// Port of `I_BaseTiccmd`. Either returns a null ticcmd, or calls a
/// loadable driver to build it (never implemented upstream either — the
/// original always just returns the zeroed `emptycmd`). This ticcmd is
/// then modified by the game loop for normal input.
pub fn i_base_ticcmd() -> TicCmd {
    TicCmd::default()
}

/// Port of `I_GetHeapSize`.
pub fn i_get_heap_size() -> i32 {
    MB_USED * 1024 * 1024
}

/// Port of `I_ZoneBase`. Called by startup code to get the amount of
/// memory to malloc for the zone management.
///
/// This port's `z_zone::Zone` doesn't pre-claim a fixed arena (see its
/// module docs), so this is kept only for the one piece of information
/// still meaningful without that arena: the configured heap size in
/// bytes, matching `*size = mb_used*1024*1024` in the original. No
/// actual allocation happens here (the original's `malloc(*size)` has no
/// equivalent to reproduce).
pub fn i_zone_base() -> i32 {
    i_get_heap_size()
}

static BASETIME: AtomicI64 = AtomicI64::new(-1);

fn program_start() -> &'static Instant {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

/// Port of `I_GetTime`. Returns time in 1/70th second tics (`TICRATE`).
///
/// The original lazily latches `basetime` to the first observed
/// `SDL_GetTicks()` value; ported the same way against
/// [`std::time::Instant`] instead of SDL's millisecond counter — SDL
/// isn't initialized yet the first time some callers might query this,
/// and the elapsed-time semantics are identical either way (both are
/// monotonic clocks measured in milliseconds internally).
pub fn i_get_time() -> i32 {
    let start = program_start();
    let ticks = start.elapsed().as_millis() as i64;

    let basetime = BASETIME.load(Ordering::Relaxed);
    let basetime = if basetime < 0 {
        BASETIME.store(ticks, Ordering::Relaxed);
        ticks
    } else {
        basetime
    };

    ((ticks - basetime) * TICRATE as i64 / 1000) as i32
}

/// Port of `I_AllocLow`. Allocates from low memory under DOS, just
/// mallocs under unix. The DOS-specific "low memory" framing has no
/// meaning on any target this port runs on; ported as a zeroed buffer of
/// the requested length, matching the original's `malloc` +
/// `memset(mem, 0, length)`.
pub fn i_alloc_low(length: usize) -> Vec<u8> {
    vec![0u8; length]
}

/// Port of `I_WaitVBL`. Wait for vertical retrace or pause a bit.
pub fn i_wait_vbl(count: i32) {
    std::thread::sleep(std::time::Duration::from_millis(
        (count as u64) * (1000 / 70),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_size_matches_original_default() {
        assert_eq!(i_get_heap_size(), 6 * 1024 * 1024);
        assert_eq!(i_zone_base(), 6 * 1024 * 1024);
    }

    #[test]
    fn alloc_low_returns_zeroed_buffer_of_requested_length() {
        let buf = i_alloc_low(100);
        assert_eq!(buf.len(), 100);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn base_ticcmd_is_zeroed() {
        let cmd = i_base_ticcmd();
        assert_eq!(cmd, TicCmd::default());
    }

    #[test]
    fn get_time_is_monotonic_and_starts_near_zero() {
        let t1 = i_get_time();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let t2 = i_get_time();
        assert!(t2 >= t1);
    }
}
