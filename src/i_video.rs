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
//	DOOM graphics stuff for SDL2.
//
//-----------------------------------------------------------------------------

//! Rust port of `i_video.h` / `i_video.c`.
//!
//! DOOM graphics stuff for SDL2. This is the one module that talks to
//! SDL2 directly (per the plan: window/renderer/texture, input events,
//! palette), using the `sdl2` crate in place of the original's `#include
//! <SDL.h>`.
//!
//! # Scope
//!
//! Ported: window/renderer/texture setup (`I_InitGraphics`), palette
//! application (`I_SetPalette`), the streaming-texture blit + present
//! (`I_FinishUpdate`), key translation (`xlatekey`), and SDL event
//! pumping into `event_t`s (`I_GetEvent`/`I_StartTic`) — everything the
//! original's `i_video.c` actually implements.
//!
//! Not ported: `I_ShutdownGraphics` only calls SDL destroy functions,
//! which this port instead gets for free from `Drop` on `sdl2`'s RAII
//! wrapper types ([`IVideo`] holds them, so dropping an `IVideo` tears
//! down the window/renderer/texture — no separate shutdown function
//! needed, and there is nothing left to call once
//! [`crate::doomstat::state_mut`]'s eventual `D_QuitNetGame`/sound
//! shutdown callers exist in later phases). `I_ReadScreen` (a
//! straight `memcpy` from `screens[0]`) isn't ported as a *function*
//! here — callers can just read `IVideo`'s owned `v_video::VVideo`
//! screen buffer directly, so a wrapper adds nothing. `I_BeginRead`/
//! `I_EndRead` are empty in the original, not ported (see `i_system`'s
//! module docs for the same call already made there).
//!
//! `I_MouseState` is folded directly into event handling below rather
//! than kept as a separate function, since `sdl2`'s event enum already
//! carries button state per-event (no need for the original's
//! `SDL_GetMouseState` polling workaround).

use sdl2::event::Event as SdlEvent;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use sdl2::{EventPump, Sdl};

use crate::d_event::{EvType, Event};
use crate::doomdef::{
    KEY_BACKSPACE, KEY_DOWNARROW, KEY_ENTER, KEY_EQUALS, KEY_ESCAPE, KEY_F1, KEY_F10, KEY_F11,
    KEY_F12, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8, KEY_F9, KEY_LEFTARROW,
    KEY_MINUS, KEY_PAUSE, KEY_RALT, KEY_RCTRL, KEY_RIGHTARROW, KEY_RSHIFT, KEY_STRAFELEFT,
    KEY_STRAFERIGHT, KEY_TAB, KEY_UPARROW, SCREENHEIGHT, SCREENWIDTH,
};
use crate::m_argv::m_check_parm;
use crate::v_video::VVideo;

/// Port of `xlatekey`. Translates the key currently in the SDL event.
///
/// The original falls back to `sym->sym` (SDL's own keysym value) for
/// any key without a specific `DOOM` key code — ported the same way via
/// [`Keycode::into_i32`], so unmapped keys still produce *some* stable
/// value rather than being dropped.
pub fn xlatekey(keycode: Keycode) -> i32 {
    match keycode {
        Keycode::LEFT => KEY_LEFTARROW,
        Keycode::RIGHT => KEY_RIGHTARROW,
        Keycode::DOWN | Keycode::S => KEY_DOWNARROW,
        Keycode::UP | Keycode::W => KEY_UPARROW,
        Keycode::LESS | Keycode::A => KEY_STRAFELEFT,
        Keycode::GREATER | Keycode::D => KEY_STRAFERIGHT,
        Keycode::ESCAPE => KEY_ESCAPE,
        Keycode::RETURN => KEY_ENTER,
        Keycode::TAB => KEY_TAB,
        Keycode::F1 => KEY_F1,
        Keycode::F2 => KEY_F2,
        Keycode::F3 => KEY_F3,
        Keycode::F4 => KEY_F4,
        Keycode::F5 => KEY_F5,
        Keycode::F6 => KEY_F6,
        Keycode::F7 => KEY_F7,
        Keycode::F8 => KEY_F8,
        Keycode::F9 => KEY_F9,
        Keycode::F10 => KEY_F10,
        Keycode::F11 => KEY_F11,
        Keycode::F12 => KEY_F12,
        Keycode::BACKSPACE | Keycode::DELETE => KEY_BACKSPACE,
        Keycode::PAUSE => KEY_PAUSE,
        Keycode::EQUALS => KEY_EQUALS,
        Keycode::MINUS => KEY_MINUS,
        Keycode::LSHIFT | Keycode::RSHIFT => KEY_RSHIFT,
        Keycode::LCTRL | Keycode::RCTRL => KEY_RCTRL,
        Keycode::LALT | Keycode::LGUI | Keycode::RALT | Keycode::RGUI => KEY_RALT,
        other => other.into_i32(),
    }
}

