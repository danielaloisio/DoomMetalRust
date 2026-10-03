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
//	Cheat sequence checking.
//
//-----------------------------------------------------------------------------

//! Rust port of `m_cheat.h` / `m_cheat.c`. Cheat code checking: a typed key
//! stream is matched against scrambled key sequences, some with
//! parameter slots (`idmus##`, `idclev##`, `idbehold`).
//!
//! # Representation
//!
//! `cheatseq_t` is `{ unsigned char* sequence; unsigned char* p; }`
//! where `p` walks `sequence` and — the odd part — *writes typed
//! parameter characters into it* (a `0` in the sequence is a parameter
//! slot; `1` marks where the parameter starts; `0xff` ends it).
//! [`CheatSeq`] owns the sequence as a `Vec<u8>` and `p` as an index
//! (`None` until first use, like the original's `NULL`), keeping
//! exactly that in-place scheme.
//!
//! The original's `firsttime` lazily-built `cheat_xlate_table` is
//! [`scramble`] evaluated on demand (it is a pure function of the key).

/// The `SCRAMBLE(a)` macro: permutes the bits of a key code.
pub fn scramble(a: u8) -> u8 {
    let a = a as u32;
    (((a & 1) << 7)
        + ((a & 2) << 5)
        + (a & 4)
        + ((a & 8) << 1)
        + ((a & 16) >> 1)
        + (a & 32)
        + ((a & 64) >> 5)
        + ((a & 128) >> 7)) as u8
}

/// (`cheatseq_t`)
#[derive(Debug, Clone)]
pub struct CheatSeq {
    pub sequence: Vec<u8>,
    p: Option<usize>,
}

impl CheatSeq {
    pub fn new(sequence: &[u8]) -> Self {
        CheatSeq {
            sequence: sequence.to_vec(),
            p: None,
        }
    }

    /// Port of `cht_CheckCheat`. Called in st_stuff module, which handles
    /// the input. Returns true if the cheat was successful, false if
    /// failed (the original's own comment, preserved).
    pub fn check(&mut self, key: u8) -> bool {
        let mut rc = false;

        let mut p = self.p.unwrap_or(0); // initialize if first time
        if self.sequence[p] == 0 {
            self.sequence[p] = key;
            p += 1;
        } else if scramble(key) == self.sequence[p] {
            p += 1;
        } else {
            p = 0;
        }

        if self.sequence[p] == 1 {
            p += 1;
        } else if self.sequence[p] == 0xff {
            // end of sequence character
            p = 0;
            rc = true;
        }
        self.p = Some(p);
        rc
    }

    /// Port of `cht_GetParam`. Returns the parameter characters typed
    /// into the sequence (the original fills a caller buffer), clearing
    /// the slots for the next use. Includes the terminating `0` the
    /// original writes when the sequence ends right after the parameter.
    pub fn get_param(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut p = 0;
        loop {
            let c = self.sequence[p];
            p += 1;
            if c == 1 {
                break;
            }
        }
        loop {
            let c = self.sequence[p];
            out.push(c);
            self.sequence[p] = 0;
            p += 1;
            if !(c != 0 && self.sequence[p] != 0xff) {
                break;
            }
        }
        if self.sequence[p] == 0xff {
            out.push(0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scrambled form of a typed string, as the sequences store it.
    fn seq_for(word: &str) -> Vec<u8> {
        let mut v: Vec<u8> = word.bytes().map(scramble).collect();
        v.push(0xff);
        v
    }

    #[test]
    fn scramble_is_a_bit_permutation_and_known_values() {
        // "iddqd" from st_stuff.c: 0xb2 0x26 0x26 0xaa 0x26
        assert_eq!(scramble(b'i'), 0xb2);
        assert_eq!(scramble(b'd'), 0x26);
        assert_eq!(scramble(b'q'), 0xaa);
        let mut seen = [false; 256];
        for i in 0..=255u8 {
            seen[scramble(i) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "bijective");
    }

    #[test]
    fn matches_only_the_full_sequence_in_order() {
        let mut c = CheatSeq::new(&[0xb2, 0x26, 0x26, 0xaa, 0x26, 0xff]);
        for k in b"iddq" {
            assert!(!c.check(*k));
        }
        assert!(c.check(b'd'), "iddqd");
        // and again (the sequence resets itself)
        for k in b"iddq" {
            assert!(!c.check(*k));
        }
        assert!(c.check(b'd'));

        // a wrong key restarts the match
        let mut c = CheatSeq::new(&seq_for("id"));
        assert!(!c.check(b'i'));
        assert!(!c.check(b'x'));
        assert!(!c.check(b'd'));
        assert!(!c.check(b'i'));
        assert!(c.check(b'd'));
    }

    #[test]
    fn parameter_slots_capture_typed_characters() {
        // idclev## from st_stuff.c
        let mut c = CheatSeq::new(&[0xb2, 0x26, 0xe2, 0x36, 0xa6, 0x6e, 1, 0, 0, 0xff]);
        for k in b"idclev" {
            assert!(!c.check(*k));
        }
        assert!(!c.check(b'1'));
        assert!(c.check(b'2'));
        assert_eq!(c.get_param(), b"12\0");
        // slots were cleared: typing a different number works
        for k in b"idclev" {
            assert!(!c.check(*k));
        }
        assert!(!c.check(b'0'));
        assert!(c.check(b'7'));
        assert_eq!(c.get_param(), b"07\0");
    }
}
