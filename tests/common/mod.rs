//! Helpers shared by the `tests/real_*.rs` integration tests.

#![allow(dead_code)]

use doommetal_rust::r_defs::Mobj;

pub use doommetal_rust::info::SPRNAMES;

/// Index of `name` in [`SPRNAMES`] (its `spritenum_t`).
pub fn sprite_num(name: &str) -> i32 {
    SPRNAMES.iter().position(|&n| n == name).unwrap() as i32
}

pub fn find_test_wad() -> Option<std::path::PathBuf> {
    let candidates = ["doom.wad"];
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

pub fn scratch_dir() -> std::path::PathBuf {
    std::env::var_os("DOOMMETALRUST_SCRATCH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// Writes a palettized frame as an RGB PNG through a `PLAYPAL` palette.
pub fn save_palettized_png(
    path: &std::path::Path,
    width: u32,
    height: u32,
    pixels: &[u8],
    playpal: &[u8],
) {
    let rgb: Vec<u8> = pixels
        .iter()
        .flat_map(|&c| playpal[c as usize * 3..c as usize * 3 + 3].to_vec())
        .collect();
    let file = std::fs::File::create(path).expect("failed to create screenshot file");
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("failed to write PNG header");
    writer
        .write_image_data(&rgb)
        .expect("failed to write PNG data");
}

/// A mobj with every field zeroed/empty, for tests to fill in. See
/// [`Mobj::blank`].
pub fn blank_mobj() -> Mobj {
    Mobj::blank(doommetal_rust::info::MobjType::MtPlayer)
}
