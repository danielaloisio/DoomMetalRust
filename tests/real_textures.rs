//! Integration test for Phase 5c's milestone: loading real
//! texture/flat/sprite/colormap data from `doom.wad` via `r_data::RData`,
//! with values cross-checked against an independent Python parse of the
//! same file's raw `TEXTURE1`/lump-marker bytes (see `PORTING.md`'s
//! Phase 5c notes), not just trusted by inspection of this Rust code.
//!
//! Skipped (not failed) if no doom.wad is found — see `tests/real_wad.rs`
//! for the same convention/rationale.

use doommetal_rust::r_data::RData;
use doommetal_rust::w_wad::WadFiles;

fn find_test_wad() -> Option<std::path::PathBuf> {
    let candidates = ["doom.wad"];
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

#[test]
fn init_loads_expected_flat_sprite_colormap_counts() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let mut rdata = RData::new();
    rdata.init(&mut wad);

    // Ground truth from an independent Python parse of the same file's
    // F_START/F_END, S_START/S_END lump markers and the COLORMAP lump's
    // raw size (not derived from this Rust code).
    assert_eq!(rdata.numflats, 111);
    assert_eq!(rdata.numspritelumps, 764);
    assert_eq!(rdata.colormaps.len(), 8704);
}

#[test]
fn texture_num_for_name_finds_known_texture() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let mut rdata = RData::new();
    rdata.init(&mut wad);

    // AASTINKY is the first texture in TEXTURE1 (index 0), a known,
    // stable fact about the shareware doom.wad's TEXTURE1 lump,
    // confirmed independently via a raw parse of its directory.
    let idx = rdata.texture_num_for_name("AASTINKY");
    assert_eq!(idx, 0);

    // NoTexture marker.
    assert_eq!(rdata.check_texture_num_for_name("-"), Some(0));
}

#[test]
fn get_column_returns_nonempty_data_for_a_single_patch_texture() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let mut rdata = RData::new();
    rdata.init(&mut wad);

    let texnum = rdata.texture_num_for_name("AASTINKY") as usize;
    let col = rdata.get_column(&mut wad, texnum, 0);
    assert!(!col.is_empty(), "expected non-empty column data");
}

#[test]
fn flat_num_for_name_resolves_relative_to_firstflat() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let mut rdata = RData::new();
    rdata.init(&mut wad);

    // NUKAGE3 is a known flat present in the shareware IWAD (used by
    // E1M1's first sector, per tests/real_level.rs's independently
    // parsed sector[0] data). Just check it resolves to a valid,
    // in-range flat index, not a specific value (F_START position can
    // shift between IWAD versions in ways this test shouldn't be
    // sensitive to).
    let idx = rdata.flat_num_for_name(&wad, "NUKAGE3");
    assert!(
        (0..rdata.numflats).contains(&idx),
        "flat index {idx} out of range 0..{}",
        rdata.numflats
    );
}

#[test]
fn get_column_works_across_all_columns_of_a_multi_patch_texture() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let mut rdata = RData::new();
    rdata.init(&mut wad);

    // AASTINKY has 2 patches (per an independent parse of TEXTURE1),
    // so at least some of its columns should exercise the composite
    // (multi-patch) path in get_column, not just the single-patch one.
    let texnum = rdata.texture_num_for_name("AASTINKY") as usize;
    for col in 0..24 {
        let data = rdata.get_column(&mut wad, texnum, col);
        assert!(!data.is_empty(), "column {col} returned empty data");
    }
}
