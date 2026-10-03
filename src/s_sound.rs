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
// DESCRIPTION:  none
//
//-----------------------------------------------------------------------------

//! Rust port of `s_sound.h` / `s_sound.c`.
//!
//! The sound logic layer: which sounds play on which of the
//! `numChannels` logical channels, distance attenuation and stereo
//! panning against the console player, priority-based channel stealing,
//! and the music selection bookkeeping (music itself is a no-op stub
//! underneath, see `i_sound`).
//!
//! # Design
//!
//! * **State.** The original's file-scope statics (`channels`,
//!   `mus_paused`, `mus_playing`, the per-sfx `usefulness`, ...) live in
//!   one [`SoundState`] behind a `thread_local!`, so the ~90 call sites
//!   in the game code can call [`s_start_sound`] without threading a
//!   sound system through every signature. The engine is
//!   single-threaded; using a *thread*-local rather than a process
//!   global additionally keeps `cargo test`'s parallel test threads from
//!   sharing (and racing on) sound state. Until [`s_init`] runs (tests,
//!   headless tools) every entry point is a no-op.
//! * **`void* origin`.** The original passes `mobj_t*` or
//!   `&sector->soundorg` as an untyped pointer and compares pointers for
//!   identity. That is [`SoundOrigin`] here: a mobj's [`ThinkerId`] or a
//!   sector index. `NULL` is `None`. As in C, `S_StopSound(NULL)` stops
//!   a channel whose origin is also `NULL`.
//! * **Deferred calls.** The ~150 `S_StartSound`/`S_StopSound` call
//!   sites in the game code sit deep inside functions that hold
//!   `&mut Thinkers`/`&mut Level` (and often a `&mut Mobj` borrowed from
//!   them), with no way to also hand each a [`SoundCtx`]. So
//!   [`s_start_sound`]/[`s_stop_sound`] just queue the call and
//!   [`s_flush`] executes the queue, in order, once per tic after
//!   `G_Ticker`, with the world in hand. Channel allocation, stealing,
//!   `M_Random` pitch draws (its own RNG stream, unrelated to
//!   `P_Random`) and stops therefore happen in the same order as in the
//!   original; what changes is that positions are read at the *end* of
//!   the tic instead of at the call, and a sound whose origin mobj was
//!   freed within the same tic is dropped rather than played.
//! * **World access.** Positions are read at call time from a
//!   [`SoundCtx`] (thinkers + level + the console player's mobj), the
//!   stand-in for `players[consoleplayer].mo` and the pointer
//!   dereferences.
//! * **`R_PointToAngle2`.** Called through
//!   [`crate::r_main::angle_from_delta`], i.e. without the original's
//!   side effect of overwriting `viewx`/`viewy` (every frame's
//!   `R_SetupFrame` resets them anyway).
//!
//! # Divergences
//!
//! * `S_sfx[].lumpnum` (cached by `I_GetSfxLumpNum`) is never read by
//!   anything — `I_InitSound` already pre-cached every sfx — so it is
//!   not kept. `usefulness` is kept for parity although nothing reads
//!   it either (its cleanup pass is commented out in the original).
//! * A sound origin whose mobj has since been removed would be a dangling
//!   pointer in C; here its position can't be resolved, so
//!   [`s_update_sounds`] stops that channel and [`s_start_sound`] plays
//!   the sound unpositioned instead of reading freed memory.
//! * The `SAWDEBUG`/`SNDSRV` `#ifdef` code is not ported.
//! * `S_sfx`'s `singularity` isn't consulted, matching this version of
//!   `S_getChannel`.

use std::cell::RefCell;

use crate::doomdef::GameMode;
use crate::doomstat;
use crate::i_sound::ISound;
use crate::m_fixed::{fixed_mul, Fixed, FRACBITS};
use crate::m_random::m_random;
use crate::p_setup::Level;
use crate::p_tick::{ThinkerId, Thinkers};
use crate::r_main::angle_from_delta;
use crate::sounds::{MusicEnum, Sfx, NUMMUSIC, NUMSFX, S_MUSIC, S_SFX};
use crate::tables::{Angle, ANGLETOFINESHIFT, FINESINE};
use crate::w_wad::WadFiles;
use crate::z_zone::PurgeTag;

/// (`S_MAX_VOLUME`)
pub const S_MAX_VOLUME: i32 = 127;
/// Distance beyond which sounds are inaudible (`S_CLIPPING_DIST`).
pub const S_CLIPPING_DIST: i32 = 1200 * 0x10000;
/// Distance within which sounds play at full volume (`S_CLOSE_DIST`).
pub const S_CLOSE_DIST: i32 = 160 * 0x10000;
/// (`S_ATTENUATOR`)
pub const S_ATTENUATOR: i32 = (S_CLIPPING_DIST - S_CLOSE_DIST) >> FRACBITS;
/// (`NORM_PITCH`)
pub const NORM_PITCH: i32 = 128;
/// (`NORM_PRIORITY`)
pub const NORM_PRIORITY: i32 = 64;
/// (`NORM_SEP`)
pub const NORM_SEP: i32 = 128;
/// (`S_STEREO_SWING`)
pub const S_STEREO_SWING: i32 = 96 * 0x10000;

