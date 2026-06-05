/*

Wolfenstein-style raycaster on a hardcoded 16×16 map.

# Inputs

- Left / Right: turn.
- Up / Down: move forward / back.

# Maximize

Smallest viable raycaster. The map is a code constant (one bit per cell,
16 rows × 16 cols = 256 bits, none on state). Player state is just
position + heading: 12 bits x, 12 bits y, 8 bits angle. 32 bits used,
32 free for later (health, doors, ammo, enemies).

# Encoding

| Start | Length | Description                                          |
|-------|--------|------------------------------------------------------|
|     0 |     12 | player x, fp 256 (0..4095 = 0..16 tiles)             |
|    12 |     12 | player y, fp 256                                     |
|    24 |      8 | heading (0..255 = 0..2π, CCW from +x)                |
|    32 |     32 | unused                                               |

# Notes

**Fixed point.** Positions are 12-bit values where one tile is 256
units, so the lower 8 bits are sub-tile and the upper 4 bits are the
map cell index. Ray directions and trig outputs use the same scale
(256 = 1.0). Distances along the ray are in the same fp 256 units
where 256 = "traveled one tile worth of distance along the ray".

**DDA.** One ray per screen column (128 columns total). Each ray steps
through map cells along the standard side-distance recurrence; on the
first wall hit we record perpendicular distance and which axis was
crossed. Camera-plane formulation (dir + plane * camera_x) gives
perp distance directly as the side_dist value at hit, no extra trig.

**Sin table.** 256 entries of i16 in fp 256 (range -256..256). Cos is
sin shifted by 64 (a quarter turn).

*/
use bitwise_games::bits::{get_bits, set_bits};
use bitwise_games::draw_command::{
    BLACK, BROWN, Color, DARK_BLUE, DARK_GREY, DrawCommand, LIGHT_GREY,
};
use bitwise_games::frame_buffer::{FrameBuffer, HEIGHT, WIDTH};
use bitwise_games::{Game, Key};

// --- Bit layout ---

const X_START: u8 = 0;
const X_BITS: u8 = 12;
const Y_START: u8 = X_START + X_BITS;
const Y_BITS: u8 = 12;
const ANGLE_START: u8 = Y_START + Y_BITS;
const ANGLE_BITS: u8 = 8;

// --- Map ---

const MAP_TILES: i32 = 16;
const TILE_SHIFT: u32 = 8;
const TILE_SIZE: i32 = 1 << TILE_SHIFT; // 256 fp units per tile
const WORLD_MAX: i32 = MAP_TILES * TILE_SIZE; // 4096

// 1 = wall, 0 = empty. One u16 per row; in the binary literal, the
// LEFTMOST digit is column 0 — so each row reads like a top-down map.
// Outer border is solid; interior carves a couple of pillars and a stub
// wall to give the player something to bump into and look at.
#[rustfmt::skip]
const MAP: [u16; 16] = [
    0b1111111111111111,
    0b1000000000000001,
    0b1000000000000001,
    0b1000000000000001,
    0b1000111000000001,
    0b1000100000000001,
    0b1000100000000001,
    0b1000100001111001,
    0b1000000000000001,
    0b1000000010000001,
    0b1000000010000001,
    0b1000010010000001,
    0b1000010000000001,
    0b1000000000000001,
    0b1000000000000001,
    0b1111111111111111,
];

fn is_wall(tx: i32, ty: i32) -> bool {
    if !(0..MAP_TILES).contains(&tx) || !(0..MAP_TILES).contains(&ty) {
        return true;
    }
    // Bit 15 of the literal = column 0 (so the literal reads left-to-right).
    (MAP[ty as usize] >> (15 - tx)) & 1 != 0
}

// --- Trig ---

