/*

Jetpack Joyride — side-scrolling endless flyer.

# Inputs

- Z held: fire jetpack (rise). Released: gravity falls.
- Z or X after death: restart.

# Maximize

Endless side-scroller where world content (hazards, missile warnings,
background tiles, parallax) is a pure function of `(seed, camera_x)`,
so per-hazard storage is zero. Barry's pose derives from `vy`'s sign;
flame/gun animation phase derives from `camera_x`. The one carve-out
is post-death animation: camera_x freezes on death, so a separate
4-bit counter ticks the tumble frames.

# Encoding

| Start | Length | Description                                          |
|-------|--------|------------------------------------------------------|
|     0 |      8 | world seed                                           |
|     8 |     18 | camera_x (px scrolled; ~73 min @ 30 FPS, 2 px/frame) |
|    26 |      7 | barry y (top of sprite; 0..127)                      |
|    33 |      6 | barry vy (i6 two's complement; -32..31)              |
|    39 |      1 | dead flag                                            |
|    40 |      4 | death-anim counter (0..15)                           |
|    44 |     20 | unused                                               |

# Notes

This is slice 1: physics + jetpack bullets + scrolling ground/ceiling
+ death/restart. Hazards (zappers, lasers, missiles), parallax, and
real sprite art come in later slices.

*/
use bitwise_games::bits::{get_bits, set_bits};
use bitwise_games::draw_command::{
    BLACK, Color, DARK_GREY, DrawCommand, LIGHT_GREY, ORANGE, WHITE, YELLOW,
};
use bitwise_games::font::{digits_of, draw_text, text_width};
use bitwise_games::frame_buffer::{FrameBuffer, HEIGHT, WIDTH};
use bitwise_games::rng;
use bitwise_games::sprite::{Sprite, TRANSPARENT};
use bitwise_games::{Game, Key};

// --- World layout ---

const BARRY_X: u32 = 20;
const BARRY_W: u32 = 6;
const BARRY_H: u32 = 10;

const BARRY_Y_MIN: u32 = 0;
const BARRY_Y_MAX: u32 = HEIGHT - BARRY_H;

const SCROLL_PX_PER_FRAME: u32 = 2;

// --- Physics ---

// Gravity applies every other frame (effective 0.5 px/frame²); thrust every
// frame when held. Net while held: -0.5 px/frame² avg. Net while falling:
// +0.5 px/frame² avg. Symmetric, arcadey arc.
const GRAVITY: i8 = 1;
const THRUST: i8 = -1;
const VY_CLAMP: i8 = 5;

// --- Death ---

const DEATH_ANIM_MAX: u8 = 15;

// --- Jetpack bullets ---

const BULLET_COUNT: u32 = 3;
const BULLET_STAGGER: u32 = 4;
const BULLET_FALL_RANGE: u32 = 10;
const JET_OFFSET_X: u32 = 2;
const JET_OFFSET_Y: u32 = BARRY_H;

// --- UI ---

const SCORE_SCALE: u32 = 1;
const BANNER_SCALE: u32 = 2;

// --- Bit layout ---

const SEED_START: u8 = 0;
const SEED_BITS: u8 = 8;
const CAMX_START: u8 = SEED_START + SEED_BITS;
const CAMX_BITS: u8 = 18;
const BARRY_Y_START: u8 = CAMX_START + CAMX_BITS;
const BARRY_Y_BITS: u8 = 7;
const VY_START: u8 = BARRY_Y_START + BARRY_Y_BITS;
const VY_BITS: u8 = 6;
const DEAD_START: u8 = VY_START + VY_BITS;
const DEAD_BITS: u8 = 1;
const DEATH_ANIM_START: u8 = DEAD_START + DEAD_BITS;
const DEATH_ANIM_BITS: u8 = 4;

const CAMX_MASK: u32 = (1u32 << CAMX_BITS) - 1;

// --- Placeholder Barry sprite, 6×10, 1 frame ---
// Real PNG-imported art slots in here later.

