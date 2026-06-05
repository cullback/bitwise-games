/*

- 64x64 board
- 40 bits: bricks for 5x8 bricks
-  6 bits: paddle position
- 12 bits: ball position, 6 bits each for x and y
-  2 bits: ball velocity. Ball can have 4 directions
-  4 bits: free

*/
use bitwise_games::bits::{get_bits, set_bits};
use bitwise_games::draw_command::{
    BLUE, DARK_BLUE, DrawCommand, GREEN, ORANGE, RED, WHITE, YELLOW,
};
use bitwise_games::frame_buffer::FrameBuffer;
use bitwise_games::{Game, Key};

const N_BRICK_ROWS: u8 = 5;
const N_BRICK_COLS: u8 = 8;
const N_BRICKS: u8 = N_BRICK_ROWS * N_BRICK_COLS;

// Game board is 64×64 in logical pixels — that's the unit ball positions live
// in (they fit in 6 bits). The framebuffer is 128×128, so render at 2× scale.
const BOARD_WIDTH: u32 = 64;
const BOARD_HEIGHT: u32 = 64;
const SCALE: u32 = 2;

// Brick dimensions
const BRICK_WIDTH: u32 = 8;
const BRICK_HEIGHT: u32 = 4;

// Ball dimensions
const BALL_SIZE: u32 = 2;

// Paddle dimensions and position
const PADDLE_WIDTH: u32 = 12;
const PADDLE_HEIGHT: u32 = 2;
const PADDLE_Y: u32 = 62;

const PADDLE_VELOCITY: u8 = 2;

// Ball velocity directions
const BALL_UP_LEFT: u8 = 0;
const BALL_UP_RIGHT: u8 = 1;
const BALL_DOWN_LEFT: u8 = 2;
const BALL_DOWN_RIGHT: u8 = 3;

struct State {
    bricks: u64,
    paddle_pos: u8,
    ball_pos_x: u8,
    ball_pos_y: u8,
    ball_vel: u8,
}

fn decode(state: u64) -> State {
    let bricks = get_bits(state, 0, N_BRICKS as u8);
    let paddle_pos = get_bits(state, N_BRICKS as u8, 6);
    let ball_pos_x = get_bits(state, (N_BRICKS + 6) as u8, 6);
    let ball_pos_y = get_bits(state, (N_BRICKS + 12) as u8, 6);
    let ball_vel = get_bits(state, (N_BRICKS + 18) as u8, 2);
    State {
        bricks,
        paddle_pos,
        ball_pos_x,
        ball_pos_y,
        ball_vel,
    }
}

fn encode(state: &State) -> u64 {
    let mut result = 0u64;
    result = set_bits(result, state.bricks, 0, N_BRICKS as u8);
    result = set_bits(result, state.paddle_pos, N_BRICKS as u8, 6);
    result = set_bits(result, state.ball_pos_x, (N_BRICKS + 6) as u8, 6);
    result = set_bits(result, state.ball_pos_y, (N_BRICKS + 12) as u8, 6);
    result = set_bits(result, state.ball_vel, (N_BRICKS + 18) as u8, 2);
    result
}

fn flip_ball_horizontal(velocity: u8) -> u8 {
    match velocity {
        BALL_UP_LEFT => BALL_UP_RIGHT,
        BALL_UP_RIGHT => BALL_UP_LEFT,
        BALL_DOWN_LEFT => BALL_DOWN_RIGHT,
        BALL_DOWN_RIGHT => BALL_DOWN_LEFT,
        _ => velocity,
    }
}

fn flip_ball_vertical(velocity: u8) -> u8 {
    match velocity {
        BALL_UP_LEFT => BALL_DOWN_LEFT,
        BALL_UP_RIGHT => BALL_DOWN_RIGHT,
        BALL_DOWN_LEFT => BALL_UP_LEFT,
        BALL_DOWN_RIGHT => BALL_UP_RIGHT,
        _ => velocity,
    }
}

// Collision detection functions (pure - no state mutation)
fn check_wall_collision(ball_x: u8, ball_y: u8, dx: i8, dy: i8) -> (bool, bool) {
    let left_wall = ball_x == 0 && dx < 0;
    let right_wall = ball_x >= BOARD_WIDTH as u8 - BALL_SIZE as u8 && dx > 0;
    let top_wall = ball_y == 0 && dy < 0;
    let bottom_wall = ball_y >= BOARD_HEIGHT as u8 - BALL_SIZE as u8 && dy > 0;

    (left_wall || right_wall, top_wall || bottom_wall)
}

