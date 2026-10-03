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
//	System interface for sound.
//
//-----------------------------------------------------------------------------

//! Rust port of `i_sound.h` / `i_sound.c`.
//!
//! System interface for sound: a small software mixer fed to SDL2's
//! audio callback. Like the original (this is DoomMetal's SDL2
//! rewrite, not linuxdoom's sndserver), there are [`NUM_CHANNELS`]
//! mixing channels at [`SAMPLERATE`] Hz, stereo, signed 16-bit.
//!
//! # Structure
//!
//! The mixing itself ([`Mixer`]) is plain data with no SDL types, so it
//! is unit-testable without an audio device. [`ISound`] wraps a
//! `Mixer` in an `Arc<Mutex<_>>` shared with the SDL callback thread,
//! standing in for the original's `audio_mutex`.
//!
//! # Faithful quirks (deliberately NOT fixed)
//!
//! * The sample's own rate (from the lump header) is read but never
//!   used: playback advances one source byte per output frame, so 11025
//!   Hz lumps play at twice their speed against the 22050 Hz device.
//! * `pitch` and `priority` are accepted and ignored.
//! * `sample_16 = (sample_16 * volume) / 40` is computed in `int` and
//!   then stored back into a `Sint16`, so with volumes above 40 (the
//!   default sfx volume is 15, the maximum 127) the result wraps
//!   ([`Mixer::mix`] does the same `as i16`).
//! * With no free channel, [`Mixer::start_sound`] steals channel 0
//!   ("stop the oldest one" — it is just channel 0).
//!
//! # Not ported
//!
//! Music: the original's music API is all stubs ("music not
//! implemented (SDL2 only)"), ported as the same no-ops. `I_UpdateSound`,
//! `I_SubmitSound`, `I_SetChannels` are empty in the original and are
//! not ported. `I_ShutdownSound`'s device/mutex/data teardown comes
//! from `Drop`. `sndserver_filename` (only for the unused SNDSERV
//! path) is not ported.

use std::sync::{Arc, Mutex};

use sdl2::audio::{AudioCallback, AudioDevice, AudioSpecDesired};
use sdl2::AudioSubsystem;

use crate::sounds::{Sfx, NUMSFX, S_SFX};
use crate::w_wad::WadFiles;

/// Sample rate of the output device (`SAMPLERATE`).
pub const SAMPLERATE: i32 = 22050;
/// Frames per audio callback buffer (`SAMPLECOUNT`).
pub const SAMPLECOUNT: u16 = 512;
/// Number of internal mixing channels (`NUM_CHANNELS`).
pub const NUM_CHANNELS: usize = 8;

/// One mixing channel (`channel_t`). `data` is a shared handle to the
/// sound's bytes instead of the original's raw pointer.
#[derive(Clone, Default)]
struct Channel {
    data: Option<Arc<[u8]>>,
    position: usize,
    active: bool,
    looping: bool,
    volume: i32,
    separation: i32,
}

