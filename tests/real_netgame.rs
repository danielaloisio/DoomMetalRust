//! Phase 11d: two real game processes over UDP on localhost, in
//! deathmatch, driven by the world-state hash the binary prints
//! (`-statehash`): both machines must show the very same hash at the
//! same tic — the lockstep guarantee the whole netcode exists for.

mod common;

use std::collections::BTreeMap;
use std::process::{Command, Stdio};

fn hashes(out: &str) -> BTreeMap<i32, String> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.strip_prefix("STATE tic ")?.split_whitespace();
            Some((it.next()?.parse().ok()?, it.collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

#[test]
fn two_processes_stay_in_sync_over_udp() {
    let Some(wad) = common::find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let exe = env!("CARGO_BIN_EXE_doommetal-rust");
    let (port1, port2) = (47311, 47312);

    let spawn = |me: &str, mine: u16, theirs: u16, extra: &[&str]| {
        Command::new(exe)
            .arg(&wad)
            .args(["-net", me, &format!("127.0.0.1:{theirs}")])
            .args(["-port", &mine.to_string()])
            .args(["-statehash", "35", "-maxtics", "215"])
            .args(extra)
            .env("SDL_VIDEODRIVER", "dummy")
            .env("SDL_AUDIODRIVER", "dummy")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the game")
    };

    // player 1 is the key player: it sets the game up, player 2 learns
    // everything from it (no game arguments of its own)
    let p1 = spawn(
        "1",
        port1,
        port2,
        &["-deathmatch", "-skill", "3", "-warp", "1", "3"],
    );
    std::thread::sleep(std::time::Duration::from_millis(500));
    let p2 = spawn("2", port2, port1, &[]);

    let o1 = p1.wait_with_output().unwrap();
    let o2 = p2.wait_with_output().unwrap();
    let (out1, out2) = (
        String::from_utf8_lossy(&o1.stdout).to_string(),
        String::from_utf8_lossy(&o2.stdout).to_string(),
    );
    let (err1, err2) = (
        String::from_utf8_lossy(&o1.stderr).to_string(),
        String::from_utf8_lossy(&o2.stderr).to_string(),
    );
    assert!(o1.status.success(), "player 1 exited badly:\n{err1}");
    assert!(o2.status.success(), "player 2 exited badly:\n{err2}");
    assert!(
        err2.contains("deathmatch: 1  startmap: 3"),
        "p2 got the key player's settings:\n{err2}"
    );
    assert!(err2.contains("player 2 of 2"), "{err2}");

    let (h1, h2) = (hashes(&out1), hashes(&out2));
    assert!(h1.len() >= 4, "player 1 hashed {} tics\n{out1}", h1.len());
    let common: Vec<_> = h1.keys().filter(|t| h2.contains_key(t)).collect();
    assert!(
        common.len() >= 4,
        "only {} common hashed tics",
        common.len()
    );
    for t in common {
        assert_eq!(h1[t], h2[t], "the two machines diverged at tic {t}");
    }
}