/// What a sound is attached to: the original's `void* origin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundOrigin {
    /// A `mobj_t*`.
    Mobj(ThinkerId),
    /// `&sectors[n].soundorg` (doors, floors, plats, ceilings, switches).
    Sector(usize),
}

/// `(mobj_t *)&sectors[n].soundorg` as an origin argument.
pub fn sector_origin(n: usize) -> Option<SoundOrigin> {
    Some(SoundOrigin::Sector(n))
}

/// Read access to the world for one sound call.
#[derive(Clone, Copy)]
pub struct SoundCtx<'a> {
    pub thinkers: &'a Thinkers,
    pub level: &'a Level,
    /// `players[consoleplayer].mo`.
    pub listener: Option<ThinkerId>,
}

#[derive(Clone, Copy)]
struct Listener {
    x: Fixed,
    y: Fixed,
    angle: Angle,
}

impl SoundCtx<'_> {
    fn origin_pos(&self, origin: SoundOrigin) -> Option<(Fixed, Fixed)> {
        match origin {
            SoundOrigin::Mobj(id) => self.thinkers.mobj(id).map(|m| (m.x, m.y)),
            SoundOrigin::Sector(n) => self
                .level
                .sectors
                .get(n)
                .map(|s| (s.soundorg.x, s.soundorg.y)),
        }
    }

    fn listener_pos(&self) -> Option<Listener> {
        let m = self.thinkers.mobj(self.listener?)?;
        Some(Listener {
            x: m.x,
            y: m.y,
            angle: m.angle,
        })
    }
}

/// `channel_t`.
#[derive(Clone, Copy, Default)]
struct Channel {
    /// `sfxinfo` (`None` = channel available).
    sfx: Option<usize>,
    origin: Option<SoundOrigin>,
    /// Handle of the sound being played (the mixer channel).
    handle: i32,
}

/// The file-scope statics of `s_sound.c`.
struct SoundState {
    initialized: bool,
    isound: ISound,
    channels: Vec<Channel>,
    /// `S_sfx[i].usefulness`.
    usefulness: Vec<i32>,
    mus_paused: bool,
    /// Index of the music playing (`mus_playing`).
    mus_playing: Option<usize>,
    /// `S_music[i].lumpnum` (0 = not looked up yet, as in the original).
    mus_lumpnum: Vec<usize>,
    /// `S_music[i].handle`.
    mus_handle: Vec<i32>,
    /// Deferred calls, see [`s_flush`].
    pending: Vec<SoundCmd>,
}

impl SoundState {
    fn new() -> Self {
        Self {
            initialized: false,
            isound: ISound::disabled(),
            channels: Vec::new(),
            usefulness: vec![-1; NUMSFX],
            mus_paused: false,
            mus_playing: None,
            mus_lumpnum: vec![0; NUMMUSIC],
            mus_handle: vec![0; NUMMUSIC],
            pending: Vec::new(),
        }
    }
}

thread_local! {
    static SOUND: RefCell<SoundState> = RefCell::new(SoundState::new());
}

fn with<R>(f: impl FnOnce(&mut SoundState) -> R) -> R {
    SOUND.with(|s| f(&mut s.borrow_mut()))
}

fn sfx_volume() -> i32 {
    doomstat::state().snd_sfx_volume
}

/// Port of `S_Init`: sets volumes, allocates the logical channels and
/// takes ownership of the (already `I_InitSound`ed) [`ISound`].
pub fn s_init(isound: ISound, sfx_volume: i32, music_volume: i32) {
    eprintln!("S_Init: default sfx volume {sfx_volume}");

    // I_SetChannels(): a dummy in the original.
    with(|s| s.isound = isound);
    s_set_sfx_volume(sfx_volume);
    // No music: another dummy.
    s_set_music_volume(music_volume);

    let n = doomstat::state().num_channels.max(0) as usize;
    with(|s| {
        s.channels = vec![Channel::default(); n];
        s.mus_paused = false;
        // Note that sounds have not been cached (yet).
        s.usefulness.iter_mut().for_each(|u| *u = -1);
        s.initialized = true;
    });
}

/// `I_ShutdownSound`: drops the sound system (closing the device) and
/// returns to the uninitialized, all-no-op state.
pub fn s_shutdown() {
    with(|s| *s = SoundState::new());
}