const T: u8 = TRANSPARENT;
const K: u8 = 0; // black
const R: u8 = 8; // red
const P: u8 = 15; // peach
const B: u8 = 12; // blue

#[rustfmt::skip]
const BARRY_PIXELS: &[u8] = &[
    T, T, P, P, T, T,
    T, P, P, P, P, T,
    T, P, K, P, K, P,
    T, P, P, P, P, T,
    T, R, R, R, R, T,
    R, R, R, R, R, R,
    R, R, R, R, R, R,
    T, B, B, B, B, T,
    T, B, T, T, B, T,
    T, B, T, T, B, T,
];

const BARRY: Sprite = Sprite {
    width: BARRY_W as u8,
    height: BARRY_H as u8,
    frames: 1,
    pixels: BARRY_PIXELS,
};

// --- Hazards: zappers ---
//
// World divided into 64-px chunks. Each chunk has one pattern, picked by
// `rng::next(seed << 32 ^ chunk_id)`. Chunks 0..2 are forced empty for a
// brief grace runway at the start of every run.

const CHUNK_W: u32 = 64;
const GRACE_CHUNKS: u32 = 2;

#[derive(Copy, Clone)]
enum Pattern {
    Empty,
    VerticalTop,    // beam from ceiling halfway down the playfield
    VerticalBottom, // beam from halfway down to floor
    HorizontalHigh, // beam across the upper third
    HorizontalLow,  // beam across the lower third
}

