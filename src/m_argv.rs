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

//! Rust port of `m_argv.h` / `m_argv.c`.
//!
//! Command-line argument storage and lookup. The original stores
//! `myargc`/`myargv` as globals set once by `main()` (`i_main.c`) before
//! `D_DoomMain()` runs; ported here as a `OnceLock`-backed global,
//! following the plan's globals strategy (a single confined `unsafe`-free
//! entry point rather than scattering access).

use std::sync::OnceLock;

static ARGS: OnceLock<Vec<String>> = OnceLock::new();

/// Port of setting `myargc`/`myargv`. Must be called exactly once, from
/// the entry point (the `main.rs` equivalent of `i_main.c`), before any
/// call to [`m_check_parm`].
///
/// # Panics
/// Panics if called more than once, since the original globals are only
/// ever assigned a single time at process startup.
pub fn set_args(args: Vec<String>) {
    ARGS.set(args)
        .expect("m_argv::set_args called more than once");
}

fn args() -> &'static [String] {
    ARGS.get()
        .expect("m_argv::set_args must be called before use")
}

/// Port of indexing `myargv[i]` directly (e.g. `p_spec.c`'s `-timer`
/// handling: `atoi(myargv[i+1])` after an [`m_check_parm`] hit).
pub fn arg(i: usize) -> Option<&'static str> {
    args().get(i).map(String::as_str)
}

/// Port of `M_CheckParm`.
///
/// Checks for the given parameter in the program's command line
/// arguments (case-insensitively, like the original's `strcasecmp`).
/// Returns the argument index (1 to argc-1) or 0 if not present —
/// matching the original's index space, where index 0 is the program
/// name itself.
pub fn m_check_parm(check: &str) -> usize {
    let args = args();
    for (i, arg) in args.iter().enumerate().skip(1) {
        if arg.eq_ignore_ascii_case(check) {
            return i;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ARGS is a OnceLock and can only be set once per process; run all
    // tests that depend on it through a single set_args call, serialized
    // against any other test module that might touch this global.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn ensure_args() {
        let _guard = TEST_LOCK.lock().unwrap();
        if ARGS.get().is_none() {
            let _ = ARGS.set(vec![
                "doommetal-rust".to_string(),
                "-iwad".to_string(),
                "doom.wad".to_string(),
                "-DEVPARM".to_string(),
            ]);
        }
    }

    #[test]
    fn finds_existing_param_case_insensitively() {
        ensure_args();
        assert_eq!(m_check_parm("-iwad"), 1);
        assert_eq!(m_check_parm("-IWAD"), 1);
        assert_eq!(m_check_parm("-devparm"), 3);
    }

    #[test]
    fn returns_zero_for_missing_param() {
        ensure_args();
        assert_eq!(m_check_parm("-nonexistent"), 0);
    }
}