fn check_paddle_collision(ball_x: u8, ball_y: u8, old_ball_y: u8, paddle_pos: u8, dy: i8) -> bool {
    if dy <= 0 {
        return false; // Ball not moving down
    }

    let ball_bottom = ball_y + BALL_SIZE as u8;
    let old_ball_bottom = old_ball_y + BALL_SIZE as u8;
    let paddle_top = PADDLE_Y as u8;

    // Check if ball crossed paddle top this frame
    let crossed_paddle = ball_bottom >= paddle_top && old_ball_bottom < paddle_top;

    if !crossed_paddle {
        return false;
    }

    // Check horizontal overlap
    let ball_left = ball_x;
    let ball_right = ball_x + BALL_SIZE as u8;
    let paddle_left = paddle_pos;
    let paddle_right = paddle_pos + PADDLE_WIDTH as u8;

    ball_right > paddle_left && ball_left < paddle_right
}

fn find_brick_collision(ball_x: u8, ball_y: u8, bricks: u64) -> Option<u8> {
    if ball_y >= N_BRICK_ROWS * BRICK_HEIGHT as u8 {
        return None; // Ball below brick area
    }

    // Check all four corners of the ball for brick collision
    let corners = [
        (ball_x, ball_y),                                             // top-left
        (ball_x + BALL_SIZE as u8 - 1, ball_y),                       // top-right
        (ball_x, ball_y + BALL_SIZE as u8 - 1),                       // bottom-left
        (ball_x + BALL_SIZE as u8 - 1, ball_y + BALL_SIZE as u8 - 1), // bottom-right
    ];

    for (x, y) in corners {
        if x < BOARD_WIDTH as u8 && y < N_BRICK_ROWS * BRICK_HEIGHT as u8 {
            let brick_col = x / BRICK_WIDTH as u8;
            let brick_row = y / BRICK_HEIGHT as u8;
            let brick_index = brick_row * N_BRICK_COLS + brick_col;

            if brick_index < N_BRICKS && (bricks >> brick_index) & 1 == 1 {
                return Some(brick_index);
            }
        }
    }

    None
}

fn determine_brick_collision_direction(
    _ball_x: u8,
    _ball_y: u8,
    _old_ball_x: u8,
    old_ball_y: u8,
    brick_index: u8,
) -> bool {
    let brick_row = brick_index / N_BRICK_COLS;
    let _brick_col = brick_index % N_BRICK_COLS;
    let brick_y = brick_row * BRICK_HEIGHT as u8;

    // Check if the ball was horizontally aligned with the brick in the previous frame
    let old_ball_bottom = old_ball_y + BALL_SIZE as u8;
    let old_ball_top = old_ball_y;
    let brick_bottom = brick_y + BRICK_HEIGHT as u8;
    let brick_top = brick_y;

    let was_vertically_aligned = old_ball_bottom > brick_top && old_ball_top < brick_bottom;

    // If ball was vertically aligned, it's a horizontal collision
    // Otherwise, it's a vertical collision
    !was_vertically_aligned
}

fn render(state: &State) -> FrameBuffer {
    let mut fb = FrameBuffer::new();
    let mut cmds = Vec::new();

    cmds.push(DrawCommand::rect(
        0,
        0,
        BOARD_WIDTH * SCALE,
        BOARD_HEIGHT * SCALE,
        DARK_BLUE,
    ));

    let brick_colors = [RED, ORANGE, YELLOW, GREEN, BLUE];
    for i in 0..N_BRICKS {
        if (state.bricks >> i) & 1 == 1 {
            let row = u32::from(i / N_BRICK_COLS);
            let col = u32::from(i % N_BRICK_COLS);
            cmds.push(DrawCommand::rect(
                col * BRICK_WIDTH * SCALE,
                row * BRICK_HEIGHT * SCALE,
                BRICK_WIDTH * SCALE,
                BRICK_HEIGHT * SCALE,
                brick_colors[row as usize],
            ));
        }
    }

    cmds.push(DrawCommand::rect(
        state.paddle_pos as u32 * SCALE,
        PADDLE_Y * SCALE,
        PADDLE_WIDTH * SCALE,
        PADDLE_HEIGHT * SCALE,
        WHITE,
    ));

    cmds.push(DrawCommand::rect(
        state.ball_pos_x as u32 * SCALE,
        state.ball_pos_y as u32 * SCALE,
        BALL_SIZE * SCALE,
        BALL_SIZE * SCALE,
        WHITE,
    ));

    fb.draw_list(&cmds);
    fb
}

// Collision response functions
fn reset_ball_position(state: &mut State) {
    state.ball_pos_x = (BOARD_WIDTH / 2 - BALL_SIZE / 2) as u8;
    state.ball_pos_y = (BOARD_HEIGHT - PADDLE_HEIGHT - BALL_SIZE) as u8;
    state.ball_vel = BALL_UP_RIGHT;
}

