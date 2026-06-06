//! View-space wall projection.
//!
//! For each linedef we transform the two endpoints into view space
//! (depth forward, lateral right), near-clip against the camera, then
//! project to screen columns and top/bot rows. Linear interpolation
//! across screen x of `top` and `bot` is perspective-correct because
//! 1/depth varies linearly in screen x under perspective projection,
//! and `top`, `bot` are themselves linear in 1/depth.

use crate::State;
use crate::level::{EYE_HEIGHT, LINEDEFS, NEAR_DEPTH, VERTICES, WALL_BOT, WALL_TOP};
use crate::trig::{cos, sin};
use bitwise_games::draw_command::{BROWN, Color, DARK_BLUE, DrawCommand};
use bitwise_games::frame_buffer::{FrameBuffer, HEIGHT, WIDTH};

const HORIZON: i32 = (HEIGHT / 2) as i32;

// Effective focal length in pixels. Vertical wall height at depth d
// (fp 256) is wall_height_fp * FOCAL / d. With FOCAL = 96 the
// horizontal FOV is ≈ 67° — a touch wider than the wolfenstein
// raycaster, which makes the angled walls feel more spacious.
const FOCAL: i32 = 96;

pub fn render(state: &State) -> FrameBuffer {
    let mut fb = FrameBuffer::new();

    // Flat sky / floor. Floor and ceiling visplanes come later.
    fb.draw(&DrawCommand::rect(0, 0, WIDTH, HEIGHT / 2, DARK_BLUE));
    fb.draw(&DrawCommand::rect(0, HEIGHT / 2, WIDTH, HEIGHT / 2, BROWN));

    let cos_a = cos(state.angle);
    let sin_a = sin(state.angle);

    for ld in LINEDEFS {
        let (v0x, v0y) = VERTICES[ld.v0 as usize];
        let (v1x, v1y) = VERTICES[ld.v1 as usize];
        draw_wall(&mut fb, state, cos_a, sin_a, v0x, v0y, v1x, v1y, ld.color);
    }

    fb
}

#[allow(clippy::too_many_arguments)]
fn draw_wall(
    fb: &mut FrameBuffer,
    state: &State,
    cos_a: i32,
    sin_a: i32,
    v0x: i32,
    v0y: i32,
    v1x: i32,
    v1y: i32,
    color: Color,
) {
    // World → view space. Forward axis is (cos a, sin a); the right
    // axis (matching screen-x increasing) is (-sin a, cos a) — same
    // convention the wolfenstein camera plane uses.
    let dx0 = v0x - state.x;
    let dy0 = v0y - state.y;
    let dx1 = v1x - state.x;
    let dy1 = v1y - state.y;

    let mut d0 = (dx0 * cos_a + dy0 * sin_a) / 256;
    let mut r0 = (-dx0 * sin_a + dy0 * cos_a) / 256;
    let mut d1 = (dx1 * cos_a + dy1 * sin_a) / 256;
    let mut r1 = (-dx1 * sin_a + dy1 * cos_a) / 256;

    // Both endpoints behind the near plane: the whole wall is behind
    // us, drop it.
    if d0 < NEAR_DEPTH && d1 < NEAR_DEPTH {
        return;
    }
    // One endpoint behind: clip it to the near plane along the wall
    // line. Linear in world space because we're solving for the
    // intersection of two straight lines.
    if d0 < NEAR_DEPTH {
        let t = ((NEAR_DEPTH - d0) * 256) / (d1 - d0);
        r0 += ((r1 - r0) * t) / 256;
        d0 = NEAR_DEPTH;
    } else if d1 < NEAR_DEPTH {
        let t = ((NEAR_DEPTH - d1) * 256) / (d0 - d1);
        r1 += ((r0 - r1) * t) / 256;
        d1 = NEAR_DEPTH;
    }

    let col0 = WIDTH as i32 / 2 + (r0 * FOCAL) / d0;
    let col1 = WIDTH as i32 / 2 + (r1 * FOCAL) / d1;

    let col_lo = col0.min(col1);
    let col_hi = col0.max(col1);
    if col_lo == col_hi {
        return; // zero-width wall edge-on
    }
    let col_start = col_lo.max(0);
    let col_end = col_hi.min(WIDTH as i32 - 1);
    if col_start > col_end {
        return; // fully off-screen
    }

    let top_h = WALL_TOP - EYE_HEIGHT;
    let bot_h = EYE_HEIGHT - WALL_BOT;
    let top0 = HORIZON - (top_h * FOCAL) / d0;
    let bot0 = HORIZON + (bot_h * FOCAL) / d0;
    let top1 = HORIZON - (top_h * FOCAL) / d1;
    let bot1 = HORIZON + (bot_h * FOCAL) / d1;

    let span = col1 - col0; // signed, may be negative if wall flipped
    for col in col_start..=col_end {
        // t in fp 256. 0 at col0, 256 at col1, signed so it works both
        // ways. Linear interpolation of top/bot is perspective-correct.
        let t = ((col - col0) * 256) / span;
        let top = (top0 + ((top1 - top0) * t) / 256).clamp(0, HEIGHT as i32 - 1);
        let bot = (bot0 + ((bot1 - bot0) * t) / 256).clamp(0, HEIGHT as i32 - 1);
        if top > bot {
            continue;
        }
        for row in top..=bot {
            fb.pixels[(row as u32 * WIDTH + col as u32) as usize] = color;
        }
    }
}
