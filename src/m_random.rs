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
//	Random number LUT.
//
//-----------------------------------------------------------------------------

//! Rust port of `m_random.h` / `m_random.c`.
//!
//! Random number LUT. Two independent cursors walk the same fixed
//! 256-byte table: `rndindex` for `M_Random` (menu/UI/non-deterministic
//! uses) and `prndindex` for `P_Random` (play simulation — must stay
//! deterministic for demo compatibility). Both cursors, like the rest of
//! the classic engine's global state, live behind module-local statics
//! (see the plan's globals strategy) rather than being passed around
//! explicitly, to keep the ported call sites `M_Random()`/`P_Random()`
//! shaped exactly like the original.

use std::sync::atomic::{AtomicI32, Ordering};

/// (`rndtable[256]`) — returns a number from 0 to 255, from a lookup
/// table. Transcribed verbatim from `m_random.c`.
#[rustfmt::skip]
pub const RNDTABLE: [u8; 256] = [
    0,   8, 109, 220, 222, 241, 149, 107,  75, 248, 254, 140,  16,  66,
    74,  21, 211,  47,  80, 242, 154,  27, 205, 128, 161,  89,  77,  36,
    95, 110,  85,  48, 212, 140, 211, 249,  22,  79, 200,  50,  28, 188,
    52, 140, 202, 120,  68, 145,  62,  70, 184, 190,  91, 197, 152, 224,
    149, 104,  25, 178, 252, 182, 202, 182, 141, 197,   4,  81, 181, 242,
    145,  42,  39, 227, 156, 198, 225, 193, 219,  93, 122, 175, 249,   0,
    175, 143,  70, 239,  46, 246, 163,  53, 163, 109, 168, 135,   2, 235,
    25,  92,  20, 145, 138,  77,  69, 166,  78, 176, 173, 212, 166, 113,
    94, 161,  41,  50, 239,  49, 111, 164,  70,  60,   2,  37, 171,  75,
    136, 156,  11,  56,  42, 146, 138, 229,  73, 146,  77,  61,  98, 196,
    135, 106,  63, 197, 195,  86,  96, 203, 113, 101, 170, 247, 181, 113,
    80, 250, 108,   7, 255, 237, 129, 226,  79, 107, 112, 166, 103, 241,
    24, 223, 239, 120, 198,  58,  60,  82, 128,   3, 184,  66, 143, 224,
    145, 224,  81, 206, 163,  45,  63,  90, 168, 114,  59,  33, 159,  95,
    28, 139, 123,  98, 125, 196,  15,  70, 194, 253,  54,  14, 109, 226,
    71,  17, 161,  93, 186,  87, 244, 138,  20,  52, 123, 251,  26,  36,
    17,  46,  52, 231, 232,  76,  31, 221,  84,  37, 216, 165, 212, 106,
    197, 242,  98,  43,  39, 175, 254, 145, 190,  84, 118, 222, 187, 136,
    120, 163, 236, 249,
];

// The original declares these as plain `int rndindex = 0;` /
// `int prndindex = 0;` globals. Doom's simulation is single-threaded, so
// `AtomicI32` here is not about real concurrency: it is the lowest-risk
// way to get a mutable `static` in safe Rust without `static mut`
// (deprecated as directly-referenced global state going forward) or the
// larger `OnceLock<UnsafeCell<_>>` machinery the plan reserves for the
// bigger state groups (doomstat/r_state/d_player) — a single counter
// doesn't need that ceremony.
static RNDINDEX: AtomicI32 = AtomicI32::new(0);
static PRNDINDEX: AtomicI32 = AtomicI32::new(0);

/// Port of `P_Random`. As `M_Random`, but used only by the play
/// simulation — this is the deterministic one (demo-safe).
pub fn p_random() -> i32 {
    let next = (PRNDINDEX.load(Ordering::Relaxed) + 1) & 0xff;
    PRNDINDEX.store(next, Ordering::Relaxed);
    RNDTABLE[next as usize] as i32
}

/// Port of `M_Random`.
pub fn m_random() -> i32 {
    let next = (RNDINDEX.load(Ordering::Relaxed) + 1) & 0xff;
    RNDINDEX.store(next, Ordering::Relaxed);
    RNDTABLE[next as usize] as i32
}

/// `prndindex`: where the play simulation's generator is.
pub fn p_rnd_index() -> i32 {
    PRNDINDEX.load(Ordering::Relaxed)
}

/// `rndindex` (the menu/non-deterministic generator's position), read
/// by `G_Ticker`'s netgame consistency check.
pub fn rnd_index() -> i32 {
    RNDINDEX.load(Ordering::Relaxed)
}

/// Port of `M_ClearRandom`. Fix randoms for demos.
pub fn m_clear_random() {
    RNDINDEX.store(0, Ordering::Relaxed);
    PRNDINDEX.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The two cursors are process-global state (faithfully mirroring the
    // C globals), so tests that depend on their starting value must not
    // run concurrently with each other.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn rndtable_has_256_entries() {
        assert_eq!(RNDTABLE.len(), 256);
    }

    #[test]
    fn rndtable_first_and_last_entries_match_source() {
        assert_eq!(RNDTABLE[0], 0);
        assert_eq!(RNDTABLE[1], 8);
        assert_eq!(RNDTABLE[255], 249);
    }

    #[test]
    fn p_random_sequence_matches_original_after_clear() {
        let _guard = TEST_LOCK.lock().unwrap();
        m_clear_random();
        // prndindex starts at 0, first call increments to 1 -> rndtable[1] == 8.
        assert_eq!(p_random(), 8);
        assert_eq!(p_random(), 109);
        assert_eq!(p_random(), 220);
    }

    #[test]
    fn m_random_sequence_matches_original_after_clear() {
        let _guard = TEST_LOCK.lock().unwrap();
        m_clear_random();
        assert_eq!(m_random(), 8);
        assert_eq!(m_random(), 109);
    }

    #[test]
    fn index_wraps_at_256() {
        let _guard = TEST_LOCK.lock().unwrap();
        m_clear_random();
        for _ in 0..255 {
            p_random();
        }
        // prndindex is now 255 -> rndtable[255] == 249.
        // One more call wraps prndindex to 0 -> rndtable[0] == 0.
        assert_eq!(p_random(), 0);
    }
}