// sin(i * 2π / 256) * 256, rounded. Cos is SIN[(a + 64) & 255].
#[rustfmt::skip]
const SIN: [i16; 256] = [
       0,    6,   13,   19,   25,   31,   38,   44,   50,   56,   62,   68,   74,   80,   86,   92,
      98,  104,  109,  115,  121,  126,  132,  137,  142,  147,  152,  157,  162,  167,  172,  177,
     181,  185,  190,  194,  198,  202,  206,  209,  213,  216,  220,  223,  226,  229,  231,  234,
     237,  239,  241,  243,  245,  247,  248,  250,  251,  252,  253,  254,  255,  255,  256,  256,
     256,  256,  256,  255,  255,  254,  253,  252,  251,  250,  248,  247,  245,  243,  241,  239,
     237,  234,  231,  229,  226,  223,  220,  216,  213,  209,  206,  202,  198,  194,  190,  185,
     181,  177,  172,  167,  162,  157,  152,  147,  142,  137,  132,  126,  121,  115,  109,  104,
      98,   92,   86,   80,   74,   68,   62,   56,   50,   44,   38,   31,   25,   19,   13,    6,
       0,   -6,  -13,  -19,  -25,  -31,  -38,  -44,  -50,  -56,  -62,  -68,  -74,  -80,  -86,  -92,
     -98, -104, -109, -115, -121, -126, -132, -137, -142, -147, -152, -157, -162, -167, -172, -177,
    -181, -185, -190, -194, -198, -202, -206, -209, -213, -216, -220, -223, -226, -229, -231, -234,
    -237, -239, -241, -243, -245, -247, -248, -250, -251, -252, -253, -254, -255, -255, -256, -256,
    -256, -256, -256, -255, -255, -254, -253, -252, -251, -250, -248, -247, -245, -243, -241, -239,
    -237, -234, -231, -229, -226, -223, -220, -216, -213, -209, -206, -202, -198, -194, -190, -185,
    -181, -177, -172, -167, -162, -157, -152, -147, -142, -137, -132, -126, -121, -115, -109, -104,
     -98,  -92,  -86,  -80,  -74,  -68,  -62,  -56,  -50,  -44,  -38,  -31,  -25,  -19,  -13,   -6,
];

fn sin(a: u8) -> i32 {
    SIN[a as usize] as i32
}

fn cos(a: u8) -> i32 {
    SIN[a.wrapping_add(64) as usize] as i32
}

// --- Movement ---

const MOVE_SPEED: i32 = 32; // fp 256 units per frame ⇒ ~1/8 tile/frame
const TURN_SPEED: u8 = 3; // angle units per frame ⇒ ~4.2°/frame

// --- Raycaster ---

// FOV ≈ 60° ⇒ tan(30°) ≈ 0.577. In fp 256: ~148. This is the camera
// plane's length; ray = dir + plane * camera_x with camera_x ∈ [-1, 1].
const PLANE_SCALE: i32 = 148;

const HORIZON_Y: u32 = HEIGHT / 2;

// Wall pseudo-height: a wall one tile away (perp_dist = TILE_SIZE = 256
// in fp) should cover the full screen height. So h = TILE_SIZE * HEIGHT
// / perp = 256 * 128 / perp = 32768 / perp.
const WALL_H_NUM: i32 = TILE_SIZE * HEIGHT as i32;

const MAX_DDA_STEPS: u32 = 64;

#[derive(Copy, Clone)]
struct RayHit {
    perp: i32,    // fp 256; "tile-widths along ray" * 256
    x_side: bool, // hit a vertical wall face (crossed an x-grid line)
}

fn cast_ray(px: i32, py: i32, rdx: i32, rdy: i32) -> Option<RayHit> {
    let mut map_x = px >> TILE_SHIFT;
    let mut map_y = py >> TILE_SHIFT;

    let step_x: i32 = if rdx >= 0 { 1 } else { -1 };
    let step_y: i32 = if rdy >= 0 { 1 } else { -1 };

    // Avoid div-by-zero; .max(1) is fine since a 0-component ray will
    // simply have astronomical side_dist on that axis and step only the
    // other one.
    let abs_rdx = rdx.unsigned_abs() as i64;
    let abs_rdy = rdy.unsigned_abs() as i64;
    let abs_rdx = abs_rdx.max(1);
    let abs_rdy = abs_rdy.max(1);

    // delta = t-distance (fp 256) to cross one full tile on this axis.
    // From dx/dt = rdx/256: t to cross TILE_SIZE = TILE_SIZE * 256 / rdx.
    let delta_x: i64 = (TILE_SIZE as i64 * 256) / abs_rdx;
    let delta_y: i64 = (TILE_SIZE as i64 * 256) / abs_rdy;

    // Sub-tile distance to the next grid line on each axis (fp 256).
    let sub_x = if step_x > 0 {
        TILE_SIZE - (px & (TILE_SIZE - 1))
    } else {
        (px & (TILE_SIZE - 1)).max(1)
    };
    let sub_y = if step_y > 0 {
        TILE_SIZE - (py & (TILE_SIZE - 1))
    } else {
        (py & (TILE_SIZE - 1)).max(1)
    };

    let mut side_dist_x: i64 = sub_x as i64 * 256 / abs_rdx;
    let mut side_dist_y: i64 = sub_y as i64 * 256 / abs_rdy;

    let mut x_side;
    for _ in 0..MAX_DDA_STEPS {
        if side_dist_x < side_dist_y {
            map_x += step_x;
            side_dist_x += delta_x;
            x_side = true;
        } else {
            map_y += step_y;
            side_dist_y += delta_y;
            x_side = false;
        }
        if is_wall(map_x, map_y) {
            let perp = if x_side {
                side_dist_x - delta_x
            } else {
                side_dist_y - delta_y
            };
            return Some(RayHit {
                perp: perp as i32,
                x_side,
            });
        }
    }
    None
}

