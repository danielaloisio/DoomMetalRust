//! Integration test for Phase 5e's `r_segs.rs` port: exercising
//! `RSegs::r_store_wall_range` against a real single-sided seg from
//! E1M1, with real texture data resolved via `RData`/`WadFiles` — not
//! just synthetic in-memory structs, since the interesting part of
//! `R_StoreWallRange` (texture boundary/midtexture selection, scale
//! calculation) only exercises real code paths when there's an actual
//! `TEXTURE1`/`TEXTURE2` texture behind `sidedef.midtexture`.
//!
//! Skipped (not failed) if no doom.wad is found — same convention as
//! `tests/real_level.rs`/`tests/real_bsp_traversal.rs`.

use doommetal_rust::m_fixed::FRACUNIT;
use doommetal_rust::p_setup::Level;
use doommetal_rust::r_data::RData;
use doommetal_rust::r_defs::SIL_BOTH;
use doommetal_rust::r_draw::RDraw;
use doommetal_rust::r_main::RMain;
use doommetal_rust::r_plane::RPlane;
use doommetal_rust::r_segs::RSegs;
use doommetal_rust::r_state::RState;
use doommetal_rust::w_wad::WadFiles;

fn find_test_wad() -> Option<std::path::PathBuf> {
    let candidates = ["doom.wad"];
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

#[test]
fn r_store_wall_range_draws_a_real_single_sided_seg_without_panicking() {
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

    // Find a real single-sided seg (backsector == None) with a nonzero
    // midtexture on its sidedef — guaranteed to exist in E1M1 (every
    // level has solid outer walls).
    let seg_idx = level
        .segs
        .iter()
        .position(|s| s.backsector.is_none() && level.sides[s.sidedef].midtexture != 0)
        .expect("E1M1 must have at least one single-sided textured seg");

    let seg = level.segs[seg_idx];
    let v1 = level.vertexes[seg.v1];
    let v2 = level.vertexes[seg.v2];
    let sector = level.sectors[seg.frontsector].clone();

    // Place the viewpoint squarely in front of the wall's midpoint,
    // facing directly along the wall's own outward normal
    // (seg.angle + ANG90, same as R_StoreWallRange's rw_normalangle)
    // rather than an arbitrary fixed direction — this guarantees
    // rw_centerangle lands near ANG90 (see r_segs.rs's
    // R_RenderSegLoop-folded-in loop: `finetangent[(rw_centerangle +
    // xtoviewangle[x]) >> ANGLETOFINESHIFT]` requires that shifted
    // value to stay within 0..4096, which only holds when the wall is
    // roughly centered in the view — exactly what a real BSP-traversal
    // caller would have already ensured via R_AddLine/R_CheckBBox
    // before ever reaching R_StoreWallRange).
    let midx = (v1.x + v2.x) / 2;
    let midy = (v1.y + v2.y) / 2;
    let rw_normalangle = seg.angle.wrapping_add(doommetal_rust::tables::ANG90);

    // Step back from the wall along its inward normal (opposite of
    // rw_normalangle) by 128 map units, then look back along
    // rw_normalangle toward the wall.
    let back_angle =
        (rw_normalangle as i32).wrapping_add(doommetal_rust::tables::ANG180 as i32) as u32;
    let step = 128 * FRACUNIT;
    let cos = doommetal_rust::tables::fine_cosine(
        (back_angle >> doommetal_rust::tables::ANGLETOFINESHIFT) as usize,
    );
    let sin = doommetal_rust::tables::FINESINE
        [(back_angle >> doommetal_rust::tables::ANGLETOFINESHIFT) as usize];

    let mut rmain = RMain::new();
    rmain.viewx = midx + doommetal_rust::m_fixed::fixed_mul(step, cos);
    rmain.viewy = midy + doommetal_rust::m_fixed::fixed_mul(step, sin);
    rmain.viewz = sector.floorheight + 41 * FRACUNIT;
    rmain.viewangle = rw_normalangle;
    rmain.init_light_tables(&rdata);

    let mut rstate = RState::default();
    let viewwidth = 320;
    let viewheight = 200;
    let centerxfrac = (viewwidth / 2) * FRACUNIT;
    rstate.r_init_texture_mapping(centerxfrac, viewwidth);

    let mut rplane = RPlane::new();
    rplane.r_clear_planes(&rmain, viewwidth, viewheight);

    let mut rsegs = RSegs::new();
    rsegs.rw_angle1 = rmain.point_to_angle(v1.x, v1.y);

    let mut rdraw = RDraw::new();
    rdraw.r_init_buffer(viewwidth, viewheight);
    let mut screen = vec![0u8; (viewwidth * viewheight) as usize];

    let centeryfrac = (viewheight / 2) * FRACUNIT;
    let projection = centerxfrac;

    // R_Subsector's visplanes for the wall's sector, which
    // R_StoreWallRange marks into.
    let front = &level.sectors[seg.frontsector];
    let mut floorplane = (front.floorheight < rmain.viewz).then(|| {
        rplane.r_find_plane(
            front.floorheight,
            front.floorpic as i32,
            front.lightlevel as i32,
            -1,
        )
    });
    let mut ceilingplane = (front.ceilingheight > rmain.viewz).then(|| {
        rplane.r_find_plane(
            front.ceilingheight,
            front.ceilingpic as i32,
            front.lightlevel as i32,
            -1,
        )
    });

    let ds = rsegs.r_store_wall_range(
        &rmain,
        &rstate,
        &mut rplane,
        &mut rdata,
        &mut wad,
        &mut level,
        &mut rdraw,
        &mut screen,
        seg_idx,
        0,
        viewwidth - 1,
        viewwidth,
        viewheight,
        centeryfrac,
        projection,
        0,
        None,
        -1,
        &mut floorplane,
        &mut ceilingplane,
    );

    // A single-sided line always gets SIL_BOTH (see R_StoreWallRange's
    // `if (!backsector)` branch) and floor/ceiling marks (it's terminal
    // per the original's own comment).
    assert_eq!(ds.silhouette, SIL_BOTH);
    assert_eq!(ds.curline, seg_idx);
    assert_eq!(ds.x1, 0);
    assert_eq!(ds.x2, viewwidth - 1);

    // ML_MAPPED must have been set on the linedef (marks it visible on
    // the automap).
    assert_ne!(
        level.lines[seg.linedef].flags & doommetal_rust::doomdata::ML_MAPPED,
        0
    );

    // At least some pixels should have been written to the screen
    // buffer (the wall is a solid single-sided line directly in front
    // of the viewer, so R_DrawColumn must have run for at least one
    // column).
    assert!(
        screen.iter().any(|&b| b != 0),
        "expected at least one nonzero pixel after drawing a real wall"
    );
}
