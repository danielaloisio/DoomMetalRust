//! Phase 5's closing milestone: a complete E1M1 frame from player 1's
//! start, through `R_RenderPlayerView` — BSP traversal, walls, floors,
//! ceilings, sky, the level's things as sprites, and the player's
//! pistol — saved as a PNG for visual review.
//!
//! The level's things are spawned by the real `p_setup::p_load_things`/
//! `p_mobj::p_spawn_mobj` (Phase 6b) into a real `p_tick::Thinkers`
//! arena — no more hand-rolled stand-in. Player 1's mobj is found by
//! scanning the arena for the first `MT_PLAYER` (mirroring how a
//! `player_t.mo` would be looked up before `player_t` itself is
//! ported — see `p_setup`'s module docs on that gap).
//!
//! Skipped (not failed) if no doom.wad is found — same convention as
//! the other `tests/real_*.rs` files.

use doommetal_rust::info::MobjType;
use doommetal_rust::m_fixed::{Fixed, FRACUNIT};
use doommetal_rust::p_setup::{self, Level};
use doommetal_rust::p_tick::{ThinkerId, Thinkers};
use doommetal_rust::r_defs::Mobj;
use doommetal_rust::r_main::{Renderer, ViewPlayer};
use doommetal_rust::r_things::{PSprite, PlayerSprites};
use doommetal_rust::tables::{Angle, ANG90};
use doommetal_rust::w_wad::WadFiles;
use doommetal_rust::z_zone::PurgeTag;

mod common;
use common::{find_test_wad, save_palettized_png, scratch_dir, sprite_num, SPRNAMES};

/// (`VIEWHEIGHT`, `p_local.h`)
const VIEWHEIGHT: Fixed = 41 * FRACUNIT;
/// (`WEAPONTOP`, `p_pspr.c`)
const WEAPONTOP: Fixed = 32 * FRACUNIT;

/// Everything a frame needs, loaded once.
struct World {
    wad: WadFiles,
    level: Level,
    renderer: Renderer,
    thinkers: Thinkers,
    player_mo: ThinkerId,
}

fn load_world(path: &std::path::Path) -> World {
    let mut wad = WadFiles::new();
    wad.init_file(path);

    // R_Init with full-screen view (setblocks 11: no status bar, which
    // isn't ported yet), high detail.
    let mut renderer = Renderer::r_init(&mut wad, 11, 0);
    renderer
        .rthings
        .r_init_sprites(&wad, &renderer.rdata, &SPRNAMES, false);

    let mut level = Level::load(&mut wad, "E1M1");
    level.resolve_textures(&renderer.rdata, &wad);

    // G_DoLoadLevel's sky setup for episode 1.
    renderer.sky.skyflatnum = renderer.rdata.flat_num_for_name(&wad, "F_SKY1");
    renderer.sky.skytexture = renderer.rdata.texture_num_for_name("SKY1");

    // P_SetupLevel's P_InitThinkers + P_LoadThings (player 1 is in the
    // game, as D_CheckNetGame leaves it).
    doommetal_rust::doomstat::state_mut().playeringame[0] = true;
    let mut thinkers = Thinkers::new();
    let things_lump = wad.get_num_for_name("E1M1") + 1;
    p_setup::p_load_things(&mut thinkers, &mut level, &mut wad, things_lump);

    let player_mo = thinkers
        .iter_mobjs()
        .find(|(_, m)| m.mobj_type == MobjType::MtPlayer)
        .map(|(id, _)| id)
        .expect("E1M1 has a player 1 start");

    World {
        wad,
        level,
        renderer,
        thinkers,
        player_mo,
    }
}

impl World {
    fn player(&self) -> &Mobj {
        self.thinkers.mobj(self.player_mo).unwrap()
    }

