//! End-to-end sound: real WAD lumps -> real SDL audio device (the
//! `dummy` driver, so no sound card is needed) -> queued `S_StartSound`
//! calls -> mixer callback draining the channel.

mod common;

use doommetal_rust::doomdef::GameMode;
use doommetal_rust::doomstat;
use doommetal_rust::i_sound::ISound;
use doommetal_rust::p_setup::Level;
use doommetal_rust::p_tick::Thinkers;
use doommetal_rust::s_sound::{self, SoundCtx};
use doommetal_rust::sounds::{MusicEnum, Sfx};
use doommetal_rust::w_wad::WadFiles;

#[test]
fn a_started_sound_is_mixed_by_the_device_and_frees_its_channel() {
    let Some(path) = common::find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    // Single test in this binary, so touching the environment is safe.
    std::env::set_var("SDL_AUDIODRIVER", "dummy");

    let mut wad = WadFiles::new();
    wad.init_file(path);

    let sdl = sdl2::init().expect("SDL init");
    let audio = sdl.audio().expect("SDL audio (dummy driver)");
    let isound = ISound::init(&audio, &mut wad);
    s_sound::s_init(isound, 15, 15);

    // S_Start: level music selection for E1M1 finds the real d_e1m1 lump.
    {
        let st = doomstat::state_mut();
        st.gamemode = GameMode::Registered;
        st.gameepisode = 1;
        st.gamemap = 1;
    }
    s_sound::s_start(&mut wad);
    assert_eq!(
        s_sound::s_music_playing(),
        Some(MusicEnum::MusE1m1 as usize)
    );

    let thinkers = Thinkers::new();
    let level = Level::default();
    let ctx = SoundCtx {
        thinkers: &thinkers,
        level: &level,
        listener: None,
    };

    // Nothing runs until the queue is flushed.
    s_sound::s_start_sound(None, Sfx::SfxPistol);
    assert_eq!(s_sound::s_channels_in_use(), 0);
    s_sound::s_update_sounds(&ctx);
    assert_eq!(s_sound::s_channels_in_use(), 1);

    // The device thread consumes the samples; the channel frees itself.
    let mut freed = false;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        s_sound::s_update_sounds(&ctx);
        if s_sound::s_channels_in_use() == 0 {
            freed = true;
            break;
        }
    }
    assert!(freed, "pistol sound never finished playing");

    s_sound::s_shutdown();
}
