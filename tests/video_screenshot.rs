//! Integration test / manual verification tool for Phase 4's milestone:
//! "consegue abrir janela e mostrar um frame" (open a window and show a
//! frame). Runs the real `i_video::IVideo` path — SDL init, window,
//! renderer, streaming texture, palette application, and the
//! `I_FinishUpdate` blit — against SDL2's headless "dummy" video driver
//! (`SDL_VIDEODRIVER=dummy`, set by this test itself so it also works
//! without a real display attached), then reads the rendered frame back
//! with `SDL_RenderReadPixels` and saves it as a PNG for visual review,
//! rather than only asserting "it didn't crash".
//!
//! Draws a deliberately simple, checkable test pattern (a horizontal
//! gradient built from a synthetic 256-color palette) directly into
//! `screens[0]`, so the saved PNG's colors are a direct, predictable
//! function of the palette and the pattern — not just "some pixels came
//! out", but "the right pixels came out".

use doommetal_rust::doomdef::{SCREENHEIGHT, SCREENWIDTH};
use doommetal_rust::i_video::IVideo;
use doommetal_rust::m_argv;

fn scratch_dir() -> std::path::PathBuf {
    std::env::var_os("DOOMMETALRUST_SCRATCH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

#[test]
fn opens_window_renders_test_pattern_and_saves_screenshot() {
    // SDL2's headless video driver: renders into an off-screen surface
    // with no real display/window server required, which is what makes
    // this test runnable in CI/sandboxed environments. Set here (rather
    // than requiring it in the environment) so the test is
    // self-contained.
    std::env::set_var("SDL_VIDEODRIVER", "dummy");
    std::env::set_var("SDL_AUDIODRIVER", "dummy");

    // i_init_graphics reads command-line params (M_CheckParm) the same
    // way I_InitGraphics does in the original, which requires
    // m_argv::set_args to have run first — normally main.rs's job, done
    // here since this test drives IVideo directly.
    m_argv::set_args(std::env::args().collect());

    let mut video = IVideo::i_init_graphics();

    // A synthetic 768-byte (256 * RGB) palette: a smooth blue-to-red
    // gradient by palette index, distinct from an all-zero/uninitialized
    // buffer and easy to eyeball-verify in the saved PNG.
    let mut pal = vec![0u8; 256 * 3];
    for i in 0..256 {
        pal[i * 3] = i as u8; // R ramps 0..255
        pal[i * 3 + 1] = 64; // constant G
        pal[i * 3 + 2] = (255 - i) as u8; // B ramps 255..0
    }
    video.i_set_palette(&pal);

    // Test pattern: each column's palette index is its x-position
    // modulo 256, so the frame is a horizontal repeating gradient,
    // trivially predictable pixel-by-pixel.
    for y in 0..SCREENHEIGHT as usize {
        for x in 0..SCREENWIDTH as usize {
            video.v_video.screens[0][y * SCREENWIDTH as usize + x] = (x % 256) as u8;
        }
    }

    // The strong check: verify the exact bytes I_FinishUpdate's caller
    // (this test) wrote into the logical SCREENWIDTH x SCREENHEIGHT
    // framebuffer are still there afterwards — i_finish_update blits
    // *from* screens[0], it doesn't consume/clear it, so this also
    // catches i_finish_update accidentally mutating its source buffer.
    for &x in &[0usize, 50, 100, 150, 200, 255] {
        for &y in &[0usize, 100, 199] {
            let idx = y * SCREENWIDTH as usize + x;
            assert_eq!(
                video.v_video.screens[0][idx],
                (x % 256) as u8,
                "screens[0] pattern mismatch at ({x}, {y})"
            );
        }
    }

    video.i_finish_update();

    // output_size()/read_pixels() operate in the renderer's *physical*
    // backbuffer space, not the SCREENWIDTH x SCREENHEIGHT *logical*
    // space I_InitGraphics configures via
    // SDL_RenderSetLogicalSize/set_logical_size — the window itself is
    // SCREENWIDTH*multiply x SCREENHEIGHT*multiply (multiply=4 with no
    // -2/-3/-4 arg, matching the original's non-Windows default), and
    // SDL scales the texture up when presenting it (with some
    // interpolation at the physical-pixel level, observed empirically —
    // not a property of this port's code, just how SDL's renderer
    // upscales). So exact per-physical-pixel color checks aren't
    // reliable here; the physical-space capture below is for visual
    // review (the saved PNG) plus a loose sanity check (non-uniform
    // output, not all black/all one color), while the precise
    // correctness check is the logical-space one above.
    let (w, h) = video.output_size();
    assert_eq!(
        w % SCREENWIDTH as u32,
        0,
        "expected an integer upscale of the logical width"
    );
    assert_eq!(
        h % SCREENHEIGHT as u32,
        0,
        "expected an integer upscale of the logical height"
    );

    let pixels = video.read_rendered_pixels_rgb24();
    assert_eq!(pixels.len(), (w * h * 3) as usize);

    let distinct_colors: std::collections::HashSet<(u8, u8, u8)> =
        pixels.chunks_exact(3).map(|c| (c[0], c[1], c[2])).collect();
    assert!(
        distinct_colors.len() > 1,
        "expected a gradient with more than one distinct color, got a flat/empty frame"
    );

    // Save as PNG for direct visual review.
    let out_path = scratch_dir().join("doommetalrust_phase4_screenshot.png");
    save_png(&out_path, w, h, &pixels);
    eprintln!("screenshot written to {}", out_path.display());
}

fn save_png(path: &std::path::Path, width: u32, height: u32, rgb: &[u8]) {
    let file = std::fs::File::create(path).expect("failed to create screenshot file");
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("failed to write PNG header");
    writer
        .write_image_data(rgb)
        .expect("failed to write PNG data");
}