    /// Player 1 standing at their start, facing `angle`, holding the
    /// pistol at rest.
    fn view_player(&self, angle: Angle) -> ViewPlayer {
        let player = self.player();
        let sector = self.level.subsectors[player.subsector.unwrap()].sector;
        ViewPlayer {
            x: player.x,
            y: player.y,
            angle,
            viewz: player.z + VIEWHEIGHT,
            extralight: 0,
            fixedcolormap: 0,
            sprites: PlayerSprites {
                sector_lightlevel: self.level.sectors[sector].lightlevel as i32,
                invisibility: 0,
                psprites: [
                    Some(PSprite {
                        sprite: sprite_num("PISG"),
                        frame: 0,
                        sx: FRACUNIT,
                        sy: WEAPONTOP,
                    }),
                    None,
                ],
            },
        }
    }

    /// Renders one frame over a screen prefilled with `fill`. `things`
    /// stands in for walking every sector's `thinglist` (see
    /// `r_main`/`r_things`' module docs) — a snapshot of the arena's
    /// live mobjs, in spawn order, same as the original's `thinglist`
    /// traversal would see since nothing moves mobjs mid-frame here.
    fn render(&mut self, angle: Angle, fill: u8) -> Vec<u8> {
        let player = self.view_player(angle);
        let things: Vec<Mobj> = self.thinkers.iter_mobjs().map(|(_, m)| *m).collect();
        let mut screen = vec![fill; (320 * 200) as usize];
        self.renderer.r_render_player_view(
            &player,
            &mut self.level,
            &mut self.wad,
            &things,
            &mut screen,
        );
        screen
    }
}

#[test]
fn renders_complete_e1m1_frames_from_the_player_start() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let mut world = load_world(&path);
    let start_angle = world.player().angle;
    assert_eq!(start_angle, ANG90, "E1M1's player 1 start faces north");

    for turn in 0..4u32 {
        let angle = start_angle.wrapping_add(turn.wrapping_mul(ANG90));

        // Every pixel of the view must be drawn by something: render
        // the same view over two different prefills; any pixel that
        // still differs was never written (a "hall of mirrors" hole).
        let a = world.render(angle, 0);
        let b = world.render(angle, 0xff);
        let holes = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert_eq!(holes, 0, "turn {turn}: {holes} pixels were never drawn");

        let r = &world.renderer;
        assert!(r.rbsp.sscount > 0, "turn {turn}: no subsectors visited");
        assert!(!r.rbsp.drawsegs.is_empty(), "turn {turn}: no walls drawn");
        assert!(!r.rplane.visplanes.is_empty(), "turn {turn}: no planes");

        let colors: std::collections::HashSet<u8> = a.iter().copied().collect();
        assert!(
            colors.len() > 40,
            "turn {turn}: only {} distinct colors, expected a textured scene",
            colors.len()
        );

        let playpal = world
            .wad
            .cache_lump_name("PLAYPAL", PurgeTag::Cache)
            .to_vec();
        let out_path = scratch_dir().join(format!("doommetalrust_e1m1_turn{turn}.png"));
        save_palettized_png(&out_path, 320, 200, &a, &playpal);
        eprintln!(
            "turn {turn}: {} subsectors, {} drawsegs, {} visplanes, {} vissprites -> {}",
            r.rbsp.sscount,
            r.rbsp.drawsegs.len(),
            r.rplane.visplanes.len(),
            r.rthings.vissprites.len(),
            out_path.display()
        );
    }
}

#[test]
fn rendering_the_same_view_twice_is_deterministic() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let mut world = load_world(&path);
    let angle = world.player().angle;
    assert_eq!(world.render(angle, 0), world.render(angle, 0));
}