/// Port of `S_Start`: called at level start — stops everything and
/// starts the level's music.
pub fn s_start(wad: &mut WadFiles) {
    if !with(|s| s.initialized) {
        return;
    }

    // kill all playing sounds at start of level (trust me - a good idea)
    for cnum in 0..with(|s| s.channels.len()) {
        if with(|s| s.channels[cnum].sfx.is_some()) {
            s_stop_channel(cnum);
        }
    }

    // Calls queued against the previous level's mobjs are stale.
    with(|s| s.pending.clear());

    // start new music for the level
    with(|s| s.mus_paused = false);

    let st = doomstat::state();
    let mnum = music_for_level(st.gamemode, st.gameepisode, st.gamemap);

    s_change_music(wad, mnum, true);
}

/// Port of `S_StartSound`.
fn start_sound_now(ctx: &SoundCtx, origin: Option<SoundOrigin>, sfx: Sfx) {
    start_sound_at_volume_now(ctx, origin, sfx as i32, sfx_volume());
}

/// Port of `S_StartSoundAtVolume`.
///
/// # Panics
/// On a bad sfx number (the original's `I_Error("Bad sfx #: %d")`),
/// including the original's off-by-one acceptance of `id == NUMSFX`,
/// which then indexes out of range.
fn start_sound_at_volume_now(
    ctx: &SoundCtx,
    origin: Option<SoundOrigin>,
    sfx_id: i32,
    mut volume: i32,
) {
    if !with(|s| s.initialized) {
        return;
    }
    if sfx_id < 1 || sfx_id > NUMSFX as i32 {
        panic!("Bad sfx #: {sfx_id}");
    }

    let info = &S_SFX[sfx_id as usize];
    let (mut pitch, priority);

    // Initialize sound parameters
    if info.link.is_some() {
        pitch = info.pitch;
        priority = info.priority;
        volume += info.volume;

        if volume < 1 {
            return;
        }
        if volume > sfx_volume() {
            volume = sfx_volume();
        }
    } else {
        pitch = NORM_PITCH;
        priority = NORM_PRIORITY;
    }

    // Check to see if it is audible, and if not, modify the params
    let mut sep = NORM_SEP;
    let listener = ctx.listener_pos();
    if let Some(o) = origin {
        // The mobj is gone by the time the queue is flushed (see module
        // docs): nothing to play the sound at.
        let Some((ox, oy)) = ctx.origin_pos(o) else {
            return;
        };
        if let Some(l) = listener {
            if Some(o) != ctx.listener.map(SoundOrigin::Mobj) {
                let audible = s_adjust_sound_params(l, ox, oy, &mut volume, &mut sep, &mut pitch);
                if ox == l.x && oy == l.y {
                    sep = NORM_SEP;
                }
                if !audible {
                    return;
                }
            }
        }
    }

    // hacks to vary the sfx pitches
    let id = sfx_id as usize;
    if id >= Sfx::SfxSawup as usize && id <= Sfx::SfxSawhit as usize {
        pitch += 8 - (m_random() & 15);
        pitch = pitch.clamp(0, 255);
    } else if id != Sfx::SfxItemup as usize && id != Sfx::SfxTink as usize {
        pitch += 16 - (m_random() & 31);
        pitch = pitch.clamp(0, 255);
    }

    // kill old sound
    stop_sound_now(origin);

    // try to find a channel
    let Some(cnum) = s_get_channel(origin, sfx_id as usize) else {
        return;
    };

    with(|s| {
        if s.usefulness[id] < 0 {
            s.usefulness[id] = 1;
        } else {
            s.usefulness[id] += 1;
        }
        // Assigns the handle: channel.handle = I_StartSound(...).
        let handle = s
            .isound
            .start_sound_id(sfx_id, volume, sep, pitch, priority);
        s.channels[cnum].handle = handle;
    });
}

/// Port of `S_StopSound`.
fn stop_sound_now(origin: Option<SoundOrigin>) {
    let found = with(|s| {
        s.channels
            .iter()
            .position(|c| c.sfx.is_some() && c.origin == origin)
    });
    if let Some(cnum) = found {
        s_stop_channel(cnum);
    }
}

/// A deferred `S_StartSoundAtVolume`/`S_StopSound` call.
#[derive(Clone, Copy)]
enum SoundCmd {
    Start {
        origin: Option<SoundOrigin>,
        sfx_id: i32,
        volume: i32,
    },
    Stop(Option<SoundOrigin>),
}

fn enqueue(cmd: SoundCmd) {
    with(|s| {
        if s.initialized {
            s.pending.push(cmd);
        }
    });
}

/// Port of `S_StartSound`. Queued, not executed: see the module docs
/// ("Deferred calls"); [`s_flush`] runs it.
pub fn s_start_sound(origin: Option<SoundOrigin>, sfx: Sfx) {
    // `snd_SfxVolume` is read now, as the original reads it at the call.
    s_start_sound_at_volume(origin, sfx as i32, sfx_volume());
}