/// The mixing state: `channels[]` and `sounds[]` in the original.
pub struct Mixer {
    channels: [Channel; NUM_CHANNELS],
    /// Decoded 8-bit unsigned PCM per sfx (`sounds[i].data`); links
    /// share the linked sound's bytes.
    sounds: Vec<Option<Arc<[u8]>>>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            channels: Default::default(),
            sounds: vec![None; NUMSFX],
        }
    }

    /// Port of `I_LoadSound`. Missing lumps fall back to `dspistol`;
    /// a lump without the `03 00` DMX header leaves the sound unloaded.
    fn load_sound(&mut self, wad: &mut WadFiles, sfx_id: usize, sfxname: &str) {
        let name = format!("ds{sfxname}");
        let lump = if wad.check_num_for_name(&name).is_none() {
            wad.get_num_for_name("dspistol")
        } else {
            wad.get_num_for_name(&name)
        };

        let size = wad.lump_length(lump);
        let mut raw = vec![0u8; size];
        wad.read_lump(lump, &mut raw);

        if size < 8 || raw[0] != 0x03 || raw[1] != 0x00 {
            return;
        }

        // samplerate = raw[2] | raw[3] << 8 — read and never used, see
        // the module docs.
        let mut samples = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
        if samples > size - 8 {
            samples = size - 8;
        }
        self.sounds[sfx_id] = Some(Arc::from(&raw[8..8 + samples]));
    }

    /// The pre-cache loop of `I_InitSound`: every sfx from 1, links
    /// share their target's data.
    pub fn load_sounds(&mut self, wad: &mut WadFiles) {
        for (i, info) in S_SFX.iter().enumerate().skip(1) {
            match info.link {
                None => self.load_sound(wad, i, info.name),
                Some(link) => self.sounds[i] = self.sounds[link as usize].clone(),
            }
        }
    }

    /// Installs decoded 8-bit PCM for `id` directly (no WAD) — lets
    /// callers and tests build a mixer without a lump.
    pub fn insert_sound(&mut self, id: usize, data: &[u8]) {
        self.sounds[id] = Some(Arc::from(data));
    }

    /// Port of `I_FindFreeChannel`.
    fn find_free_channel(&self) -> usize {
        self.channels.iter().position(|c| !c.active).unwrap_or(0)
    }

    /// Port of `I_StartSound`. Returns the channel (the "handle"), or
    /// -1 for a bad id / unloaded sound.
    pub fn start_sound(&mut self, id: i32, vol: i32, sep: i32, _pitch: i32, _priority: i32) -> i32 {
        if id < 1 || id as usize >= NUMSFX {
            eprintln!("I_StartSound: invalid sound id {id}");
            return -1;
        }
        let Some(data) = self.sounds[id as usize].clone() else {
            eprintln!("I_StartSound: sound {id} not loaded");
            return -1;
        };

        let channel = self.find_free_channel();
        self.channels[channel] = Channel {
            data: Some(data),
            position: 0,
            active: true,
            looping: false,
            volume: vol,
            separation: sep,
        };
        channel as i32
    }

    /// Port of `I_StopSound`.
    pub fn stop_sound(&mut self, handle: i32) {
        if let Some(c) = self.channel_mut(handle) {
            c.active = false;
        }
    }

    /// Port of `I_SoundIsPlaying`.
    pub fn sound_is_playing(&self, handle: i32) -> bool {
        usize::try_from(handle)
            .ok()
            .and_then(|h| self.channels.get(h))
            .is_some_and(|c| c.active)
    }

    /// Port of `I_UpdateSoundParams` (pitch ignored, as in the original).
    pub fn update_sound_params(&mut self, handle: i32, vol: i32, sep: i32, _pitch: i32) {
        if let Some(c) = self.channel_mut(handle) {
            if c.active {
                c.volume = vol;
                c.separation = sep;
            }
        }
    }

    fn channel_mut(&mut self, handle: i32) -> Option<&mut Channel> {
        usize::try_from(handle)
            .ok()
            .and_then(|h| self.channels.get_mut(h))
    }

    /// Port of `I_AudioCallback`'s body: mixes into `output`, which is
    /// interleaved stereo (`output.len() / 2` frames). Overwrites it.
    pub fn mix(&mut self, output: &mut [i16]) {
        output.fill(0);
        let frames = output.len() / 2;

        for chan in self.channels.iter_mut() {
            if !chan.active {
                continue;
            }
            let Some(data) = chan.data.as_ref() else {
                continue;
            };

            let mut i = 0;
            while i < frames && chan.position < data.len() {
                let sample = data[chan.position] as i32;
                let mut sample_16 = ((sample - 128) << 8) as i16;
                // Stored back into a Sint16: wraps for volume > 40.
                sample_16 = ((sample_16 as i32 * chan.volume) / 40) as i16;

                let left_vol = 255 - chan.separation;
                let right_vol = chan.separation;

                let left = output[i * 2] as i32 + (sample_16 as i32 * left_vol) / 255;
                let right = output[i * 2 + 1] as i32 + (sample_16 as i32 * right_vol) / 255;

                output[i * 2] = left.clamp(-32768, 32767) as i16;
                output[i * 2 + 1] = right.clamp(-32768, 32767) as i16;

                chan.position += 1;
                i += 1;
            }

            if chan.position >= data.len() {
                if chan.looping {
                    chan.position = 0;
                } else {
                    chan.active = false;
                }
            }
        }
    }
}

/// SDL callback: locks the shared mixer and fills the buffer.
struct MixerCallback(Arc<Mutex<Mixer>>);

impl AudioCallback for MixerCallback {
    type Channel = i16;

    fn callback(&mut self, out: &mut [i16]) {
        match self.0.lock() {
            Ok(mut mixer) => mixer.mix(out),
            Err(_) => out.fill(0),
        }
    }
}

/// The sound system: mixer plus (when opened) the SDL audio device.
///
/// Like the original, if the device can't be opened `I_InitSound` just
/// reports on stderr and every later call is a silent no-op
/// (`enabled == false`: `start_sound` returns -1, `sound_is_playing`
/// false).
pub struct ISound {
    mixer: Arc<Mutex<Mixer>>,
    _device: Option<AudioDevice<MixerCallback>>,
    enabled: bool,
    music_volume: i32,
}

