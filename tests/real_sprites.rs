//! Integration tests for Phase 5e's `r_things.rs` port against the real
//! doom.wad: `R_InitSprites` over the full `sprnames[]` list (every
//! sprite's frame/rotation lumps must be consistent, or it panics like
//! the original's `I_Error`), and `R_DrawMasked` drawing a real sprite
//! in front of / clipped behind a real E1M1 wall drawn by
//! `R_StoreWallRange`.
//!
//! Skipped (not failed) if no doom.wad is found — same convention as
//! the other `tests/real_*.rs` files.

use doommetal_rust::m_fixed::{fixed_mul, FRACUNIT};
use doommetal_rust::p_setup::Level;
use doommetal_rust::r_data::RData;
use doommetal_rust::r_defs::{DrawSeg, Mobj, SIL_BOTH};
use doommetal_rust::r_draw::RDraw;
use doommetal_rust::r_main::RMain;
use doommetal_rust::r_plane::RPlane;
use doommetal_rust::r_segs::RSegs;
use doommetal_rust::r_state::RState;
use doommetal_rust::r_things::{MaskedDrawContext, RThings, ViewParams};
use doommetal_rust::tables::{fine_cosine, Angle, ANG180, ANG90, ANGLETOFINESHIFT, FINESINE};
use doommetal_rust::w_wad::WadFiles;
use doommetal_rust::z_zone::PurgeTag;

mod common;
use common::{blank_mobj, find_test_wad, save_palettized_png, scratch_dir, sprite_num, SPRNAMES};

#[test]
fn r_init_sprites_accepts_every_sprite_in_the_real_wad() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    let mut wad = WadFiles::new();
    wad.init_file(&path);
    let mut rdata = RData::new();
    rdata.init(&mut wad);

    let mut things = RThings::new();
    things.r_init_sprites(&wad, &rdata, &SPRNAMES, false);
    assert_eq!(things.sprites.len(), SPRNAMES.len());

    let frames = |name: &str| &things.sprites[sprite_num(name) as usize].spriteframes;

    // Imp: frames A..U, walking frames drawn from 8 rotations (some
    // via flipped lumps, e.g. TROOA2A8).
    assert_eq!(frames("TROO").len(), 21);
    let troo_a = frames("TROO")[0];
    assert!(troo_a.rotate);
    assert!(troo_a.lump.iter().all(|&l| l >= 0));
    assert!(
        troo_a.flip.contains(&1),
        "TROOA's right-facing rotations come from flipped lumps"
    );

    // Player: frames A..W.
    assert_eq!(frames("PLAY").len(), 23);

    // Barrel: two single-rotation frames, same lump for all 8 views.
    let bar1 = frames("BAR1");
    assert_eq!(bar1.len(), 2);
    assert!(!bar1[0].rotate);
    assert!(bar1[0].lump.iter().all(|&l| l == bar1[0].lump[0]));

    // Doom II-only sprites have no lumps in a Doom 1 IWAD: no frames,
    // not an error (the original's `maxframe == -1` case).
    assert!(frames("VILE").is_empty());
    assert!(frames("FATT").is_empty());
}

/// Everything needed to draw one real E1M1 wall with a sprite near it.
struct Scene {
    wad: WadFiles,
    rdata: RData,
    level: Level,
    rmain: RMain,
    rplane: RPlane,
    rsegs: RSegs,
    rdraw: RDraw,
    things: RThings,
    view: ViewParams,
    screen: Vec<u8>,
    drawsegs: Vec<DrawSeg>,
    /// Wall midpoint and the direction from the wall back toward the
    /// viewer.
    midx: i32,
    midy: i32,
    back_angle: Angle,
    floorheight: i32,
}

/// Same viewpoint as `tests/real_wall_range.rs`: 128 units in front of a
/// real single-sided wall's midpoint, looking straight at it. Draws the
/// wall over the whole view width and keeps its drawseg.
fn wall_scene(path: &std::path::Path) -> Scene {
    let mut wad = WadFiles::new();
    wad.init_file(path);
    let mut level = Level::load(&mut wad, "E1M1");
    let mut rdata = RData::new();
    rdata.init(&mut wad);
    level.resolve_textures(&rdata, &wad);

    let mut things = RThings::new();
    things.r_init_sprites(&wad, &rdata, &SPRNAMES, false);

    let seg_idx = level
        .segs
        .iter()
        .position(|s| s.backsector.is_none() && level.sides[s.sidedef].midtexture != 0)
        .expect("E1M1 must have at least one single-sided textured seg");
    let seg = level.segs[seg_idx];
    let v1 = level.vertexes[seg.v1];
    let v2 = level.vertexes[seg.v2];
    let floorheight = level.sectors[seg.frontsector].floorheight;

    let midx = (v1.x + v2.x) / 2;
    let midy = (v1.y + v2.y) / 2;
    let rw_normalangle = seg.angle.wrapping_add(ANG90);
    let back_angle = rw_normalangle.wrapping_add(ANG180);

    let mut rmain = RMain::new();
    let (cos, sin) = dir(back_angle);
    rmain.viewx = midx + fixed_mul(128 * FRACUNIT, cos);
    rmain.viewy = midy + fixed_mul(128 * FRACUNIT, sin);
    rmain.viewz = floorheight + 41 * FRACUNIT;
    rmain.viewangle = rw_normalangle;
    (rmain.viewcos, rmain.viewsin) = dir(rw_normalangle);
    rmain.init_light_tables(&rdata);

    let view = ViewParams::new(320, 200, 0);

    let mut rstate = RState::default();
    rstate.r_init_texture_mapping(view.centerxfrac, view.viewwidth);

    let mut rplane = RPlane::new();
    rplane.r_clear_planes(&rmain, view.viewwidth, view.viewheight);

    let mut rsegs = RSegs::new();
    rsegs.rw_angle1 = rmain.point_to_angle(v1.x, v1.y);

    let mut rdraw = RDraw::new();
    rdraw.r_init_buffer(view.viewwidth, view.viewheight);
    let mut screen = vec![0u8; (view.viewwidth * view.viewheight) as usize];

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
        view.viewwidth - 1,
        view.viewwidth,
        view.viewheight,
        view.centeryfrac,
        view.projection,
        0,
        None,
        -1,
        &mut floorplane,
        &mut ceilingplane,
    );
    assert_eq!(ds.silhouette, SIL_BOTH);

    Scene {
        wad,
        rdata,
        level,
        rmain,
        rplane,
        rsegs,
        rdraw,
        things,
        view,
        screen,
        drawsegs: vec![ds],
        midx,
        midy,
        back_angle,
        floorheight,
    }
}