/// Port of `S_StartSoundAtVolume`. Queued, see [`s_start_sound`].
pub fn s_start_sound_at_volume(origin: Option<SoundOrigin>, sfx_id: i32, volume: i32) {
    enqueue(SoundCmd::Start {
        origin,
        sfx_id,
        volume,
    });
}

/// Port of `S_StopSound`. Queued, see [`s_start_sound`].
pub fn s_stop_sound(origin: Option<SoundOrigin>) {
    enqueue(SoundCmd::Stop(origin));
}

/// Runs every queued sound call, in order, against the current world.
/// The game loop calls this once per tic, right after `G_Ticker`.
///
/// # Panics
/// On a bad sfx number, like the original's `I_Error`.
pub fn s_flush(ctx: &SoundCtx) {
    let pending = with(|s| std::mem::take(&mut s.pending));
    for cmd in pending {
        match cmd {
            SoundCmd::Start {
                origin,
                sfx_id,
                volume,
            } => start_sound_at_volume_now(ctx, origin, sfx_id, volume),
            SoundCmd::Stop(origin) => stop_sound_now(origin),
        }
    }
}

/// How many logical channels currently hold a sound (diagnostics and
/// tests; the original has no such accessor).
pub fn s_channels_in_use() -> usize {
    with(|s| s.channels.iter().filter(|c| c.sfx.is_some()).count())
}

/// Index (`MusicEnum as usize`) of the music selected by
/// `S_ChangeMusic`, if any (diagnostics and tests).
pub fn s_music_playing() -> Option<usize> {
    with(|s| s.mus_playing)
}

/// Port of `S_PauseSound`.
pub fn s_pause_sound() {
    with(|s| {
        if let Some(m) = s.mus_playing {
            if !s.mus_paused {
                s.isound.pause_song(s.mus_handle[m]);
                s.mus_paused = true;
            }
        }
    });
}

/// Port of `S_ResumeSound`.
pub fn s_resume_sound() {
    with(|s| {
        if let Some(m) = s.mus_playing {
            if s.mus_paused {
                s.isound.resume_song(s.mus_handle[m]);
                s.mus_paused = false;
            }
        }
    });
}

/// Port of `S_UpdateSounds`: updates the params of moving positional
/// sounds and frees finished channels. `ctx.listener` is the
/// `listener` argument.
pub fn s_update_sounds(ctx: &SoundCtx) {
    if !with(|s| s.initialized) {
        return;
    }

    // Anything still queued starts first (normally already flushed at
    // the end of the tic).
    s_flush(ctx);

    // (The original's periodic "usefulness" cache cleanup is commented
    // out there and not ported.)
    let listener = ctx.listener_pos();

    for cnum in 0..with(|s| s.channels.len()) {
        let (sfx, origin, handle, playing) = with(|s| {
            let c = s.channels[cnum];
            (
                c.sfx,
                c.origin,
                c.handle,
                s.isound.sound_is_playing(c.handle),
            )
        });
        let Some(sfx) = sfx else { continue };

        if !playing {
            // if channel is allocated but sound has stopped, free it
            s_stop_channel(cnum);
            continue;
        }

        // initialize parameters
        let mut volume = sfx_volume();
        let mut pitch = NORM_PITCH;
        let mut sep = NORM_SEP;

        let info = &S_SFX[sfx];
        if info.link.is_some() {
            pitch = info.pitch;
            volume += info.volume;

            if volume < 1 {
                s_stop_channel(cnum);
                continue;
            } else if volume > sfx_volume() {
                volume = sfx_volume();
            }
        }

        // check non-local sounds for distance clipping or modify their
        // params
        if let Some(o) = origin {
            if Some(o) != ctx.listener.map(SoundOrigin::Mobj) {
                let (Some(l), Some((ox, oy))) = (listener, ctx.origin_pos(o)) else {
                    // Origin gone (dangling in C) or no listener.
                    if ctx.origin_pos(o).is_none() {
                        s_stop_channel(cnum);
                    }
                    continue;
                };
                let audible = s_adjust_sound_params(l, ox, oy, &mut volume, &mut sep, &mut pitch);
                if !audible {
                    s_stop_channel(cnum);
                } else {
                    with(|s| s.isound.update_sound_params(handle, volume, sep, pitch));
                }
            }
        }
    }
}

/// Port of `S_SetMusicVolume`.
///
/// # Panics
/// On a volume outside 0..=127 (`I_Error`).
pub fn s_set_music_volume(volume: i32) {
    if !(0..=127).contains(&volume) {
        panic!("Attempt to set music volume at {volume}");
    }
    with(|s| {
        s.isound.set_music_volume(127);
        s.isound.set_music_volume(volume);
    });
    doomstat::state_mut().snd_music_volume = volume;
}

/// Port of `S_SetSfxVolume`.
///
/// # Panics
/// On a volume outside 0..=127 (`I_Error`).
pub fn s_set_sfx_volume(volume: i32) {
    if !(0..=127).contains(&volume) {
        panic!("Attempt to set sfx volume at {volume}");
    }
    doomstat::state_mut().snd_sfx_volume = volume;
}

