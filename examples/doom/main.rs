/*

Doom-style sector renderer — slice 1.

# Inputs

- Left / Right: turn.
- Up / Down: move forward / back.

# Maximize

Smallest viable renderer where walls aren't axis-aligned. One convex
hexagonal sector, six angled walls, projected through a view-space
transform and rasterised per column. No portals, no BSP traversal yet
(the world is one sub-sector). The data model — vertices, linedefs,
sectors — is the one we'll grow; slice 2 adds a second sector and a
portal so the BSP starts doing real work.

# Encoding

Same 32-bit player record as the wolfenstein example:

| Start | Length | Description                                          |
|-------|--------|------------------------------------------------------|
|     0 |     12 | player x, fp 256 (0..4095 = 0..16 world units)       |
|    12 |     12 | player y, fp 256                                     |
|    24 |      8 | heading (0..255 = 0..2π, CW from +x in screen space) |
|    32 |     32 | unused                                               |

# Notes

**Coordinates.** World units mirror the wolfenstein scale so 256 fp
units = 1 "tile" of feel. The hexagon is centred at (2048, 2048) with
radius 512 fp; spawn sits west of centre facing the east corner so the
opening frame shows two angled walls meeting head-on.

**Collision.** Point-in-convex-polygon: each linedef's cross product
against `(player − v0)` must keep the player on the interior side.
The check runs per axis after the proposed move, so you slide along
walls instead of getting stuck.

*/

mod level;
mod render;
mod trig;

use bitwise_games::bits::{get_bits, set_bits};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::{Game, Key};
use trig::{cos, sin};

const X_START: u8 = 0;
const X_BITS: u8 = 12;
const Y_START: u8 = X_START + X_BITS;
const Y_BITS: u8 = 12;
const ANGLE_START: u8 = Y_START + Y_BITS;
const ANGLE_BITS: u8 = 8;

const MOVE_SPEED: i32 = 32; // fp 256 units per frame
const TURN_SPEED: u8 = 3;

pub struct State {
    pub x: i32,
    pub y: i32,
    pub angle: u8,
}

fn decode(state: u64) -> State {
    State {
        x: get_bits::<u32>(state, X_START, X_BITS) as i32,
        y: get_bits::<u32>(state, Y_START, Y_BITS) as i32,
        angle: get_bits::<u8>(state, ANGLE_START, ANGLE_BITS),
    }
}

fn encode(state: &State) -> u64 {
    let mut s = 0u64;
    s = set_bits(s, state.x as u32, X_START, X_BITS);
    s = set_bits(s, state.y as u32, Y_START, Y_BITS);
    s = set_bits(s, state.angle, ANGLE_START, ANGLE_BITS);
    s
}

fn fresh() -> State {
    // West of the hexagon centre, facing east toward V0. The opening
    // view shows two angled walls (NE and SE sides) meeting at the
    // east corner straight ahead.
    State {
        x: 1700,
        y: 2048,
        angle: 0,
    }
}

fn try_move(state: &mut State, dx: i32, dy: i32) {
    let nx = state.x + dx;
    if level::point_inside((nx, state.y)) {
        state.x = nx;
    }
    let ny = state.y + dy;
    if level::point_inside((state.x, ny)) {
        state.y = ny;
    }
}

struct DoomGame;

impl Game for DoomGame {
    const NAME: &'static str = "Doom";
    const FPS: usize = 30;

    fn init(_args: Vec<String>) -> (u64, FrameBuffer) {
        let s = fresh();
        (encode(&s), render::render(&s))
    }

    fn update(
        state: u64,
        held: &[Key],
        _buffered: Option<Key>,
        _mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let mut s = decode(state);

        if held.contains(&Key::Left) {
            s.angle = s.angle.wrapping_sub(TURN_SPEED);
        }
        if held.contains(&Key::Right) {
            s.angle = s.angle.wrapping_add(TURN_SPEED);
        }

        let mut dx = 0;
        let mut dy = 0;
        if held.contains(&Key::Up) {
            dx += cos(s.angle) * MOVE_SPEED / 256;
            dy += sin(s.angle) * MOVE_SPEED / 256;
        }
        if held.contains(&Key::Down) {
            dx -= cos(s.angle) * MOVE_SPEED / 256;
            dy -= sin(s.angle) * MOVE_SPEED / 256;
        }
        if dx != 0 || dy != 0 {
            try_move(&mut s, dx, dy);
        }

        (encode(&s), render::render(&s))
    }
}

fn main() {
    bitwise_games::run_game::<DoomGame>();
}