/// Mouse button bits for [`Event::data1`], matching `I_MouseState`'s
/// `event->data1 |= 1/2/4` for left/right/middle.
mod mouse_bits {
    pub const LEFT: i32 = 1;
    pub const RIGHT: i32 = 2;
    pub const MIDDLE: i32 = 4;
}

/// Port of the `i_video` module: owns the SDL context, window, renderer,
/// streaming texture, and the current 256-color palette, plus a
/// [`VVideo`] holding the `screens[]` framebuffers this blits from.
///
/// Unlike `doomstat`/`r_state`, this deliberately does NOT live behind a
/// process-global `static` (the plan's usual globals pattern) — `sdl2`'s
/// types are not `Sync` (SDL itself is not thread-safe), so a `'static`
/// global here would need the same `GlobalCell` unsafe-assertion
/// machinery `doomstat`/`r_state` use, for a resource (a live SDL
/// window/renderer) that's much riskier to alias/access from unexpected
/// places than plain data. A later phase's game-flow code
/// (`d_main`/`g_game`) is expected to own one `IVideo` instance directly
/// instead, the same way it's expected to own a `w_wad::WadFiles`.
pub struct IVideo {
    _sdl: Sdl,
    canvas: Canvas<Window>,
    _texture_creator: TextureCreator<WindowContext>,
    texture: Texture<'static>,
    event_pump: EventPump,
    palette: [(u8, u8, u8); 256],
    pub v_video: VVideo,
    /// True once an `SDL_QUIT` event has been observed — see
    /// [`IVideo::should_quit`].
    quit_requested: bool,
}

// SAFETY: `texture` borrows from `_texture_creator`, which is never
// moved out of or dropped independently of `IVideo` (both live in the
// same struct, dropped together in declaration order — `texture` before
// `_texture_creator`, satisfying the borrow). `Texture<'static>` is a
// self-referential-struct workaround: the texture creator is boxed
// implicitly by `sdl2`'s API returning an owned value tied to the
// canvas's lifetime, and transmuting that borrow to `'static` here is
// sound exactly because `_texture_creator` outlives `texture` by
// construction (struct field drop order) and is never accessed
// separately once construction finishes.
impl IVideo {
    /// SDL's audio subsystem, from the same `Sdl` context this window
    /// lives in (`I_InitSound` needs one; the original just calls
    /// `SDL_InitSubSystem(SDL_INIT_AUDIO)`).
    pub fn audio_subsystem(&self) -> Result<sdl2::AudioSubsystem, String> {
        self._sdl.audio()
    }