/// Port of `S_StartMusic`.
pub fn s_start_music(wad: &mut WadFiles, m_id: i32) {
    s_change_music(wad, m_id, false);
}

/// Port of `S_ChangeMusic`.
///
/// # Panics
/// On `m_id` outside `1..NUMMUSIC` (`I_Error("Bad music number")`) or a
/// missing `d_<name>` lump.
pub fn s_change_music(wad: &mut WadFiles, musicnum: i32, looping: bool) {
    if !with(|s| s.initialized) {
        return;
    }
    if musicnum <= MusicEnum::MusNone as i32 || musicnum >= NUMMUSIC as i32 {
        panic!("Bad music number {musicnum}");
    }
    let m = musicnum as usize;

    if with(|s| s.mus_playing) == Some(m) {
        return;
    }

    // shutdown old music
    s_stop_music(wad);

    // get lumpnum if neccessary
    if with(|s| s.mus_lumpnum[m]) == 0 {
        let name = format!("d_{}", S_MUSIC[m].unwrap_or(""));
        let lump = wad.get_num_for_name(&name);
        with(|s| s.mus_lumpnum[m] = lump);
    }

    // load & register it
    let lump = with(|s| s.mus_lumpnum[m]);
    let data = wad.cache_lump_num(lump, PurgeTag::Music);
    with(|s| {
        s.mus_handle[m] = s.isound.register_song(data);
        // play it
        s.isound.play_song(s.mus_handle[m], looping);
        s.mus_playing = Some(m);
    });
}

/// Port of `S_StopMusic`.
pub fn s_stop_music(wad: &mut WadFiles) {
    let Some((m, lump)) = with(|s| {
        s.mus_playing.map(|m| {
            if s.mus_paused {
                s.isound.resume_song(s.mus_handle[m]);
            }
            s.isound.stop_song(s.mus_handle[m]);
            s.isound.unregister_song(s.mus_handle[m]);
            (m, s.mus_lumpnum[m])
        })
    }) else {
        return;
    };
    let _ = m;
    // Z_ChangeTag(mus_playing->data, PU_CACHE)
    wad.cache_lump_num(lump, PurgeTag::Cache);
    with(|s| s.mus_playing = None);
}

/// Port of `S_StopChannel`.
fn s_stop_channel(cnum: usize) {
    with(|s| stop_channel_in(s, cnum));
}

/// Port of `S_AdjustSoundParams`: volume, stereo separation and pitch
/// for a source relative to the listener. Returns whether it is
/// audible (`vol > 0`).
fn s_adjust_sound_params(
    listener: Listener,
    sx: Fixed,
    sy: Fixed,
    vol: &mut i32,
    sep: &mut i32,
    _pitch: &mut i32,
) -> bool {
    // calculate the distance to sound origin and clip it if necessary
    let adx = listener.x.wrapping_sub(sx).wrapping_abs();
    let ady = listener.y.wrapping_sub(sy).wrapping_abs();

    // From _GG1_ p.428. Appox. eucledian distance fast.
    let mut approx_dist = adx + ady - (adx.min(ady) >> 1);

    let gamemap = doomstat::state().gamemap;
    if gamemap != 8 && approx_dist > S_CLIPPING_DIST {
        return false;
    }

    // angle of source to listener
    let mut angle: Angle =
        angle_from_delta(sx.wrapping_sub(listener.x), sy.wrapping_sub(listener.y));

    if angle > listener.angle {
        angle -= listener.angle;
    } else {
        angle = angle.wrapping_add(0xffff_ffff - listener.angle);
    }

    angle >>= ANGLETOFINESHIFT;

    // stereo separation
    *sep = 128 - (fixed_mul(S_STEREO_SWING, FINESINE[angle as usize]) >> FRACBITS);

    let sfx_volume = sfx_volume();

    // volume calculation
    if approx_dist < S_CLOSE_DIST {
        *vol = sfx_volume;
    } else if gamemap == 8 {
        if approx_dist > S_CLIPPING_DIST {
            approx_dist = S_CLIPPING_DIST;
        }
        *vol =
            15 + ((sfx_volume - 15) * ((S_CLIPPING_DIST - approx_dist) >> FRACBITS)) / S_ATTENUATOR;
    } else {
        // distance effect
        *vol = (sfx_volume * ((S_CLIPPING_DIST - approx_dist) >> FRACBITS)) / S_ATTENUATOR;
    }

    *vol > 0
}

