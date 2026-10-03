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
//	Main program, simply calls D_DoomMain high level loop.
//
//-----------------------------------------------------------------------------

//! Rust port of `i_main.c`, plus a trimmed `D_DoomMain`/`D_DoomLoop`.
//!
//! The original's `main` is a thin shim around `D_DoomMain`. Here it loads
//! the config (`M_LoadDefaults`), opens the IWAD, brings up the
//! renderer/video/sound/status bar/HUD/menu, starts the title loop (or a
//! game, with `-warp`/`-skill`/`-episode`) and runs the `TryRunTics` loop
//! through `d_main::MainHost`.
//!
//! The WAD path is the first non-flag argument, falling back to
//! `doom.wad`.
//!
//! The engine modules live in `lib.rs`; this binary re-exposes them via
//! `doommetal_rust::*`.

use doommetal_rust::d_main::{
    d_display_world, d_start_title, identify_version, MainHost, StartArgs,
};
use doommetal_rust::d_net::{NetGame, NetStart};
use doommetal_rust::doomstat;
use doommetal_rust::g_game::{self, GameWorld};
use doommetal_rust::i_sound::ISound;
use doommetal_rust::i_video::IVideo;
use doommetal_rust::m_argv;
use doommetal_rust::r_main::Renderer;
use doommetal_rust::s_sound::{self, SoundCtx};
use doommetal_rust::w_wad::WadFiles;

fn wad_path() -> std::path::PathBuf {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(path) = args.iter().find(|a| !a.starts_with('-')) {
        return std::path::PathBuf::from(path);
    }
    // Same default the tests/real_*.rs integration tests fall back to.
    std::path::PathBuf::from("doom.wad")
}

fn has_arg(args: &[String], name: &str) -> bool {
    args.iter().skip(1).any(|a| a.eq_ignore_ascii_case(name))
}

/// FNV-1a over what a netgame must keep identical on every machine: each
/// player in the game (position, health, ammo) and the P_Random index.
fn world_hash(world: &GameWorld) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut put = |v: i64| {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let ingame = doomstat::state().playeringame;
    for (i, p) in world.players.iter().enumerate() {
        if !ingame.get(i).copied().unwrap_or(false) {
            continue;
        }
        if let Some(m) = world.thinkers.mobj(p.mo) {
            put(m.x as i64);
            put(m.y as i64);
            put(m.z as i64);
            put(m.angle as i64);
        }
        put(p.health as i64);
        put(p.ammo[0] as i64);
    }
    put(doommetal_rust::m_random::p_rnd_index() as i64);
    h
}

/// The fields behind [`world_hash`], for finding where two machines
/// parted ways.
fn world_detail(world: &GameWorld) -> String {
    let ingame = doomstat::state().playeringame;
    let mut out = String::new();
    for (i, p) in world.players.iter().enumerate() {
        if !ingame.get(i).copied().unwrap_or(false) {
            continue;
        }
        if let Some(m) = world.thinkers.mobj(p.mo) {
            out += &format!(
                "p{i}[x{} y{} z{} a{} hp{} ammo{} st{:?}] ",
                m.x, m.y, m.z, m.angle, p.health, p.ammo[0], p.playerstate
            );
        }
    }
    out + &format!("rnd{}", doommetal_rust::m_random::p_rnd_index())
}