fn dir(angle: Angle) -> (i32, i32) {
    let fine = (angle >> ANGLETOFINESHIFT) as usize;
    (fine_cosine(fine), FINESINE[fine])
}

/// An Imp (frame A) standing on the wall's floor, `dist` units from the
/// wall toward the viewer (negative = behind the wall), facing the
/// viewer.
fn imp(scene: &Scene, dist: i32) -> Mobj {
    let (cos, sin) = dir(scene.back_angle);
    let mut mobj = blank_mobj();
    mobj.x = scene.midx + fixed_mul(dist * FRACUNIT, cos);
    mobj.y = scene.midy + fixed_mul(dist * FRACUNIT, sin);
    mobj.z = scene.floorheight;
    mobj.angle = scene.back_angle;
    mobj.sprite = doommetal_rust::info::SpriteNum::SprTroo;
    mobj
}

/// Projects `mobj` and runs `R_DrawMasked` over the scene's drawsegs.
/// Returns the indices of the pixels that changed.
fn draw_masked_with(scene: &mut Scene, mobj: &Mobj) -> Vec<usize> {
    scene.things.r_clear_sprites();
    scene
        .things
        .r_project_sprite(mobj, &scene.rmain, &scene.rdata, &scene.view);
    assert_eq!(
        scene.things.vissprites.len(),
        1,
        "the Imp should be in view"
    );

    let before = scene.screen.clone();
    let mut ctx = MaskedDrawContext {
        rmain: &scene.rmain,
        rdata: &mut scene.rdata,
        wad: &mut scene.wad,
        level: &scene.level,
        rplane: &mut scene.rplane,
        rsegs: &mut scene.rsegs,
        rdraw: &mut scene.rdraw,
        screen: &mut scene.screen,
        view: &scene.view,
    };
    let drawsegs = scene.drawsegs.clone();
    scene.things.r_draw_masked(&drawsegs, None, &mut ctx);

    before
        .iter()
        .zip(&scene.screen)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn sprite_in_front_of_wall_is_drawn_and_behind_it_is_clipped() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };

    // Behind the wall: the wall's drawseg (solid, SIL_BOTH, bigger
    // scale) clips every sprite column to nothing.
    let mut scene = wall_scene(&path);
    let behind = imp(&scene, -64);
    assert!(draw_masked_with(&mut scene, &behind).is_empty());

    // In front of the wall: the drawseg is behind the sprite, so it
    // doesn't clip and the Imp's opaque pixels land on the wall.
    let mut scene = wall_scene(&path);
    let front = imp(&scene, 64);
    let changed = draw_masked_with(&mut scene, &front);
    let vis = scene.things.vissprites[0];
    let width = (vis.x2 - vis.x1 + 1) as usize;
    assert!(
        changed.len() > width * 10,
        "expected the Imp to cover a real area of the wall, only {} pixels changed",
        changed.len()
    );

    // Everything drawn stays inside the sprite's projected columns.
    let viewwidth = scene.view.viewwidth;
    for &i in &changed {
        let x = i as i32 % viewwidth;
        assert!(
            (vis.x1..=vis.x2).contains(&x),
            "pixel changed at column {x}, outside the sprite's {}..={}",
            vis.x1,
            vis.x2
        );
    }

    // Save the frame (through the real PLAYPAL) for visual review.
    let playpal = scene
        .wad
        .cache_lump_name("PLAYPAL", PurgeTag::Cache)
        .to_vec();
    let out_path = scratch_dir().join("doommetalrust_r_things_imp.png");
    save_palettized_png(
        &out_path,
        viewwidth as u32,
        scene.view.viewheight as u32,
        &scene.screen,
        &playpal,
    );
    eprintln!("sprite frame written to {}", out_path.display());
}