/// Port of `S_getChannel`: finds a free channel, or one of lower or
/// equal priority to steal. The channel is left claimed by
/// (`sfx`, `origin`); `None` = no channel (the original's -1).
fn s_get_channel(origin: Option<SoundOrigin>, sfx: usize) -> Option<usize> {
    with(|s| {
        let n = s.channels.len();
        // Find an open channel
        let mut cnum = 0;
        while cnum < n {
            if s.channels[cnum].sfx.is_none() {
                break;
            } else if origin.is_some() && s.channels[cnum].origin == origin {
                // (the caller already ran S_StopSound(origin); kept for
                // parity)
                stop_channel_in(s, cnum);
                break;
            }
            cnum += 1;
        }

        // None available
        if cnum == n {
            // Look for lower priority
            cnum = s
                .channels
                .iter()
                .position(|c| S_SFX[c.sfx.unwrap_or(0)].priority >= S_SFX[sfx].priority)
                .unwrap_or(n);
            if cnum == n {
                // FUCK!  No lower priority.  Sorry, Charlie.
                return None;
            }
            // Otherwise, kick out lower priority.
            stop_channel_in(s, cnum);
        }

        s.channels[cnum].sfx = Some(sfx);
        s.channels[cnum].origin = origin;
        Some(cnum)
    })
}

/// `S_StopChannel` body, for callers already holding the state borrow
/// (the original also scans for another channel using the same sfx, to
/// no effect).
fn stop_channel_in(s: &mut SoundState, cnum: usize) {
    let c = s.channels[cnum];
    if let Some(sfx) = c.sfx {
        if s.isound.sound_is_playing(c.handle) {
            s.isound.stop_sound(c.handle);
        }
        s.usefulness[sfx] -= 1;
        s.channels[cnum].sfx = None;
    }
}

/// The music-selection part of `S_Start`.
fn music_for_level(gamemode: GameMode, gameepisode: i32, gamemap: i32) -> i32 {
    if gamemode == GameMode::Commercial {
        MusicEnum::MusRunnin as i32 + gamemap - 1
    } else {
        use MusicEnum::*;
        let spmus = [
            MusE3m4, // American  e4m1
            MusE3m2, // Romero    e4m2
            MusE3m3, // Shawn     e4m3
            MusE1m5, // American  e4m4
            MusE2m7, // Tim       e4m5
            MusE2m4, // Romero    e4m6
            MusE2m6, // J.Anderson e4m7 CHIRON.WAD
            MusE2m5, // Shawn     e4m8
            MusE1m9, // Tim       e4m9
        ];
        if gameepisode < 4 {
            MusE1m1 as i32 + (gameepisode - 1) * 9 + gamemap - 1
        } else {
            spmus[(gamemap - 1) as usize] as i32
        }
    }
}

/// Test helper for the game-logic modules: a fresh, headless sound
/// system that knows every sfx, plus [`test_flush`] to run the queue.
#[cfg(test)]
pub(crate) fn test_init(channels: usize) {
    s_shutdown();
    let mut mixer = crate::i_sound::Mixer::new();
    for id in 1..NUMSFX {
        mixer.insert_sound(id, &[200; 64]);
    }
    s_init(ISound::headless(mixer), 15, 15);
    with(|s| s.channels = vec![Channel::default(); channels]);
}

/// Runs the queued calls with no listener (everything plays unpanned).
#[cfg(test)]
pub(crate) fn test_flush(thinkers: &Thinkers, level: &Level) {
    s_flush(&SoundCtx {
        thinkers,
        level,
        listener: None,
    });
}

