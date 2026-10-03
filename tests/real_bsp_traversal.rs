//! Integration test for Phase 5d's pending milestone: exercising
//! [`RBsp::render_bsp_node`] against the real E1M1 level end-to-end —
//! confirming the BSP traversal runs over real map data without panics
//! and visits a plausible portion of the tree, ahead of Phase 5e wiring
//! up actual rasterization.
//!
//! Skipped (not failed) if no doom.wad is found — same convention as
//! `tests/real_level.rs`/`tests/real_wad.rs`.

use doommetal_rust::p_setup::Level;
use doommetal_rust::r_bsp::{RBsp, RecordingHooks};
use doommetal_rust::r_main::RMain;
use doommetal_rust::r_state::RState;
use doommetal_rust::w_wad::WadFiles;

fn find_test_wad() -> Option<std::path::PathBuf> {
    let candidates = ["doom.wad"];
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

/// The map has no ported `P_LoadThings`/player-start yet (Phase 6), so
/// the view position is derived from the BSP tree itself instead: the
/// center of the root node's first child bbox is guaranteed to sit
/// inside the map's convex bounds, giving a deterministic, always-valid
/// viewpoint for exercising traversal without depending on unported
/// game data.
fn viewpoint_inside_map(level: &Level) -> (i32, i32) {
    use doommetal_rust::m_bbox::{BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};

    let root = level.nodes.last().expect("level has at least one node");
    let bbox = root.bbox[0];
    let x = (bbox[BOXLEFT] + bbox[BOXRIGHT]) / 2;
    let y = (bbox[BOXBOTTOM] + bbox[BOXTOP]) / 2;
    (x, y)
}

/// `viewangletox`/`clipangle` for the full-screen (320-wide) view, as
/// `R_ExecuteSetViewSize` sets them up.
fn full_screen_rstate() -> RState {
    let mut rstate = RState::default();
    rstate.r_init_texture_mapping(160 * doommetal_rust::m_fixed::FRACUNIT, 320);
    rstate
}

#[test]
fn render_bsp_node_traverses_e1m1_without_panicking() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);
    let mut level = Level::load(&mut wad, "E1M1");

    let (viewx, viewy) = viewpoint_inside_map(&level);

    let mut rmain = RMain::new();
    rmain.viewx = viewx;
    rmain.viewy = viewy;
    rmain.viewz = 41 * doommetal_rust::m_fixed::FRACUNIT;
    rmain.viewangle = 0;
    let rstate = full_screen_rstate();

    let mut bsp = RBsp::new();
    bsp.clear_clip_segs(doommetal_rust::doomdef::SCREENWIDTH);
    let mut hooks = RecordingHooks::default();

    let root_bspnum = (level.nodes.len() - 1) as i32;
    bsp.render_bsp_node(&mut hooks, &rmain, &rstate, &mut level, root_bspnum);

    // The traversal must visit at least one subsector (sscount > 0),
    // and every subsector visited must be a real index into the level's
    // subsector array (no out-of-range panics happened above, but this
    // also confirms genuine work was done, not a degenerate 0-node
    // tree).
    assert!(
        bsp.sscount > 0,
        "expected at least one subsector to be visited"
    );
    assert!(bsp.sscount as usize <= level.subsectors.len());

    // find_plane is called once per visible floor/ceiling plane per
    // subsector reached, so seeing calls here confirms subsector()
    // actually ran its floor/ceiling logic, not just recursed past it.
    assert!(
        !hooks.find_plane_calls.is_empty(),
        "expected at least one find_plane call from visited subsectors"
    );

    // add_sprites is called once per visited subsector's sector.
    assert!(!hooks.sprite_sectors.is_empty());
}

#[test]
fn render_bsp_node_from_multiple_viewpoints_stays_in_range() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);
    let mut level = Level::load(&mut wad, "E1M1");

    use doommetal_rust::m_bbox::{BOXBOTTOM, BOXLEFT, BOXRIGHT, BOXTOP};
    let root = *level.nodes.last().expect("level has at least one node");

    // Exercise both of the root node's child bboxes as viewpoints (the
    // "front" and "back" half of the map from the root partition line),
    // rather than only the single point the other test uses.
    for child in 0..2 {
        let bbox = root.bbox[child];
        let viewx = (bbox[BOXLEFT] + bbox[BOXRIGHT]) / 2;
        let viewy = (bbox[BOXBOTTOM] + bbox[BOXTOP]) / 2;

        let mut rmain = RMain::new();
        rmain.viewx = viewx;
        rmain.viewy = viewy;
        rmain.viewz = 41 * doommetal_rust::m_fixed::FRACUNIT;
        rmain.viewangle = 0;
        let rstate = full_screen_rstate();

        let mut bsp = RBsp::new();
        bsp.clear_clip_segs(doommetal_rust::doomdef::SCREENWIDTH);
        let mut hooks = RecordingHooks::default();

        let root_bspnum = (level.nodes.len() - 1) as i32;
        bsp.render_bsp_node(&mut hooks, &rmain, &rstate, &mut level, root_bspnum);

        assert!(
            bsp.sscount > 0,
            "child {child}: expected subsectors visited"
        );
        assert!(bsp.sscount as usize <= level.subsectors.len());
    }
}