fn handle_wall_collision(
    state: &mut State,
    old_ball_x: u8,
    old_ball_y: u8,
    horizontal_hit: bool,
    vertical_hit: bool,
) {
    if horizontal_hit {
        state.ball_vel = flip_ball_horizontal(state.ball_vel);
        state.ball_pos_x = old_ball_x;
    }
    if vertical_hit {
        state.ball_vel = flip_ball_vertical(state.ball_vel);
        state.ball_pos_y = old_ball_y;
    }
}

fn handle_paddle_collision(state: &mut State, old_ball_y: u8) {
    state.ball_vel = flip_ball_vertical(state.ball_vel);
    state.ball_pos_y = old_ball_y;
}

fn handle_brick_collision(
    state: &mut State,
    old_ball_x: u8,
    old_ball_y: u8,
    brick_index: u8,
    is_vertical: bool,
) {
    // Remove the brick
    state.bricks &= !(1 << brick_index);

    // Bounce the ball
    if is_vertical {
        state.ball_vel = flip_ball_vertical(state.ball_vel);
        state.ball_pos_y = old_ball_y;
    } else {
        state.ball_vel = flip_ball_horizontal(state.ball_vel);
        state.ball_pos_x = old_ball_x;
    }
}

fn handle_collisions(state: &mut State, dx: i8, dy: i8, old_ball_x: u8, old_ball_y: u8) {
    // Check wall collisions
    let (horizontal_wall, vertical_wall) =
        check_wall_collision(state.ball_pos_x, state.ball_pos_y, dx, dy);

    // Check for bottom wall (game over)
    if state.ball_pos_y >= BOARD_HEIGHT as u8 - BALL_SIZE as u8 && dy > 0 {
        reset_ball_position(state);
        return;
    }

    // Handle wall bounces
    if horizontal_wall || vertical_wall {
        handle_wall_collision(
            state,
            old_ball_x,
            old_ball_y,
            horizontal_wall,
            vertical_wall,
        );
        return;
    }

    // Check paddle collision
    if check_paddle_collision(
        state.ball_pos_x,
        state.ball_pos_y,
        old_ball_y,
        state.paddle_pos,
        dy,
    ) {
        handle_paddle_collision(state, state.ball_pos_y);
        return;
    }

    // Check brick collision
    if let Some(brick_index) =
        find_brick_collision(state.ball_pos_x, state.ball_pos_y, state.bricks)
    {
        let is_vertical = determine_brick_collision_direction(
            state.ball_pos_x,
            state.ball_pos_y,
            old_ball_x,
            old_ball_y,
            brick_index,
        );
        handle_brick_collision(state, old_ball_x, old_ball_y, brick_index, is_vertical);
        return;
    }
}

fn update_paddle_position(paddle_pos: u8, input: &[Key]) -> u8 {
    let mut new_paddle_pos = paddle_pos;
    if input.contains(&Key::Left) {
        if paddle_pos > 0 {
            new_paddle_pos -= PADDLE_VELOCITY;
        }
    }
    if input.contains(&Key::Right) {
        if paddle_pos < BOARD_WIDTH as u8 - PADDLE_WIDTH as u8 {
            new_paddle_pos += PADDLE_VELOCITY;
        }
    }
    new_paddle_pos
}

struct Breakout;

impl Game for Breakout {
    const NAME: &'static str = "Breakout";
    const FPS: usize = 30;

    fn new(_args: Vec<String>) -> (u64, FrameBuffer) {
        let state = State {
            bricks: (1 << N_BRICKS) - 1,
            paddle_pos: ((BOARD_WIDTH - PADDLE_WIDTH) / 2) as u8,
            ball_pos_x: ((BOARD_WIDTH - BALL_SIZE) / 2) as u8,
            ball_pos_y: 57, // just above paddle
            ball_vel: BALL_UP_RIGHT,
        };
        (encode(&state), render(&state))
    }

    fn update(
        state: u64,
        held: &[Key],
        _buffered: Option<Key>,
        _mouse: Option<(u8, u8)>,
    ) -> (u64, FrameBuffer) {
        let mut state = decode(state);

        state.paddle_pos = update_paddle_position(state.paddle_pos, held);

        // Move ball
        let (dx, dy) = match state.ball_vel {
            BALL_UP_LEFT => (-1, -1),
            BALL_UP_RIGHT => (1, -1),
            BALL_DOWN_LEFT => (-1, 1),
            BALL_DOWN_RIGHT => (1, 1),
            _ => unreachable!(),
        };

        let old_ball_x = state.ball_pos_x;
        let old_ball_y = state.ball_pos_y;

        state.ball_pos_x = (state.ball_pos_x as i8 + dx) as u8;
        state.ball_pos_y = (state.ball_pos_y as i8 + dy) as u8;

        handle_collisions(&mut state, dx, dy, old_ball_x, old_ball_y);

        (encode(&state), render(&state))
    }
}

fn main() {
    bitwise_games::run_game::<Breakout>();
}