/// The sfx (as `Sfx as usize`) on each busy channel, in channel order.
#[cfg(test)]
pub(crate) fn test_playing() -> Vec<usize> {
    with(|s| s.channels.iter().filter_map(|c| c.sfx).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i_sound::Mixer;
    use crate::info::MobjType;
    use crate::p_tick::{ThinkFn, ThinkerData};
    use crate::r_defs::Mobj;

    struct World {
        thinkers: Thinkers,
        level: Level,
        listener: ThinkerId,
    }

    impl World {
        fn new() -> Self {
            let mut thinkers = Thinkers::new();
            let listener = Self::add(&mut thinkers, 0, 0);
            Self {
                thinkers,
                level: Level::default(),
                listener,
            }
        }

        fn add(t: &mut Thinkers, x: i32, y: i32) -> ThinkerId {
            let mut m = Mobj::blank(MobjType::MtPlayer);
            m.x = x;
            m.y = y;
            t.add_thinker(ThinkFn::MobjThinker, ThinkerData::Mobj(m))
        }

        fn mobj_at(&mut self, units: i32) -> SoundOrigin {
            SoundOrigin::Mobj(Self::add(&mut self.thinkers, units * 0x10000, 0))
        }

        fn ctx(&self) -> SoundCtx<'_> {
            SoundCtx {
                thinkers: &self.thinkers,
                level: &self.level,
                listener: Some(self.listener),
            }
        }
    }

    /// Fresh per-thread state with a headless mixer that knows the
    /// sounds the tests use. Volumes are set to the defaults `doomstat`
    /// already has, so parallel tests see no change.
    fn init(channels: usize) {
        s_shutdown();
        let mut mixer = Mixer::new();
        for id in [
            Sfx::SfxPistol,
            Sfx::SfxShotgn,
            Sfx::SfxSawup,
            Sfx::SfxItemup,
        ] {
            mixer.insert_sound(id as usize, &[200; 64]);
        }
        s_init(ISound::headless(mixer), 15, 15);
        with(|s| s.channels = vec![Channel::default(); channels]);
    }

    fn busy() -> usize {
        with(|s| s.channels.iter().filter(|c| c.sfx.is_some()).count())
    }

    #[test]
    fn everything_is_a_noop_before_init() {
        s_shutdown();
        let w = World::new();
        start_sound_now(&w.ctx(), None, Sfx::SfxPistol);
        s_update_sounds(&w.ctx());
        assert_eq!(busy(), 0);
    }

    #[test]
    fn unpositioned_sound_takes_a_channel_and_replaces_its_predecessor() {
        init(3);
        let w = World::new();
        start_sound_now(&w.ctx(), None, Sfx::SfxPistol);
        assert_eq!(busy(), 1);
        // Same (NULL) origin: S_StopSound(NULL) kills the old one first.
        start_sound_now(&w.ctx(), None, Sfx::SfxShotgn);
        assert_eq!(busy(), 1);
        assert_eq!(with(|s| s.channels[0].sfx), Some(Sfx::SfxShotgn as usize));
        s_shutdown();
    }

    #[test]
    fn distinct_origins_use_distinct_channels_and_full_table_steals() {
        init(2);
        let mut w = World::new();
        let (a, b, c) = (w.mobj_at(10), w.mobj_at(20), w.mobj_at(30));
        start_sound_now(&w.ctx(), Some(a), Sfx::SfxPistol);
        start_sound_now(&w.ctx(), Some(b), Sfx::SfxPistol);
        assert_eq!(busy(), 2);
        // Table full: equal priority (64 >= 64) kicks out channel 0.
        start_sound_now(&w.ctx(), Some(c), Sfx::SfxPistol);
        assert_eq!(busy(), 2);
        assert_eq!(with(|s| s.channels[0].origin), Some(c));
        stop_sound_now(Some(b));
        assert_eq!(busy(), 1);
        s_shutdown();
    }

    #[test]
    fn priority_decides_channel_stealing() {
        init(1);
        let mut w = World::new();
        let (a, b) = (w.mobj_at(10), w.mobj_at(20));
        // Bigger number = lower priority: a playing pistol (64) is not
        // kicked out by itemup (78), since `64 >= 78` is false.
        start_sound_now(&w.ctx(), Some(a), Sfx::SfxPistol);
        start_sound_now(&w.ctx(), Some(b), Sfx::SfxItemup);
        assert_eq!(with(|s| s.channels[0].origin), Some(a));
        assert_eq!(with(|s| s.channels[0].sfx), Some(Sfx::SfxPistol as usize));
        // ... but the reverse steals: itemup playing, pistol arrives.
        stop_sound_now(Some(a));
        start_sound_now(&w.ctx(), Some(a), Sfx::SfxItemup);
        start_sound_now(&w.ctx(), Some(b), Sfx::SfxPistol);
        assert_eq!(with(|s| s.channels[0].origin), Some(b));
        s_shutdown();
    }

    #[test]
    fn far_sources_are_inaudible_and_near_ones_full_volume() {
        init(3);
        let mut w = World::new();
        let far = w.mobj_at(1300); // > S_CLIPPING_DIST (1200 units)
        let near = w.mobj_at(100); // < S_CLOSE_DIST (160 units)
        start_sound_now(&w.ctx(), Some(far), Sfx::SfxPistol);
        assert_eq!(busy(), 0);
        start_sound_now(&w.ctx(), Some(near), Sfx::SfxPistol);
        assert_eq!(busy(), 1);
        s_shutdown();
    }

    #[test]
    fn adjust_params_matches_the_original_formula() {
        let l = Listener {
            x: 0,
            y: 0,
            angle: 0,
        };
        let (mut vol, mut sep, mut pitch) = (0, 0, NORM_PITCH);
        // 160 units: exactly S_CLOSE_DIST is not "< close": attenuated
        // volume = 15 * ((1200-160)*... ) / 1040 = 15.
        assert!(s_adjust_sound_params(
            l,
            160 << 16,
            0,
            &mut vol,
            &mut sep,
            &mut pitch
        ));
        assert_eq!(vol, 15);
        // Halfway between close and clipping: (1200-680)/1040 = 0.5 -> 7.
        assert!(s_adjust_sound_params(
            l,
            680 << 16,
            0,
            &mut vol,
            &mut sep,
            &mut pitch
        ));
        assert_eq!(vol, 15 * 520 / 1040);
        // Source dead ahead (source angle == listener angle == 0): the
        // original's `angle > listener->angle` is false, so it takes the
        // `angle + (0xffffffff - listener->angle)` branch (== -1 turn
        // unit), whose tiny negative sine puts sep at 129, not 128.
        assert_eq!(sep, 129);
        // Beyond clipping: inaudible.
        assert!(!s_adjust_sound_params(
            l,
            1201 << 16,
            0,
            &mut vol,
            &mut sep,
            &mut pitch
        ));
    }

    #[test]
    fn update_stops_channels_that_moved_out_of_range() {
        init(3);
        let mut w = World::new();
        let o = w.mobj_at(100);
        start_sound_now(&w.ctx(), Some(o), Sfx::SfxPistol);
        assert_eq!(busy(), 1);
        s_update_sounds(&w.ctx());
        assert_eq!(busy(), 1);
        let SoundOrigin::Mobj(id) = o else {
            unreachable!()
        };
        w.thinkers.mobj_mut(id).unwrap().x = 2000 << 16;
        s_update_sounds(&w.ctx());
        assert_eq!(busy(), 0);
        s_shutdown();
    }

    #[test]
    fn update_stops_channels_whose_origin_was_removed() {
        init(3);
        let mut w = World::new();
        let o = w.mobj_at(100);
        start_sound_now(&w.ctx(), Some(o), Sfx::SfxPistol);
        let SoundOrigin::Mobj(id) = o else {
            unreachable!()
        };
        w.thinkers.remove(id);
        // Like the original, the mobj stays readable until the thinker
        // sweep frees it.
        w.thinkers.run_thinkers(|_, _| {});
        s_update_sounds(&w.ctx());
        assert_eq!(busy(), 0);
        s_shutdown();
    }

    #[test]
    fn stop_sound_only_stops_the_matching_origin() {
        init(3);
        let mut w = World::new();
        let (a, b) = (w.mobj_at(10), w.mobj_at(20));
        start_sound_now(&w.ctx(), Some(a), Sfx::SfxPistol);
        start_sound_now(&w.ctx(), Some(b), Sfx::SfxShotgn);
        stop_sound_now(Some(a));
        assert_eq!(busy(), 1);
        assert_eq!(with(|s| s.channels[1].origin), Some(b));
        s_shutdown();
    }

    #[test]
    #[should_panic(expected = "Bad sfx #: 0")]
    fn bad_sfx_number_panics() {
        init(3);
        let w = World::new();
        start_sound_at_volume_now(&w.ctx(), None, 0, 15);
    }

    #[test]
    fn music_selection_matches_s_start() {
        use MusicEnum::*;
        assert_eq!(music_for_level(GameMode::Shareware, 1, 1), MusE1m1 as i32);
        assert_eq!(music_for_level(GameMode::Registered, 2, 3), MusE2m3 as i32);
        assert_eq!(music_for_level(GameMode::Retail, 4, 1), MusE3m4 as i32);
        assert_eq!(music_for_level(GameMode::Retail, 4, 9), MusE1m9 as i32);
        assert_eq!(
            music_for_level(GameMode::Commercial, 1, 5),
            MusRunnin as i32 + 4
        );
    }

    #[test]
    fn calls_are_queued_until_flushed_and_run_in_order() {
        init(3);
        let mut w = World::new();
        let a = w.mobj_at(10);
        s_start_sound(Some(a), Sfx::SfxPistol);
        s_stop_sound(Some(a));
        s_start_sound(Some(a), Sfx::SfxShotgn);
        assert_eq!(busy(), 0);
        s_flush(&w.ctx());
        // start, stop, start: only the last survives.
        assert_eq!(busy(), 1);
        assert_eq!(with(|s| s.channels[0].sfx), Some(Sfx::SfxShotgn as usize));
        s_shutdown();
    }

    #[test]
    fn queued_sound_from_a_freed_mobj_is_dropped() {
        init(3);
        let mut w = World::new();
        let a = w.mobj_at(10);
        s_start_sound(Some(a), Sfx::SfxPistol);
        let SoundOrigin::Mobj(id) = a else {
            unreachable!()
        };
        w.thinkers.remove(id);
        w.thinkers.run_thinkers(|_, _| {});
        s_flush(&w.ctx());
        assert_eq!(busy(), 0);
        s_shutdown();
    }

    #[test]
    fn sector_origins_resolve_through_the_level() {
        init(3);
        let mut w = World::new();
        w.level.sectors = vec![Default::default(); 2];
        w.level.sectors[1].soundorg.x = 50 << 16;
        s_start_sound(sector_origin(1), Sfx::SfxDoropn);
        s_flush(&w.ctx());
        assert_eq!(busy(), 1);
        // a sector that doesn't exist can't be resolved: dropped.
        s_start_sound(sector_origin(9), Sfx::SfxDoropn);
        s_flush(&w.ctx());
        assert_eq!(busy(), 1);
        s_shutdown();
    }
}
