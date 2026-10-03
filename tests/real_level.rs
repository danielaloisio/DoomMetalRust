//! Integration test for Phase 5b's milestone: loading a real map level
//! (E1M1) from the real `doom.wad` IWAD via `p_setup::Level::load`, with
//! counts cross-checked against an independent Python parse of the same
//! file's raw lump directory/header bytes (see `PORTING.md`'s Phase 5b
//! notes), not just trusted by inspection of this Rust code.
//!
//! Skipped (not failed) if no doom.wad is found — see `tests/real_wad.rs`
//! for the same convention/rationale.

use doommetal_rust::p_setup::Level;
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
fn loads_e1m1_with_expected_counts() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let level = Level::load(&mut wad, "E1M1");

    // Ground truth from an independent Python parse of the same file's
    // lump directory/header bytes (not derived from this Rust code):
    // lump size / on-disk record size for each E1M1 map lump.
    assert_eq!(level.vertexes.len(), 470);
    assert_eq!(level.lines.len(), 486);
    assert_eq!(level.sides.len(), 666);
    assert_eq!(level.sectors.len(), 88);
    assert_eq!(level.subsectors.len(), 239);
    assert_eq!(level.nodes.len(), 238);
    assert_eq!(level.segs.len(), 747);
}

#[test]
fn every_sector_line_actually_references_that_sector() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let level = Level::load(&mut wad, "E1M1");

    // Cross-check P_GroupLines's output structurally: every line index
    // recorded in a sector's `lines` must actually have that sector as
    // its front or back sector.
    for (sector_idx, sector) in level.sectors.iter().enumerate() {
        assert!(
            !sector.lines.is_empty(),
            "sector {sector_idx} has no lines grouped into it"
        );
        for &line_idx in &sector.lines {
            let line = &level.lines[line_idx];
            assert!(
                line.frontsector == Some(sector_idx) || line.backsector == Some(sector_idx),
                "sector {sector_idx} claims line {line_idx}, but that line doesn't reference it"
            );
        }
    }
}

#[test]
fn every_line_with_a_sector_is_grouped_into_it() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let level = Level::load(&mut wad, "E1M1");

    // The reverse direction: every line's front/back sector must list
    // that line among its grouped lines (P_GroupLines's completeness,
    // not just its correctness above).
    for (line_idx, line) in level.lines.iter().enumerate() {
        if let Some(front) = line.frontsector {
            assert!(
                level.sectors[front].lines.contains(&line_idx),
                "line {line_idx}'s frontsector {front} doesn't list it back"
            );
        }
        if let Some(back) = line.backsector {
            assert!(
                level.sectors[back].lines.contains(&line_idx),
                "line {line_idx}'s backsector {back} doesn't list it back"
            );
        }
    }
}

#[test]
fn vertex_coordinates_are_within_a_plausible_map_range() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let level = Level::load(&mut wad, "E1M1");

    // E1M1's vertex coordinates are known (from the original DOOM 1
    // shareware map) to be well within +/-10000 map units; converted to
    // fixed_t (<<16), sanity-check that load_vertexes's SHORT-then-shift
    // conversion didn't overflow/wrap or leave raw non-fixed-point
    // values in place.
    let max_fixed = 10_000i32 << 16;
    for v in &level.vertexes {
        assert!(
            v.x.abs() < max_fixed && v.y.abs() < max_fixed,
            "vertex ({}, {}) outside plausible range for E1M1",
            v.x >> 16,
            v.y >> 16
        );
    }
}

#[test]
fn subsector_sectors_are_populated_by_group_lines() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let level = Level::load(&mut wad, "E1M1");

    // Every subsector's `sector` index must be in range — P_GroupLines
    // resolves it from the subsector's first seg's sidedef, which
    // should always point at a valid sector for a well-formed WAD.
    for (i, ss) in level.subsectors.iter().enumerate() {
        assert!(
            ss.sector < level.sectors.len(),
            "subsector {i} has out-of-range sector index {}",
            ss.sector
        );
    }
}

#[test]
fn first_vertices_and_sector_match_independently_parsed_values() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);

    let level = Level::load(&mut wad, "E1M1");

    // Ground truth from an independent Python parse of the raw
    // VERTEXES/SECTORS lump bytes for E1M1 (not derived from this Rust
    // code) — exact fixed_t values, confirming the <<FRACBITS
    // conversion is bit-for-bit correct, not just "plausible range".
    assert_eq!(level.vertexes[0].x, 71303168);
    assert_eq!(level.vertexes[0].y, -241172480);
    assert_eq!(level.vertexes[1].x, 67108864);
    assert_eq!(level.vertexes[1].y, -241172480);
    assert_eq!(level.vertexes[2].x, 67108864);
    assert_eq!(level.vertexes[2].y, -239075328);

    assert_eq!(level.sectors[0].floorheight, -80 << 16);
    assert_eq!(level.sectors[0].ceilingheight, 216 << 16);
    assert_eq!(level.sectors[0].lightlevel, 255);
    assert_eq!(level.sectors[0].special, 7);
    assert_eq!(level.sectors[0].tag, 0);
}

#[test]
fn resolve_textures_matches_independently_parsed_names() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);
    let mut level = Level::load(&mut wad, "E1M1");
    let mut rdata = RData::new();
    rdata.init(&mut wad);
    level.resolve_textures(&rdata, &wad);

    // Ground truth from an independent Python parse of the raw
    // SECTORS/SIDEDEFS lump bytes for E1M1 (not derived from this Rust
    // code): sector 0 is floorpic "NUKAGE3" / ceilingpic "F_SKY1";
    // sidedef 0 is top "-" / bottom "-" / mid "DOOR3" (top/bottom "-"
    // means "no texture", resolved to index 0 per
    // R_CheckTextureNumForName's NoTexture marker, not
    // TEXTURE_UNRESOLVED).
    assert_eq!(
        level.sectors[0].floorpic as i32,
        rdata.flat_num_for_name(&wad, "NUKAGE3")
    );
    assert_eq!(
        level.sectors[0].ceilingpic as i32,
        rdata.flat_num_for_name(&wad, "F_SKY1")
    );

    assert_eq!(level.sides[0].toptexture, 0);
    assert_eq!(level.sides[0].bottomtexture, 0);
    assert_eq!(
        level.sides[0].midtexture as i32,
        rdata.texture_num_for_name("DOOR3")
    );

    // No sector/side should be left at TEXTURE_UNRESOLVED (-1) after
    // resolve_textures runs, across the whole level, not just index 0.
    for sector in &level.sectors {
        assert_ne!(sector.floorpic, -1);
        assert_ne!(sector.ceilingpic, -1);
    }
    for side in &level.sides {
        assert_ne!(side.toptexture, -1);
        assert_ne!(side.bottomtexture, -1);
        assert_ne!(side.midtexture, -1);
    }
}