// --- State ---

struct State {
    x: i32, // fp 256, 0..WORLD_MAX
    y: i32,
    angle: u8,
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

// --- Render ---

fn shade(x_side: bool) -> Color {
    if x_side { DARK_GREY } else { LIGHT_GREY }
}

fn render(state: &State) -> FrameBuffer {
    let mut fb = FrameBuffer::new();

    // Sky / floor.
    fb.draw(&DrawCommand::rect(0, 0, WIDTH, HORIZON_Y, DARK_BLUE));
    fb.draw(&DrawCommand::rect(
        0,
        HORIZON_Y,
        WIDTH,
        HEIGHT - HORIZON_Y,
        BROWN,
    ));

    let a = state.angle;
    let dir_x = cos(a);
    let dir_y = sin(a);
    // Camera plane perpendicular to dir, length = tan(FOV/2).
    let plane_x = -dir_y * PLANE_SCALE / 256;
    let plane_y = dir_x * PLANE_SCALE / 256;

    for col in 0..WIDTH as i32 {
        // camera_x ∈ [-1, 1] in fp 256, i.e. [-256, 256].
        let camera = (2 * col - WIDTH as i32) * 256 / WIDTH as i32;
        let rdx = dir_x + plane_x * camera / 256;
        let rdy = dir_y + plane_y * camera / 256;

        let Some(hit) = cast_ray(state.x, state.y, rdx, rdy) else {
            continue;
        };

        let h = (WALL_H_NUM / hit.perp.max(1)).min(HEIGHT as i32) as u32;
        let top = HORIZON_Y.saturating_sub(h / 2);
        let bottom = (HORIZON_Y + h / 2).min(HEIGHT);
        let color = shade(hit.x_side);
        fb.draw(&DrawCommand::rect(col as u32, top, 1, bottom - top, color));
    }

    // Crosshair.
    fb.draw(&DrawCommand::rect(WIDTH / 2 - 1, HEIGHT / 2, 3, 1, BLACK));
    fb.draw(&DrawCommand::rect(WIDTH / 2, HEIGHT / 2 - 1, 1, 3, BLACK));

    fb
}

// --- Update ---

fn fresh() -> State {
    // Spawn at tile (5, 2) facing +y (south). A wall at (5, 4) sits ~1.5
    // tiles dead ahead so the opening frame shows real depth, and turning
    // left sweeps the long east corridor.
    State {
        x: 5 * TILE_SIZE + TILE_SIZE / 2,
        y: 2 * TILE_SIZE + TILE_SIZE / 2,
        angle: 64,
    }
}

fn try_move(state: &mut State, dx: i32, dy: i32) {
    // Axis-independent: try x, then y. Keeps you from sticking on corners.
    let nx = state.x + dx;
    if nx > 0 && nx < WORLD_MAX && !is_wall(nx >> TILE_SHIFT, state.y >> TILE_SHIFT) {
        state.x = nx;
    }
    let ny = state.y + dy;
    if ny > 0 && ny < WORLD_MAX && !is_wall(state.x >> TILE_SHIFT, ny >> TILE_SHIFT) {
        state.y = ny;
    }
}

struct WolfGame;

impl Game for WolfGame {
    const NAME: &'static str = "Wolfenstein";
    const FPS: usize = 30;

    fn new(_args: Vec<String>) -> (u64, FrameBuffer) {
        let s = fresh();
        (encode(&s), render(&s))
    }

    fn update(
        state: u64,
        held: &[Key],
        _buffered: Option<Key>,
        _mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let mut s = decode(state);

        // Screen y points down, so angle increases CW in screen space.
        // Left arrow turns CCW (angle decreases); Right turns CW.
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

        (encode(&s), render(&s))
    }
}

fn main() {
    bitwise_games::run_game::<WolfGame>();
}