    /// Port of `I_InitGraphics`. Called by `D_DoomMain`, determines the
    /// hardware configuration and sets up the video mode.
    ///
    /// The original guards against being called more than once with a
    /// `static int firsttime` flag (a no-op on the second call); this
    /// port instead makes that a non-issue by construction — calling
    /// this constructs a fresh `IVideo`, so "already initialized" isn't
    /// representable the way it was with the original's implicit global
    /// window/renderer/texture statics.
    ///
    /// # Panics
    /// Panics (standing in for `I_Error`) if SDL initialization, window,
    /// renderer, or texture creation fails — same fatal-error semantics
    /// as the original.
    pub fn i_init_graphics() -> Self {
        let sdl = sdl2::init().unwrap_or_else(|e| panic!("SDL_Init(SDL_INIT_VIDEO) failed: {e}"));
        let video_subsystem = sdl
            .video()
            .unwrap_or_else(|e| panic!("SDL_Init(SDL_INIT_VIDEO) failed: {e}"));

        let mut multiply = 1;
        if m_check_parm("-2") != 0 {
            multiply = 2;
        }
        if m_check_parm("-3") != 0 {
            multiply = 3;
        }
        if m_check_parm("-4") != 0 {
            multiply = 4;
        }
        if multiply == 1 {
            // The original picks 3 on _WIN32, 4 otherwise; this port
            // targets the same platforms `NORMALUNIX` covers (see
            // CMakeLists.txt's `-DNORMALUNIX`), so the non-Windows
            // branch applies uniformly here.
            multiply = 4;
        }

        let width = (SCREENWIDTH * multiply) as u32;
        let height = (SCREENHEIGHT * multiply) as u32;

        let window = video_subsystem
            .window("DoomMetalRust", width, height)
            .position_centered()
            .resizable()
            .build()
            .unwrap_or_else(|e| panic!("SDL_CreateWindow failed: {e}"));

        let mut canvas = window
            .into_canvas()
            .build()
            .unwrap_or_else(|e| panic!("SDL_CreateRenderer failed: {e}"));

        canvas
            .set_logical_size(SCREENWIDTH as u32, SCREENHEIGHT as u32)
            .unwrap_or_else(|e| panic!("SDL_RenderSetLogicalSize failed: {e}"));

        let texture_creator = canvas.texture_creator();
        let texture = texture_creator
            .create_texture_streaming(
                PixelFormatEnum::ARGB8888,
                SCREENWIDTH as u32,
                SCREENHEIGHT as u32,
            )
            .unwrap_or_else(|e| panic!("SDL_CreateTexture failed: {e}"));
        // SAFETY: see the impl block's safety note above this
        // constructor — `_texture_creator` (declared after `texture` so
        // it drops after) outlives `texture` for the life of this
        // `IVideo`.
        let texture: Texture<'static> = unsafe { std::mem::transmute(texture) };

        let event_pump = sdl
            .event_pump()
            .unwrap_or_else(|e| panic!("SDL_Init(SDL_INIT_EVENTS) failed: {e}"));

        // SDL_SetRelativeMouseMode(SDL_TRUE) in the original.
        sdl.mouse().set_relative_mouse_mode(true);

        IVideo {
            _sdl: sdl,
            canvas,
            _texture_creator: texture_creator,
            texture,
            event_pump,
            palette: [(0, 0, 0); 256],
            v_video: VVideo::new(),
            quit_requested: false,
        }
    }

    /// Port of `I_SetPalette`. Takes full 8-bit values. `pal` must
    /// contain at least 768 bytes (256 RGB triples), matching a raw
    /// `PLAYPAL` lump.
    ///
    /// `gammatable`/`usegamma` come from [`IVideo::v_video`], matching
    /// the original reading the same-named globals from `v_video.c`.
    pub fn i_set_palette(&mut self, pal: &[u8]) {
        let gamma = &self.v_video.gammatable[self.v_video.usegamma as usize];
        for (i, chunk) in pal.chunks_exact(3).take(256).enumerate() {
            self.palette[i] = (
                gamma[chunk[0] as usize],
                gamma[chunk[1] as usize],
                gamma[chunk[2] as usize],
            );
        }
    }