#[test]
fn outdoor_view_draws_the_sky() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let mut world = load_world(&path);
    let skyflatnum = world.renderer.sky.skyflatnum;

    // Stand at the centroid of the largest subsector (by seg count)
    // under an open sky — subsectors are convex, so the centroid of
    // their vertices is inside.
    let level = &world.level;
    let ss = (0..level.subsectors.len())
        .filter(|&i| level.sectors[level.subsectors[i].sector].ceilingpic as i32 == skyflatnum)
        .max_by_key(|&i| level.subsectors[i].numlines)
        .expect("E1M1 has outdoor areas");
    let sub = level.subsectors[ss];
    let segs = &level.segs[sub.firstline as usize..(sub.firstline + sub.numlines) as usize];
    let n = segs.len() as i64;
    let cx = (segs
        .iter()
        .map(|s| level.vertexes[s.v1].x as i64)
        .sum::<i64>()
        / n) as Fixed;
    let cy = (segs
        .iter()
        .map(|s| level.vertexes[s.v1].y as i64)
        .sum::<i64>()
        / n) as Fixed;
    let floorheight = level.sectors[sub.sector].floorheight;
    let player_mo = world.player_mo;
    let player = world.thinkers.mobj_mut(player_mo).unwrap();
    player.x = cx;
    player.y = cy;
    player.subsector = Some(ss);
    player.z = floorheight;

    let mut best: Option<(u32, usize, Vec<u8>)> = None;
    for turn in 0..4u32 {
        let angle = turn.wrapping_mul(ANG90);
        let a = world.render(angle, 0);
        let b = world.render(angle, 0xff);
        let holes = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert_eq!(holes, 0, "turn {turn}: {holes} pixels were never drawn");

        // Sky visplanes drawn this frame, by pixel count.
        let sky_pixels: usize = world
            .renderer
            .rplane
            .visplanes
            .iter()
            .filter(|pl| pl.picnum == skyflatnum && pl.minx <= pl.maxx)
            .map(|pl| {
                (pl.minx..=pl.maxx)
                    .map(|x| (pl.bottom_at(x) as i32 - pl.top_at(x) as i32 + 1).max(0) as usize)
                    .sum::<usize>()
            })
            .sum();
        if best.as_ref().is_none_or(|(_, px, _)| sky_pixels > *px) {
            best = Some((turn, sky_pixels, a));
        }
    }

    let (turn, sky_pixels, frame) = best.unwrap();
    assert!(
        sky_pixels > 1000,
        "expected a real patch of sky from an outdoor subsector, got {sky_pixels} pixels"
    );

    let playpal = world
        .wad
        .cache_lump_name("PLAYPAL", PurgeTag::Cache)
        .to_vec();
    let out_path = scratch_dir().join("doommetalrust_e1m1_sky.png");
    save_palettized_png(&out_path, 320, 200, &frame, &playpal);
    eprintln!(
        "sky: subsector {ss}, turn {turn}, {sky_pixels} sky pixels -> {}",
        out_path.display()
    );
}

#[test]
fn every_view_size_draws_every_pixel_of_its_window() {
    let Some(path) = find_test_wad() else {
        eprintln!("skipping: no doom.wad found");
        return;
    };
    let mut world = load_world(&path);
    let angle = world.player().angle;

    for blocks in 3..=11 {
        world.renderer.r_execute_set_view_size(blocks, 0);
        let (vx, vy, vw, vh) = {
            let r = &world.renderer.rdraw;
            (
                r.viewwindowx as usize,
                r.viewwindowy as usize,
                r.scaledviewwidth as usize,
                r.viewheight as usize,
            )
        };
        let a = world.render(angle, 0);
        let b = world.render(angle, 0xff);
        let mut holes = 0;
        for y in vy..vy + vh {
            for x in vx..vx + vw {
                if a[y * 320 + x] != b[y * 320 + x] {
                    holes += 1;
                }
            }
        }
        // ...and nothing outside the window was touched.
        let outside = a
            .iter()
            .zip(&b)
            .enumerate()
            .filter(|(i, (x, y))| {
                let (px, py) = (i % 320, i / 320);
                let inside = px >= vx && px < vx + vw && py >= vy && py < vy + vh;
                !inside && (**x != 0 || **y != 0xff)
            })
            .count();
        eprintln!(
            "setblocks {blocks}: window {vw}x{vh} at ({vx},{vy}): {holes} holes, {outside} stray"
        );
        assert_eq!(
            holes, 0,
            "setblocks {blocks}: undrawn pixels in the view window"
        );
        assert_eq!(
            outside, 0,
            "setblocks {blocks}: drew outside the view window"
        );
    }
}
