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
//	Main loop menu stuff.
//	Default Config File.
//	PCX Screenshots.
//
//-----------------------------------------------------------------------------

//! Rust port of `m_misc.h` / `m_misc.c`. Main loop, menu and screen-shot
//! odds and ends: text drawing ([`m_draw_text`]), whole-file helpers
//! ([`m_write_file`]/[`m_read_file`]), the configuration file
//! ([`Defaults`], `M_LoadDefaults`/`M_SaveDefaults`) and the PCX
//! screenshot ([`m_screenshot`]).
//!
//! # Divergences
//!
//! * The original's `defaults[]` table binds each name to the address
//!   of a global (`&mouseSensitivity`, `&snd_SfxVolume`, `&key_fire`,
//!   ...). Here [`Defaults`] holds the values by name and
//!   [`Defaults::apply`] copies the ones that have a home in this port
//!   into `doomstat` (volumes, messages, mouse sensitivity, screen
//!   size, detail, channels, gamma). The key/mouse/joystick bindings
//!   and `use_mouse`/`use_joystick` are read, kept and written back but
//!   `g_game`'s controls still use the built-in bindings.
//! * The string-valued entries (`chatmacro0..9`, and the Linux
//!   `mousedev`/`mousetype`) aren't part of the table: neither is
//!   used by anything ported (chat macros are the `dstrings` defaults).
//! * `M_ReadFile` returns a `Vec<u8>` instead of a `Z_Malloc`'d block.
//! * `M_DrawText` guards the original's off-by-one (`c >
//!   HU_FONTSIZE`, which would index one past the font) by treating
//!   that index as a space too.
//! * The screenshot reads the *visible* `screens[0]` (`I_ReadScreen` is
//!   a plain copy) and takes the palette as a parameter.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::d_player::Player;
use crate::doomdef::{
    KEY_DOWNARROW, KEY_LEFTARROW, KEY_RALT, KEY_RCTRL, KEY_RIGHTARROW, KEY_RSHIFT, KEY_STRAFELEFT,
    KEY_STRAFERIGHT, KEY_UPARROW, SCREENHEIGHT, SCREENWIDTH,
};
use crate::doomstat;
use crate::hu_stuff::{hu_font, HU_FONTSIZE, HU_FONTSTART};
use crate::v_video::{patch_width, VVideo};

/// Port of `M_DrawText`. Write a string using the hu_font; returns the
/// x after the last character. `direct` selects `V_DrawPatchDirect`
/// (which is the same call in this build).
pub fn m_draw_text(v: &mut VVideo, mut x: i32, y: i32, direct: bool, string: &str) -> i32 {
    let font = hu_font();
    for b in string.bytes() {
        let c = b.to_ascii_uppercase() as i32 - HU_FONTSTART as i32;
        if c < 0 || c >= HU_FONTSIZE as i32 {
            x += 4;
            continue;
        }

        let w = patch_width(&font[c as usize]);
        if x + w > SCREENWIDTH {
            break;
        }
        if direct {
            v.v_draw_patch_direct(x, y, 0, &font[c as usize]);
        } else {
            v.v_draw_patch(x, y, 0, &font[c as usize]);
        }
        x += w;
    }
    x
}

/// Port of `M_WriteFile`. Returns false if the file couldn't be
/// written completely.
pub fn m_write_file(name: impl AsRef<Path>, source: &[u8]) -> bool {
    std::fs::write(name, source).is_ok()
}

/// Port of `M_ReadFile`.
///
/// # Panics
/// If the file can't be read (`I_Error("Couldn't read file %s")`).
pub fn m_read_file(name: impl AsRef<Path>) -> Vec<u8> {
    let name = name.as_ref();
    std::fs::read(name).unwrap_or_else(|_| panic!("Couldn't read file {}", name.display()))
}

/// The `defaults[]` table: (name, default value). Values are `int`s;
/// see the module docs for the string entries left out.
pub const DEFAULTS: &[(&str, i32)] = &[
    ("mouse_sensitivity", 5),
    ("sfx_volume", 8),
    ("music_volume", 8),
    ("show_messages", 1),
    ("key_right", KEY_RIGHTARROW),
    ("key_left", KEY_LEFTARROW),
    ("key_up", KEY_UPARROW),
    ("key_down", KEY_DOWNARROW),
    ("key_strafeleft", KEY_STRAFELEFT),
    ("key_straferight", KEY_STRAFERIGHT),
    ("key_fire", KEY_RCTRL),
    ("key_use", b' ' as i32),
    ("key_strafe", KEY_RALT),
    ("key_speed", KEY_RSHIFT),
    ("use_mouse", 1),
    ("mouseb_fire", 0),
    ("mouseb_strafe", 1),
    ("mouseb_forward", 2),
    ("use_joystick", 0),
    ("joyb_fire", 0),
    ("joyb_strafe", 1),
    ("joyb_use", 3),
    ("joyb_speed", 2),
    ("screenblocks", 9),
    ("detaillevel", 0),
    ("snd_channels", 3),
    ("usegamma", 0),
];