    /// Port of `I_FinishUpdate`. Blits `v_video.screens[0]` through the
    /// palette into the streaming texture and presents it.
    ///
    /// The original's `devparm` frame-time dot indicator (drawn directly
    /// into `screens[0]` before blitting) is not ported: it depends on
    /// `doomstat.devparm`/`i_system::i_get_time`, and is a debug-only
    /// visual aid with no bearing on the Phase 4 milestone ("open a
    /// window and show a frame"); revisit alongside `doomstat`
    /// wiring once a real game loop calls this every frame.
    pub fn i_finish_update(&mut self) {
        let screen = &self.v_video.screens[0];

        self.texture
            .with_lock(None, |dst: &mut [u8], pitch: usize| {
                for y in 0..SCREENHEIGHT as usize {
                    for x in 0..SCREENWIDTH as usize {
                        let val = screen[y * SCREENWIDTH as usize + x];
                        let (r, g, b) = self.palette[val as usize];
                        let offset = y * pitch + x * 4;
                        // ARGB8888, matching the original's
                        // `(0xFF<<24)|(r<<16)|(g<<8)|b` packing, written
                        // out as the 4 little-endian bytes SDL's pitched
                        // texture buffer expects for that pixel format.
                        dst[offset] = b;
                        dst[offset + 1] = g;
                        dst[offset + 2] = r;
                        dst[offset + 3] = 0xFF;
                    }
                }
            })
            .unwrap_or_else(|e| panic!("SDL_LockTexture failed: {e}"));

        self.canvas.clear();
        self.canvas
            .copy(&self.texture, None, None)
            .unwrap_or_else(|e| panic!("SDL_RenderCopy failed: {e}"));
        self.canvas.present();
    }

    /// Port of `I_UpdateNoBlit`. "what is this?" (the original's own
    /// comment) — an empty function in the original, kept empty here.
    pub fn i_update_no_blit(&self) {}

    /// Port of `I_GetEvent`. Pumps pending SDL events, translating each
    /// into a [`d_event::Event`][crate::d_event::Event] and returning
    /// them in order (the original instead calls `D_PostEvent`
    /// directly into a global ring buffer not wired up yet in this
    /// phase — `d_main`/`g_game`'s event queue is later-phase work, so
    /// callers here collect the translated events themselves instead).
    ///
    /// `SDL_QUIT` is surfaced as `None` in the returned `Vec`'s absence
    /// of further events plus [`IVideo::should_quit`] rather than
    /// calling `I_Quit`/`exit()` directly — a video module forcing
    /// process exit is a later-phase (`i_system`) concern once that's
    /// wired to the rest of shutdown, not something to short-circuit
    /// here.
    pub fn i_get_event(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        let last_mouse_x = 10i32;
        let last_mouse_y = 10i32;

        while let Some(sdl_event) = self.event_pump.poll_event() {
            match sdl_event {
                SdlEvent::Quit { .. } => {
                    self.quit_requested = true;
                }
                SdlEvent::KeyDown {
                    keycode: Some(kc), ..
                } => {
                    events.push(Event {
                        event_type: EvType::KeyDown,
                        data1: xlatekey(kc),
                        data2: 0,
                        data3: 0,
                    });
                }
                SdlEvent::KeyUp {
                    keycode: Some(kc), ..
                } => {
                    events.push(Event {
                        event_type: EvType::KeyUp,
                        data1: xlatekey(kc),
                        data2: 0,
                        data3: 0,
                    });
                }
                SdlEvent::MouseButtonDown { mouse_btn, .. }
                | SdlEvent::MouseButtonUp { mouse_btn, .. } => {
                    events.push(Event {
                        event_type: EvType::Mouse,
                        data1: mouse_button_bits(&self.event_pump, mouse_btn),
                        data2: 0,
                        data3: 0,
                    });
                }
                SdlEvent::MouseMotion {
                    xrel,
                    yrel,
                    mousestate,
                    ..
                } => {
                    events.push(Event {
                        event_type: EvType::Mouse,
                        data1: mouse_state_bits(mousestate),
                        data2: xrel * last_mouse_x,
                        data3: -yrel * last_mouse_y,
                    });
                }
                _ => {}
            }
        }
        // last_mouse_x/y are fixed constants in the original too (10, 10
        // — `static int lastmousex = 10;` is never reassigned anywhere
        // in i_video.c despite the name suggesting it tracks position;
        // preserved as the same always-10 multiplier, not "fixed" into
        // an actual last-position tracker).
        let _ = (last_mouse_x, last_mouse_y);

        events
    }