#[derive(Copy, Clone)]
struct Zapper {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

fn pattern_for_chunk(seed: u8, chunk_id: u32) -> Pattern {
    if chunk_id < GRACE_CHUNKS {
        return Pattern::Empty;
    }
    let mix = ((seed as u64) << 32) ^ (chunk_id as u64);
    match rng::next(mix) % 8 {
        0..=2 => Pattern::Empty,
        3 => Pattern::VerticalTop,
        4 => Pattern::VerticalBottom,
        5 => Pattern::HorizontalHigh,
        _ => Pattern::HorizontalLow,
    }
}

fn zapper_for_chunk(seed: u8, chunk_id: u32) -> Option<Zapper> {
    let cx = (chunk_id * CHUNK_W + CHUNK_W / 2) as i32;
    let lx = (chunk_id * CHUNK_W + 8) as i32;
    let rx = (chunk_id * CHUNK_W + CHUNK_W - 8) as i32;
    let mid_y = (HEIGHT / 2) as i32;
    match pattern_for_chunk(seed, chunk_id) {
        Pattern::Empty => None,
        Pattern::VerticalTop => Some(Zapper {
            x1: cx,
            y1: 0,
            x2: cx,
            y2: mid_y,
        }),
        Pattern::VerticalBottom => Some(Zapper {
            x1: cx,
            y1: mid_y,
            x2: cx,
            y2: HEIGHT as i32 - 1,
        }),
        Pattern::HorizontalHigh => Some(Zapper {
            x1: lx,
            y1: 28,
            x2: rx,
            y2: 28,
        }),
        Pattern::HorizontalLow => Some(Zapper {
            x1: lx,
            y1: 100,
            x2: rx,
            y2: 100,
        }),
    }
}

fn visible_chunk_range(camera_x: u32) -> (u32, u32) {
    let lo = camera_x / CHUNK_W;
    let hi = (camera_x + WIDTH) / CHUNK_W;
    (lo, hi)
}

fn zapper_hits_barry(z: &Zapper, barry_y: u8, camera_x: u32) -> bool {
    let bx_lo = camera_x as i32 + BARRY_X as i32;
    let bx_hi = bx_lo + BARRY_W as i32;
    let by_lo = barry_y as i32;
    let by_hi = by_lo + BARRY_H as i32;
    let zx_lo = z.x1.min(z.x2);
    let zx_hi = z.x1.max(z.x2) + 1;
    let zy_lo = z.y1.min(z.y2);
    let zy_hi = z.y1.max(z.y2) + 1;
    bx_lo < zx_hi && bx_hi > zx_lo && by_lo < zy_hi && by_hi > zy_lo
}

// --- State ---

struct State {
    seed: u8,
    camera_x: u32,
    barry_y: u8,
    vy: i8,
    dead: bool,
    death_anim: u8,
}

fn vy_to_bits(vy: i8) -> u8 {
    (vy as u8) & 0x3F
}

fn vy_from_bits(bits: u8) -> i8 {
    let bits = bits & 0x3F;
    if bits & 0x20 != 0 {
        (bits | 0xC0) as i8 // sign-extend from 6-bit
    } else {
        bits as i8
    }
}

fn decode(state: u64) -> State {
    State {
        seed: get_bits::<u8>(state, SEED_START, SEED_BITS),
        camera_x: get_bits::<u32>(state, CAMX_START, CAMX_BITS),
        barry_y: get_bits::<u8>(state, BARRY_Y_START, BARRY_Y_BITS),
        vy: vy_from_bits(get_bits::<u8>(state, VY_START, VY_BITS)),
        dead: get_bits::<u8>(state, DEAD_START, DEAD_BITS) == 1,
        death_anim: get_bits::<u8>(state, DEATH_ANIM_START, DEATH_ANIM_BITS),
    }
}

fn encode(state: &State) -> u64 {
    let mut s = 0u64;
    s = set_bits(s, state.seed, SEED_START, SEED_BITS);
    s = set_bits(s, state.camera_x, CAMX_START, CAMX_BITS);
    s = set_bits(s, state.barry_y, BARRY_Y_START, BARRY_Y_BITS);
    s = set_bits(s, vy_to_bits(state.vy), VY_START, VY_BITS);
    s = set_bits(s, state.dead as u8, DEAD_START, DEAD_BITS);
    s = set_bits(s, state.death_anim, DEATH_ANIM_START, DEATH_ANIM_BITS);
    s
}

// --- Render ---

fn draw_clipped_rect(fb: &mut FrameBuffer, x: i32, y: i32, w: u32, h: u32, color: Color) {
    let x_lo = x.max(0);
    let y_lo = y.max(0);
    let x_hi = (x + w as i32).min(WIDTH as i32);
    let y_hi = (y + h as i32).min(HEIGHT as i32);
    if x_lo >= x_hi || y_lo >= y_hi {
        return;
    }
    fb.draw(&DrawCommand::rect(
        x_lo as u32,
        y_lo as u32,
        (x_hi - x_lo) as u32,
        (y_hi - y_lo) as u32,
        color,
    ));
}

/// Metal mount-plate that anchors a beam endpoint. Centered on (sx, sy),
/// oriented perpendicular to the beam, with a dark slot along the beam axis
/// to suggest the emitter aperture.
fn draw_node_cap(fb: &mut FrameBuffer, sx: i32, sy: i32, beam_is_vertical: bool) {
    if beam_is_vertical {
        draw_clipped_rect(fb, sx - 2, sy - 1, 5, 3, LIGHT_GREY);
        draw_clipped_rect(fb, sx - 1, sy, 3, 1, DARK_GREY);
    } else {
        draw_clipped_rect(fb, sx - 1, sy - 2, 3, 5, LIGHT_GREY);
        draw_clipped_rect(fb, sx, sy - 1, 1, 3, DARK_GREY);
    }
}

/// Draw an axis-aligned 1-px stripe, clipping to screen. Negative or
/// out-of-range screen coords are handled cleanly.
fn draw_clipped_stripe(fb: &mut FrameBuffer, sx1: i32, sy1: i32, sx2: i32, sy2: i32, color: Color) {
    if sy1 == sy2 {
        if sy1 < 0 || sy1 >= HEIGHT as i32 {
            return;
        }
        let x_lo = sx1.min(sx2).max(0);
        let x_hi = sx1.max(sx2).min(WIDTH as i32 - 1);
        if x_lo > x_hi {
            return;
        }
        fb.draw(&DrawCommand::rect(
            x_lo as u32,
            sy1 as u32,
            (x_hi - x_lo + 1) as u32,
            1,
            color,
        ));
    } else if sx1 == sx2 {
        if sx1 < 0 || sx1 >= WIDTH as i32 {
            return;
        }
        let y_lo = sy1.min(sy2).max(0);
        let y_hi = sy1.max(sy2).min(HEIGHT as i32 - 1);
        if y_lo > y_hi {
            return;
        }
        fb.draw(&DrawCommand::rect(
            sx1 as u32,
            y_lo as u32,
            1,
            (y_hi - y_lo + 1) as u32,
            color,
        ));
    }
}

fn draw_zappers(fb: &mut FrameBuffer, seed: u8, camera_x: u32) {
    // 1-bit flicker on frame parity: glow alternates yellow/orange every
    // frame at 30 FPS → ~15 Hz, fast enough to read as a continuous arc.
    let flicker = (camera_x >> 1) & 1;
    let glow = if flicker == 0 { YELLOW } else { ORANGE };
    let core = WHITE;

    let cam = camera_x as i32;
    let (lo, hi) = visible_chunk_range(camera_x);
    for chunk_id in lo..=hi {
        let Some(z) = zapper_for_chunk(seed, chunk_id) else {
            continue;
        };
        let sx1 = z.x1 - cam;
        let sy1 = z.y1;
        let sx2 = z.x2 - cam;
        let sy2 = z.y2;

        let vertical = z.x1 == z.x2;
        if vertical {
            // Vertical beam: glow flanks at x±1, core at x.
            draw_clipped_stripe(fb, sx1 - 1, sy1, sx2 - 1, sy2, glow);
            draw_clipped_stripe(fb, sx1 + 1, sy1, sx2 + 1, sy2, glow);
            draw_clipped_stripe(fb, sx1, sy1, sx2, sy2, core);
        } else {
            // Horizontal beam: glow flanks at y±1, core at y.
            draw_clipped_stripe(fb, sx1, sy1 - 1, sx2, sy2 - 1, glow);
            draw_clipped_stripe(fb, sx1, sy1 + 1, sx2, sy2 + 1, glow);
            draw_clipped_stripe(fb, sx1, sy1, sx2, sy2, core);
        }

        draw_node_cap(fb, sx1, sy1, vertical);
        draw_node_cap(fb, sx2, sy2, vertical);
    }
}

fn draw_background(fb: &mut FrameBuffer) {
    fb.draw(&DrawCommand::rect(0, 0, WIDTH, HEIGHT, BLACK));
}

fn draw_banner(fb: &mut FrameBuffer, line1: &[u8], line2: &[u8]) {
    let mut cmds = Vec::new();
    let l1w = text_width(line1.len(), BANNER_SCALE);
    let l2w = text_width(line2.len(), BANNER_SCALE);
    let bw = l1w.max(l2w) + 8;
    let bh = 36;
    let bx = (WIDTH - bw) / 2;
    let by = (HEIGHT - bh) / 2;
    cmds.push(DrawCommand::rect(bx, by, bw, bh, BLACK));
    cmds.push(DrawCommand::rect(bx, by, bw, 1, ORANGE));
    cmds.push(DrawCommand::rect(bx, by + bh - 1, bw, 1, ORANGE));
    cmds.push(DrawCommand::rect(bx, by, 1, bh, ORANGE));
    cmds.push(DrawCommand::rect(bx + bw - 1, by, 1, bh, ORANGE));
    draw_text(
        &mut cmds,
        line1,
        bx + (bw - l1w) / 2,
        by + 6,
        BANNER_SCALE,
        ORANGE,
    );
    draw_text(
        &mut cmds,
        line2,
        bx + (bw - l2w) / 2,
        by + 22,
        BANNER_SCALE,
        WHITE,
    );
    fb.draw_list(&cmds);
}

fn render(state: &State, z_held: bool) -> FrameBuffer {
    let mut fb = FrameBuffer::new();

    draw_background(&mut fb);
    draw_zappers(&mut fb, state.seed, state.camera_x);

    // Jetpack bullets — only when Z held and alive.
    if !state.dead && z_held {
        for i in 0..BULLET_COUNT {
            let phase = (state.camera_x + i * BULLET_STAGGER) % BULLET_FALL_RANGE;
            let bx = BARRY_X + JET_OFFSET_X;
            let by = state.barry_y as u32 + JET_OFFSET_Y + phase;
            if by < HEIGHT {
                fb.draw(&DrawCommand::rect(bx, by, 1, 2, YELLOW));
            }
        }
    }

    fb.blit(&BARRY, 0, BARRY_X as i32, state.barry_y as i32);

    // Score (camera_x in px) above the ceiling, white-on-dark.
    let mut score_cmds = Vec::new();
    let digits = digits_of(state.camera_x);
    draw_text(&mut score_cmds, &digits, 2, 2, SCORE_SCALE, WHITE);
    fb.draw_list(&score_cmds);

    if state.dead {
        draw_banner(&mut fb, b"GAME OVER", b"PRESS Z");
    }

    fb
}

// --- Game impl ---

fn fresh(seed: u8) -> State {
    State {
        seed,
        camera_x: 0,
        barry_y: 60,
        vy: 0,
        dead: false,
        death_anim: 0,
    }
}

struct JetpackGame;

impl Game for JetpackGame {
    const NAME: &'static str = "Jetpack";
    const FPS: usize = 30;