impl ISound {
    /// A sound system with no device — what the original is left with
    /// when `I_InitSound` fails (or before it runs).
    pub fn disabled() -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new())),
            _device: None,
            enabled: false,
            music_volume: 15,
        }
    }

    /// Port of `I_InitSound`: opens the device, pre-caches every sfx
    /// from the WAD, unpauses.
    pub fn init(audio: &AudioSubsystem, wad: &mut WadFiles) -> Self {
        eprint!("I_InitSound: ");

        let mut mixer = Mixer::new();
        mixer.load_sounds(wad);
        let mixer = Arc::new(Mutex::new(mixer));

        let desired = AudioSpecDesired {
            freq: Some(SAMPLERATE),
            channels: Some(2),
            samples: Some(SAMPLECOUNT),
        };
        let cb_mixer = Arc::clone(&mixer);
        let device = match audio.open_playback(None, &desired, |spec| {
            eprintln!(
                "configured audio device (freq={}, channels={}, samples={})",
                spec.freq, spec.channels, spec.samples
            );
            MixerCallback(cb_mixer)
        }) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("SDL_OpenAudioDevice failed: {e}");
                return Self::disabled();
            }
        };

        eprintln!("I_InitSound: pre-cached all sound data");
        device.resume();
        eprintln!("I_InitSound: sound module ready");

        Self {
            mixer,
            _device: Some(device),
            enabled: true,
            music_volume: 15,
        }
    }

    /// An enabled sound system around `mixer` with no SDL device: calls
    /// take effect on the mixer (which nothing drains). For tests and
    /// headless runs of the `s_sound` logic.
    pub fn headless(mixer: Mixer) -> Self {
        Self {
            mixer: Arc::new(Mutex::new(mixer)),
            _device: None,
            enabled: true,
            music_volume: 15,
        }
    }

    fn with_mixer<R>(&self, default: R, f: impl FnOnce(&mut Mixer) -> R) -> R {
        if !self.enabled {
            return default;
        }
        match self.mixer.lock() {
            Ok(mut m) => f(&mut m),
            Err(_) => default,
        }
    }

    /// Port of `I_StartSound` (without the unused `priority`/`pitch`
    /// semantics, see module docs).
    pub fn start_sound(&self, id: Sfx, vol: i32, sep: i32, pitch: i32, priority: i32) -> i32 {
        self.start_sound_id(id as i32, vol, sep, pitch, priority)
    }

    /// `I_StartSound` with a raw id (so `s_sound` can pass through the
    /// original's range check).
    pub fn start_sound_id(&self, id: i32, vol: i32, sep: i32, pitch: i32, priority: i32) -> i32 {
        self.with_mixer(-1, |m| m.start_sound(id, vol, sep, pitch, priority))
    }

    /// Port of `I_StopSound`.
    pub fn stop_sound(&self, handle: i32) {
        self.with_mixer((), |m| m.stop_sound(handle));
    }

    /// Port of `I_SoundIsPlaying`.
    pub fn sound_is_playing(&self, handle: i32) -> bool {
        self.with_mixer(false, |m| m.sound_is_playing(handle))
    }

    /// Port of `I_UpdateSoundParams`.
    pub fn update_sound_params(&self, handle: i32, vol: i32, sep: i32, pitch: i32) {
        self.with_mixer((), |m| m.update_sound_params(handle, vol, sep, pitch));
    }

    /// Port of `I_SetSfxVolume`: only sets the global `snd_SfxVolume`.
    pub fn set_sfx_volume(&self, volume: i32) {
        crate::doomstat::state_mut().snd_sfx_volume = volume;
    }

    /// Port of `I_GetSfxLumpNum`.
    pub fn get_sfx_lump_num(wad: &WadFiles, sfx: Sfx) -> usize {
        wad.get_num_for_name(&format!("ds{}", S_SFX[sfx as usize].name))
    }

    // ---- MUSIC API: all stubs in the original ----

    /// `I_InitMusic`.
    pub fn init_music(&self) {
        eprintln!("I_InitMusic: music not implemented (SDL2 only)");
    }

    /// `I_SetMusicVolume` (stores the value, nothing plays).
    pub fn set_music_volume(&mut self, volume: i32) {
        self.music_volume = volume;
    }

    /// `I_PlaySong` (no-op).
    pub fn play_song(&self, _handle: i32, _looping: bool) {}

    /// `I_PauseSong` (no-op).
    pub fn pause_song(&self, _handle: i32) {}

    /// `I_ResumeSong` (no-op).
    pub fn resume_song(&self, _handle: i32) {}

    /// `I_StopSong` (no-op).
    pub fn stop_song(&self, _handle: i32) {}

    /// `I_UnRegisterSong` (no-op).
    pub fn unregister_song(&self, _handle: i32) {}

    /// `I_RegisterSong` (always handle 0).
    pub fn register_song(&self, _data: &[u8]) -> i32 {
        0
    }

    /// `I_QrySongPlaying` (never playing).
    pub fn qry_song_playing(&self, _handle: i32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixer_with(id: usize, data: &[u8]) -> Mixer {
        let mut m = Mixer::new();
        m.sounds[id] = Some(Arc::from(data));
        m
    }

    #[test]
    fn start_rejects_bad_and_unloaded_ids() {
        let mut m = mixer_with(1, &[128; 4]);
        assert_eq!(m.start_sound(0, 15, 128, 128, 64), -1);
        assert_eq!(m.start_sound(NUMSFX as i32, 15, 128, 128, 64), -1);
        assert_eq!(m.start_sound(2, 15, 128, 128, 64), -1);
        assert_eq!(m.start_sound(1, 15, 128, 128, 64), 0);
        assert!(m.sound_is_playing(0));
    }

    #[test]
    fn mix_applies_volume_and_separation_like_the_original() {
        // 255 -> (127 << 8) = 32512; *15/40 = 12192; sep 128:
        // left 12192*127/255 = 6072, right 12192*128/255 = 6119.
        let mut m = mixer_with(1, &[255]);
        m.start_sound(1, 15, 128, 128, 64);
        let mut out = [0i16; 4];
        m.mix(&mut out);
        assert_eq!(out[0], 6072);
        assert_eq!(out[1], 6119);
        // one source byte only: second frame silent, channel freed
        assert_eq!((out[2], out[3]), (0, 0));
        assert!(!m.sound_is_playing(0));
    }

    #[test]
    fn high_volume_wraps_in_sint16_like_the_original() {
        // 32512 * 127 / 40 = 103_225 -> as i16 wraps to 103225 - 131072.
        let mut m = mixer_with(1, &[255]);
        m.start_sound(1, 127, 255, 128, 64);
        let mut out = [0i16; 2];
        m.mix(&mut out);
        let wrapped = (103_225i32 as i16) as i32;
        assert_eq!(out[1] as i32, wrapped);
        assert_eq!(out[0], 0); // separation 255 -> left volume 0
    }

    #[test]
    fn two_channels_sum_and_clip() {
        let mut m = mixer_with(1, &[255]);
        for _ in 0..2 {
            m.start_sound(1, 40, 255, 128, 64);
        }
        let mut out = [0i16; 2];
        m.mix(&mut out);
        // each contributes 32512 to the right; sum clips to 32767.
        assert_eq!(out[1], 32767);
    }

    #[test]
    fn full_table_steals_channel_zero() {
        let mut m = mixer_with(1, &[128; 100]);
        for expected in 0..NUM_CHANNELS as i32 {
            assert_eq!(m.start_sound(1, 15, 128, 128, 64), expected);
        }
        assert_eq!(m.start_sound(1, 15, 128, 128, 64), 0);
    }

    #[test]
    fn stop_and_update_ignore_bad_handles() {
        let mut m = mixer_with(1, &[128; 10]);
        m.stop_sound(-1);
        m.stop_sound(99);
        m.update_sound_params(-1, 1, 1, 1);
        let h = m.start_sound(1, 15, 128, 128, 64);
        m.update_sound_params(h, 20, 10, 0);
        assert_eq!(m.channels[h as usize].volume, 20);
        m.stop_sound(h);
        assert!(!m.sound_is_playing(h));
        // no effect on an inactive channel
        m.update_sound_params(h, 99, 99, 0);
        assert_eq!(m.channels[h as usize].volume, 20);
    }

    #[test]
    fn disabled_system_is_silent() {
        let s = ISound::disabled();
        assert_eq!(s.start_sound(Sfx::SfxPistol, 15, 128, 128, 64), -1);
        assert!(!s.sound_is_playing(0));
    }

    #[test]
    fn real_wad_loads_all_sounds_and_links_share_data() {
        let Some(path) = ["doom.wad"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())
        else {
            eprintln!("skipping: no doom.wad found");
            return;
        };
        let mut wad = WadFiles::new();
        wad.init_file(path);
        let mut m = Mixer::new();
        m.load_sounds(&mut wad);
        let pistol = m.sounds[Sfx::SfxPistol as usize].clone().unwrap();
        assert!(pistol.len() > 100);
        let chgun = m.sounds[Sfx::SfxChgun as usize].clone().unwrap();
        assert!(Arc::ptr_eq(&pistol, &chgun));
        assert!(m.sounds[Sfx::SfxShotgn as usize].is_some());
    }
}