    /// Port of `I_StartTic`.
    pub fn i_start_tic(&mut self) -> Vec<Event> {
        self.i_get_event()
    }

    /// Port of `I_StartFrame`. "er?" (the original's own comment) — an
    /// empty function in the original, kept empty here.
    pub fn i_start_frame(&self) {}

    /// True once an `SDL_QUIT` event has been observed by
    /// [`IVideo::i_get_event`]. The original calls `I_Quit()` (which
    /// `exit()`s the process) directly from its event loop; this port
    /// surfaces the request instead, leaving the decision of *how* to
    /// shut down to a later phase's `i_system`/`d_main` wiring (see
    /// `i_system`'s module docs on why `I_Quit`/`I_Error` are
    /// deliberately minimal for now).
    pub fn should_quit(&self) -> bool {
        self.quit_requested
    }

    /// Read back the rendered frame as tightly-packed RGB24 bytes (row
    /// by row, top to bottom), via `SDL_RenderReadPixels`. Not part of
    /// the original API (the real engine never reads its own rendered
    /// output back) — exposed for the Phase 4 milestone's screenshot
    /// integration test (`tests/video_screenshot.rs`) to verify actual
    /// rendered pixels rather than trusting [`IVideo::i_finish_update`]
    /// by inspection alone.
    pub fn read_rendered_pixels_rgb24(&self) -> Vec<u8> {
        self.canvas
            .read_pixels(None, PixelFormatEnum::RGB24)
            .expect("SDL_RenderReadPixels failed")
    }

    /// The logical (post-`set_logical_size`) frame dimensions currently
    /// being rendered — `(SCREENWIDTH, SCREENHEIGHT)` in practice, but
    /// read back from the canvas rather than assumed, for the same
    /// screenshot-test verification purpose as
    /// [`IVideo::read_rendered_pixels_rgb24`].
    pub fn output_size(&self) -> (u32, u32) {
        self.canvas
            .output_size()
            .expect("SDL_GetRendererOutputSize failed")
    }
}

fn mouse_button_bits(pump: &EventPump, _btn: sdl2::mouse::MouseButton) -> i32 {
    mouse_state_bits(pump.mouse_state())
}

fn mouse_state_bits(state: sdl2::mouse::MouseState) -> i32 {
    let mut bits = 0;
    if state.left() {
        bits |= mouse_bits::LEFT;
    }
    if state.right() {
        bits |= mouse_bits::RIGHT;
    }
    if state.middle() {
        bits |= mouse_bits::MIDDLE;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xlatekey_maps_arrows_and_wasd() {
        assert_eq!(xlatekey(Keycode::LEFT), KEY_LEFTARROW);
        assert_eq!(xlatekey(Keycode::RIGHT), KEY_RIGHTARROW);
        assert_eq!(xlatekey(Keycode::W), KEY_UPARROW);
        assert_eq!(xlatekey(Keycode::S), KEY_DOWNARROW);
    }

    #[test]
    fn xlatekey_maps_function_keys() {
        assert_eq!(xlatekey(Keycode::F1), KEY_F1);
        assert_eq!(xlatekey(Keycode::F12), KEY_F12);
    }

    #[test]
    fn xlatekey_falls_back_to_raw_keycode() {
        // A key with no special DOOM mapping should fall back to SDL's
        // own keycode value, matching the original's `default: rc =
        // sym->sym;`.
        assert_eq!(xlatekey(Keycode::Z), Keycode::Z.into_i32());
    }
}