fn main() {
    m_argv::set_args(std::env::args().collect());

    let path = wad_path();
    if !path.exists() {
        eprintln!(
            "DoomMetalRust: no IWAD found at {} (pass a path as the first argument)",
            path.display()
        );
        std::process::exit(1);
    }

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    // M_LoadDefaults: the config file ($HOME/.doomrc or `-config`) over
    // the table defaults — volumes, screen size, mouse sensitivity, ...
    let config_path = doommetal_rust::m_misc::config_path();
    let defaults = doommetal_rust::m_misc::m_load_defaults(&config_path);
    let usegamma = defaults.apply();

    // R_Init: `screenblocks` (default 9: a bordered view above the
    // status bar), high detail (low detail isn't supported).
    let screenblocks = doomstat::state().screenblocks.clamp(3, 11);
    doomstat::state_mut().screenblocks = screenblocks;
    let mut renderer = Renderer::r_init(&mut wad, screenblocks, 0);
    let sprnames = doommetal_rust::info::SPRNAMES;
    renderer
        .rthings
        .r_init_sprites(&wad, &renderer.rdata, &sprnames, false);

    // IdentifyVersion (from the IWAD's lumps, see d_main's docs)
    let gamemode = identify_version(&wad);
    doomstat::state_mut().gamemode = gamemode;

    // I_InitGraphics / I_InitSound / S_Init — sound has to be up before
    // the level loads, since `S_Start` (called during level setup)
    // starts the level's music.
    let mut video = IVideo::i_init_graphics();
    video.v_video.usegamma = usegamma;
    let isound = match video.audio_subsystem() {
        Ok(audio) => ISound::init(&audio, &mut wad),
        Err(e) => {
            eprintln!("I_InitSound: SDL audio unavailable: {e}");
            ISound::disabled()
        }
    };
    let (sfx_volume, music_volume) = {
        let st = doomstat::state();
        (st.snd_sfx_volume, st.snd_music_volume)
    };
    s_sound::s_init(isound, sfx_volume, music_volume);

    // ST_Init: status bar graphics + screens[4]; HU_Init: the heads-up
    // font; M_Init: the menu.
    doommetal_rust::st_stuff::st_init(&mut wad, &mut video.v_video);
    doommetal_rust::hu_stuff::hu_init(&mut wad);
    doommetal_rust::m_menu::m_init();

    let playpal = wad
        .cache_lump_name("PLAYPAL", doommetal_rust::z_zone::PurgeTag::Cache)
        .to_vec();
    video.i_set_palette(&playpal);

    let mut world = GameWorld::new(wad, renderer);

    // -skill / -episode / -warp start a game directly; otherwise the
    // attract loop begins at the title (D_DoomMain's tail).
    let args: Vec<String> = std::env::args().collect();
    let start = StartArgs::parse(&args, gamemode);
    {
        let has = |n: &str| args.iter().skip(1).any(|a| a.eq_ignore_ascii_case(n));
        let st = doomstat::state_mut();
        st.nomonsters = has("-nomonsters");
        st.respawnparm = has("-respawn");
        st.fastparm = has("-fast");
    }
    // D_CheckNetGame: players and settings (single player without -net).
    let (netinfo, transport) = match doommetal_rust::i_net::i_init_network(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("I_InitNetwork: {e}");
            std::process::exit(1);
        }
    };
    let mut net = NetGame::new(netinfo, transport);
    let mut netstart = NetStart {
        skill: start.skill as i32,
        deathmatch: if has_arg(&args, "-altdeath") {
            2
        } else {
            i32::from(has_arg(&args, "-deathmatch"))
        },
        nomonsters: has_arg(&args, "-nomonsters"),
        respawn: has_arg(&args, "-respawn"),
        map: start.map,
        episode: start.episode,
    };
    {
        let mut host = MainHost {
            world: &mut world,
            video: &mut video,
            quit: false,
        };
        if net.info().netgame && net.info().consoleplayer != 0 {
            eprintln!("listening for network start info...");
        } else if net.info().netgame {
            eprintln!("sending network start info...");
        }
        net.d_check_net_game(&mut host, &mut netstart);
    }
    eprintln!(
        "startskill {}  deathmatch: {}  startmap: {}  startepisode: {}",
        netstart.skill, netstart.deathmatch, netstart.map, netstart.episode
    );
    eprintln!(
        "player {} of {} ({} nodes)",
        net.info().consoleplayer + 1,
        net.info().numplayers,
        net.info().numnodes
    );
    doomstat::state_mut().fastparm = has_arg(&args, "-fast");
    world.consistency_check = net.info().netgame;

    if start.autostart || net.info().netgame {
        let skill = doommetal_rust::doomdef::Skill::from_index(netstart.skill as usize)
            .unwrap_or(start.skill);
        g_game::g_init_new(&mut world, skill, netstart.episode, netstart.map);
    } else {
        d_start_title();
    }

    doomstat::state_mut().gametic = 0;

    // Diagnostics (this port's own, used by tests/real_netgame.rs):
    // `-statehash N` prints a hash of the world every N tics, `-maxtics N`
    // quits after N tics.
    let arg_value = |name: &str| -> Option<i32> {
        args.iter()
            .position(|a| a.eq_ignore_ascii_case(name))
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
    };
    let statehash_every = arg_value("-statehash");
    let maxtics = arg_value("-maxtics");
    let mut last_hashed = -1;

    // D_DoomLoop: TryRunTics runs the tics the network allows (at least
    // one), then the frame is drawn.
    loop {
        video.i_start_frame();

        let quit = {
            let mut host = MainHost {
                world: &mut world,
                video: &mut video,
                quit: false,
            };
            net.try_run_tics(&mut host);
            host.quit
        };
        if quit || video.should_quit() {
            break;
        }

        let gametic = doomstat::state().gametic;
        if let Some(n) = statehash_every {
            if n > 0 && gametic % n == 0 && gametic != last_hashed {
                last_hashed = gametic;
                println!(
                    "STATE tic {gametic} {:016x} {}",
                    world_hash(&world),
                    world_detail(&world)
                );
            }
        }
        if maxtics.is_some_and(|m| gametic >= m) {
            break;
        }

        // S_UpdateSounds(players[consoleplayer].mo): runs the sound
        // calls queued during this tic, then moves positional sounds.
        s_sound::s_update_sounds(&SoundCtx {
            thinkers: &world.thinkers,
            level: &world.level,
            listener: world
                .players
                .get(doomstat::state().displayplayer as usize)
                .map(|p| p.mo),
        });

        // I_Quit, from the menu's quit confirmation: the quit sound was
        // just started above; give it a moment, like M_QuitResponse's
        // I_WaitVBL(105), then leave.
        if doomstat::state().quit_requested {
            doommetal_rust::i_system::i_wait_vbl(105);
            break;
        }

        d_display_world(&mut world, &mut video);
    }

    net.d_quit_net_game(doomstat::state().usergame);

    // M_SaveDefaults (I_Quit's last act): screen size, volumes, ...
    doommetal_rust::m_misc::m_save_defaults(
        &config_path,
        &doommetal_rust::m_misc::Defaults::capture(video.v_video.usegamma),
    );
}