    fn new(args: Vec<String>) -> (u64, FrameBuffer) {
        let seed = args.get(1).and_then(|s| s.parse::<u8>().ok()).unwrap_or(0);
        let state = fresh(seed);
        (encode(&state), render(&state, false))
    }

    fn update(
        state: u64,
        held: &[Key],
        buffered: Option<Key>,
        _mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let mut state = decode(state);
        let z_held = held.contains(&Key::Z);

        if state.dead {
            // Restart on Z/X; otherwise tick the death-anim counter and freeze.
            if matches!(buffered, Some(Key::Z) | Some(Key::X)) {
                let new_seed = rng::next(encode(&state)) as u8;
                let s = fresh(new_seed);
                return (encode(&s), render(&s, false));
            }
            if state.death_anim < DEATH_ANIM_MAX {
                state.death_anim += 1;
            }
            return (encode(&state), render(&state, false));
        }

        // Physics. Gravity applies every other frame for a lighter feel
        // (effective 0.5 px/frame² without needing fractional vy).
        let frame_idx = state.camera_x / SCROLL_PX_PER_FRAME;
        let mut vy = state.vy;
        if frame_idx % 2 == 0 {
            vy += GRAVITY;
        }
        if z_held {
            vy += THRUST;
        }
        vy = vy.clamp(-VY_CLAMP, VY_CLAMP);

        // Floor and ceiling are walls, not hazards — clamp and zero vy.
        let new_y = state.barry_y as i32 + vy as i32;
        if new_y >= BARRY_Y_MAX as i32 {
            state.barry_y = BARRY_Y_MAX as u8;
            state.vy = 0;
        } else if new_y <= BARRY_Y_MIN as i32 {
            state.barry_y = BARRY_Y_MIN as u8;
            state.vy = 0;
        } else {
            state.barry_y = new_y as u8;
            state.vy = vy;
        }

        // Hazard collision after physics. If hit, freeze position; the dead
        // path takes over next tick.
        let (lo, hi) = visible_chunk_range(state.camera_x);
        for chunk_id in lo..=hi {
            if let Some(z) = zapper_for_chunk(state.seed, chunk_id) {
                if zapper_hits_barry(&z, state.barry_y, state.camera_x) {
                    state.dead = true;
                    state.death_anim = 0;
                    return (encode(&state), render(&state, z_held));
                }
            }
        }

        state.camera_x = (state.camera_x + SCROLL_PX_PER_FRAME) & CAMX_MASK;

        (encode(&state), render(&state, z_held))
    }
}

fn main() {
    bitwise_games::run_game::<JetpackGame>();
}