/// The configuration values, in [`DEFAULTS`] order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    values: Vec<i32>,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            values: DEFAULTS.iter().map(|&(_, v)| v).collect(),
        }
    }
}

/// `sscanf("%i")`/the `0x` special case of `M_LoadDefaults`: decimal,
/// `0x` hex or leading-`0` octal, with an optional sign.
fn parse_c_int(s: &str) -> Option<i32> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let v = if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()?
    } else if body.len() > 1 && body.starts_with('0') {
        i64::from_str_radix(&body[1..], 8).ok()?
    } else {
        body.parse::<i64>().ok()?
    };
    Some(if neg { -v } else { v } as i32)
}

impl Defaults {
    /// The value of `name` (`None` for an unknown name).
    pub fn get(&self, name: &str) -> Option<i32> {
        DEFAULTS
            .iter()
            .position(|&(n, _)| n == name)
            .map(|i| self.values[i])
    }

    /// Sets `name` (ignored if unknown, like the original's loop).
    pub fn set(&mut self, name: &str, value: i32) {
        if let Some(i) = DEFAULTS.iter().position(|&(n, _)| n == name) {
            self.values[i] = value;
        }
    }

    /// Port of `M_LoadDefaults`' file parsing: lines of `name value`.
    /// Unknown names and unparsable values are skipped.
    pub fn parse(&mut self, text: &str) {
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(name), Some(parm)) = (it.next(), it.next()) else {
                continue;
            };
            // String parameters ("...") only exist for the entries left
            // out of the table.
            if parm.starts_with('"') {
                continue;
            }
            if let Some(v) = parse_c_int(parm) {
                self.set(name, v);
            }
        }
    }

    /// Port of `M_SaveDefaults`' output: `name\t\tvalue\n` per entry.
    pub fn serialize(&self) -> String {
        DEFAULTS
            .iter()
            .zip(&self.values)
            .map(|(&(n, _), v)| format!("{n}\t\t{v}\n"))
            .collect()
    }

    /// Copies the values that have a home in this port into `doomstat`
    /// (and returns `usegamma` for the caller's `VVideo`). See the
    /// module docs for what isn't applied.
    pub fn apply(&self) -> i32 {
        let g = |n: &str| self.get(n).unwrap();
        let st = doomstat::state_mut();
        st.mouse_sensitivity = g("mouse_sensitivity");
        st.snd_sfx_volume = g("sfx_volume");
        st.snd_music_volume = g("music_volume");
        st.show_messages = g("show_messages");
        st.screenblocks = g("screenblocks");
        st.detail_level = g("detaillevel");
        st.num_channels = g("snd_channels");
        g("usegamma")
    }

    /// The inverse of [`Defaults::apply`]: reads the live values back
    /// (what `M_SaveDefaults` sees through its pointers).
    pub fn capture(usegamma: i32) -> Defaults {
        let mut d = Defaults::default();
        let st = doomstat::state();
        d.set("mouse_sensitivity", st.mouse_sensitivity);
        d.set("sfx_volume", st.snd_sfx_volume);
        d.set("music_volume", st.snd_music_volume);
        d.set("show_messages", st.show_messages);
        d.set("screenblocks", st.screenblocks);
        d.set("detaillevel", st.detail_level);
        d.set("snd_channels", st.num_channels);
        d.set("usegamma", usegamma);
        d
    }
}

/// `basedefault`: `$HOME/.doomrc`, or `default.cfg` without a home
/// (`d_main.c`'s `IdentifyVersion`).
pub fn default_config_path() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".doomrc"),
        None => PathBuf::from("default.cfg"),
    }
}

/// The config file to use: `-config <file>` or [`default_config_path`].
pub fn config_path() -> PathBuf {
    let i = crate::m_argv::m_check_parm("-config");
    match (i != 0).then(|| crate::m_argv::arg(i + 1)).flatten() {
        Some(file) => {
            let path = PathBuf::from(file);
            println!("\tdefault file: {}", path.display());
            path
        }
        None => default_config_path(),
    }
}

/// Port of `M_LoadDefaults`: table defaults, overridden by the config
/// file when it exists.
pub fn m_load_defaults(path: &Path) -> Defaults {
    let mut d = Defaults::default();
    if let Ok(text) = std::fs::read_to_string(path) {
        d.parse(&text);
    }
    d
}

/// Port of `M_SaveDefaults`. A file that can't be written is silently
/// skipped ("can't write the file, but don't complain").
pub fn m_save_defaults(path: &Path, defaults: &Defaults) {
    if let Ok(mut f) = std::fs::File::create(path) {
        let _ = f.write_all(defaults.serialize().as_bytes());
    }
}

