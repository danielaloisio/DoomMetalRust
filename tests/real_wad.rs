//! Integration test against the real `doom.wad` shareware/retail IWAD,
//! verifying Phase 3's milestone: "consegue carregar e listar lumps do
//! WAD" (load and list WAD lumps), with counts/names cross-checked
//! against an independent Python parse of the same file's raw
//! header/directory bytes (see PORTING.md's Phase 3 notes), not just
//! trusted by inspection of this Rust code.
//!
//! Skipped (not failed) if no doom.wad is found, since the IWAD is not
//! part of this repository (see `.gitignore`) and isn't guaranteed to be
//! present in every environment this test suite runs in.

use doommetal_rust::w_wad::WadFiles;

fn find_test_wad() -> Option<std::path::PathBuf> {
    let candidates = ["doom.wad"];
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

#[test]
fn loads_real_doom_wad_with_expected_lump_count_and_names() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    // Ground truth from an independent Python parse of the same file's
    // header/directory bytes (not derived from this Rust code).
    assert_eq!(wad.num_lumps(), 2306);

    let playpal = wad.get_num_for_name("PLAYPAL");
    assert_eq!(playpal, 0);
    assert_eq!(wad.lump_length(playpal), 10752);

    let colormap = wad.get_num_for_name("COLORMAP");
    assert_eq!(colormap, 1);
    assert_eq!(wad.lump_length(colormap), 8704);

    let e1m1 = wad.get_num_for_name("E1M1");
    assert_eq!(e1m1, 7);
    assert_eq!(wad.lump_length(e1m1), 0);

    assert_eq!(wad.check_num_for_name("F_END"), Some(2305));
}

#[test]
fn reads_playpal_lump_bytes_and_matches_length() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let playpal = wad.get_num_for_name("PLAYPAL");
    let len = wad.lump_length(playpal);
    let mut buf = vec![0u8; len];
    wad.read_lump(playpal, &mut buf);

    assert_eq!(buf.len(), 10752);
    // PLAYPAL is 14 palettes of 256 RGB triples = 14*256*3 = 10752 bytes,
    // a well known structural fact about this lump — sanity-checks that
    // we read real palette data, not zeros/garbage.
    assert_eq!(len, 14 * 256 * 3);
    assert!(
        buf.iter().any(|&b| b != 0),
        "PLAYPAL bytes should not be all zero"
    );
}

#[test]
fn cache_lump_num_returns_same_bytes_as_read_lump() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let colormap = wad.get_num_for_name("COLORMAP");
    let len = wad.lump_length(colormap);
    let mut direct = vec![0u8; len];
    wad.read_lump(colormap, &mut direct);

    let cached = wad
        .cache_lump_num(colormap, doommetal_rust::z_zone::PurgeTag::Cache)
        .to_vec();

    assert_eq!(direct, cached);
}