/// Port of `WritePCXfile`: run-length-less PCX of a palettised image.
pub fn pcx_bytes(data: &[u8], width: i32, height: i32, palette: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height) as usize + 900);
    out.push(0x0a); // PCX id
    out.push(5); // 256 color
    out.push(1); // "uncompressed"
    out.push(8); // 256 color
    for v in [
        0u16,
        0,
        (width - 1) as u16,
        (height - 1) as u16,
        width as u16,
        height as u16,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0u8; 48]); // palette
    out.push(0); // reserved
    out.push(1); // chunky image
    out.extend_from_slice(&(width as u16).to_le_bytes()); // bytes per line
    out.extend_from_slice(&2u16.to_le_bytes()); // not a grey scale
    out.extend_from_slice(&[0u8; 58]); // filler

    // pack the image
    for &b in &data[..(width * height) as usize] {
        if b & 0xc0 != 0xc0 {
            out.push(b);
        } else {
            out.push(0xc1);
            out.push(b);
        }
    }

    // write the palette
    out.push(0x0c); // palette ID byte
    out.extend_from_slice(&palette[..768]);
    out
}

/// Port of `M_ScreenShot`: writes `DOOMnn.pcx` (first free `nn`) in
/// the current directory and tells the player.
///
/// # Panics
/// If all 100 names are taken (`I_Error`).
pub fn m_screenshot(v: &VVideo, palette: &[u8], plyr: &mut Player) {
    // munge planar buffer to linear (nothing to do: screens[0] is)
    let linear = &v.screens[0];

    // find a file name to save it to
    let name = (0..100)
        .map(|i| format!("DOOM{:02}.pcx", i))
        .find(|n| !Path::new(n).exists())
        .expect("M_ScreenShot: Couldn't create a PCX");

    // save the pcx file
    m_write_file(
        &name,
        &pcx_bytes(linear, SCREENWIDTH, SCREENHEIGHT, palette),
    );

    plyr.message = Some("screen shot");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_original_table() {
        let d = Defaults::default();
        assert_eq!(d.get("mouse_sensitivity"), Some(5));
        assert_eq!(d.get("sfx_volume"), Some(8));
        assert_eq!(d.get("screenblocks"), Some(9));
        assert_eq!(d.get("key_use"), Some(32));
        assert_eq!(d.get("nope"), None);
    }

    #[test]
    fn parse_reads_decimal_hex_octal_and_skips_junk() {
        let mut d = Defaults::default();
        d.parse(
            "sfx_volume\t\t12\nkey_fire 0xae\nscreenblocks 011\nbogus 5\nshow_messages\nmusic_volume -3\nchatmacro0 \"hello\"\nusegamma zzz\n",
        );
        assert_eq!(d.get("sfx_volume"), Some(12));
        assert_eq!(d.get("key_fire"), Some(0xae));
        assert_eq!(d.get("screenblocks"), Some(9)); // octal 011
        assert_eq!(d.get("show_messages"), Some(1), "no value: untouched");
        assert_eq!(d.get("music_volume"), Some(-3));
        assert_eq!(d.get("usegamma"), Some(0), "unparsable: untouched");
    }

    #[test]
    fn serialize_round_trips() {
        let mut d = Defaults::default();
        d.set("sfx_volume", 3);
        d.set("screenblocks", 11);
        let text = d.serialize();
        assert!(text.contains("sfx_volume\t\t3\n"));
        let mut back = Defaults::default();
        back.parse(&text);
        assert_eq!(back, d);
    }

    #[test]
    fn load_and_save_use_the_file() {
        let dir = std::env::temp_dir().join(format!("doommetalrust_cfg_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("default.cfg");
        // missing file: table defaults
        assert_eq!(m_load_defaults(&path), Defaults::default());

        let mut d = Defaults::default();
        d.set("mouse_sensitivity", 9);
        m_save_defaults(&path, &d);
        assert_eq!(m_load_defaults(&path).get("mouse_sensitivity"), Some(9));
        // an unwritable location is silently ignored
        m_save_defaults(&dir.join("no/such/dir/x.cfg"), &d);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_and_read_file() {
        let path = std::env::temp_dir().join(format!("doommetalrust_rw_{}", std::process::id()));
        assert!(m_write_file(&path, b"abc"));
        assert_eq!(m_read_file(&path), b"abc");
        std::fs::remove_file(&path).ok();
        assert!(!m_write_file("/no/such/dir/file", b"x"));
        let r = std::panic::catch_unwind(|| m_read_file("/no/such/file"));
        assert!(r.is_err());
    }

    #[test]
    fn pcx_layout() {
        let data = [1u8, 0xc5, 3, 4];
        let pal: Vec<u8> = (0..768).map(|i| (i % 251) as u8).collect();
        let pcx = pcx_bytes(&data, 2, 2, &pal);
        assert_eq!(&pcx[..4], &[0x0a, 5, 1, 8]);
        assert_eq!(&pcx[8..10], &1u16.to_le_bytes()); // xmax = width-1
                                                      // header is 128 bytes; 0xc5 is escaped as 0xc1 0xc5
        assert_eq!(&pcx[128..133], &[1, 0xc1, 0xc5, 3, 4]);
        assert_eq!(pcx[133], 0x0c);
        assert_eq!(pcx.len(), 128 + 5 + 1 + 768);
    }
}
