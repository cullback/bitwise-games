/*

- 64x64 board
- 40 bits: bricks for 5x8 bricks
-  6 bits: paddle position
- 12 bits: ball position, 6 bits each for x and y
-  2 bits: ball velocity. Ball can have 4 directions
-  4 bits: free

*/
use bitwise_games::draw_command::{
    BLUE, DARK_BLUE, DrawCommand, GREEN, ORANGE, RED, Rectangle, WHITE, YELLOW,
};

use bitwise_games::Game;
use bitwise_games::frame_buffer::FrameBuffer;
use minifb::Key;

const BRICK_ROWS: usize = 5;
const BRICK_COLS: usize = 8;
const BRICKS: usize = BRICK_ROWS * BRICK_COLS;

struct Breakout {
    bricks: u64,
    paddle_pos: u8,
    ball_pos_x: u8,
    ball_pos_y: u8,
    ball_vel: u8,
}

fn from_u64(state: u64) -> Breakout {
    let bricks = state & ((1 << BRICKS) - 1);
    let paddle_pos = ((state >> BRICKS) & 0x3F) as u8;
    let ball_pos_x = ((state >> (BRICKS + 6)) & 0x3F) as u8;
    let ball_pos_y = ((state >> (BRICKS + 12)) & 0x3F) as u8;
    let ball_vel = ((state >> (BRICKS + 18)) & 0x3) as u8;
    Breakout {
        bricks,
        paddle_pos,
        ball_pos_x,
        ball_pos_y,
        ball_vel,
    }
}

fn to_u64(state: &Breakout) -> u64 {
    state.bricks
        | (state.paddle_pos as u64) << BRICKS
        | (state.ball_pos_x as u64) << (BRICKS + 6)
        | (state.ball_pos_y as u64) << (BRICKS + 12)
        | (state.ball_vel as u64) << (BRICKS + 18)
}

fn draw(state: &Breakout) -> Vec<u32> {
    let mut fb = FrameBuffer::new(Breakout::WIDTH as u32, Breakout::HEIGHT as u32);
    let scale = (Breakout::WIDTH / 64) as u32;

    // Draw background
    fb.draw(&DrawCommand::Rectangle(Rectangle {
        x: 0,
        y: 0,
        width: Breakout::WIDTH as u32,
        height: Breakout::HEIGHT as u32,
        color: DARK_BLUE,
    }));

    // Draw bricks
    let brick_width: u8 = 8;
    let brick_height: u8 = 4;
    let brick_colors = [RED, ORANGE, YELLOW, GREEN, BLUE];
    for i in 0..BRICKS {
        if (state.bricks >> i) & 1 == 1 {
            let row = i / BRICK_COLS;
            let col = i % BRICK_COLS;
            let rect = DrawCommand::Rectangle(Rectangle {
                x: (col as u32 * brick_width as u32) * scale,
                y: (row as u32 * brick_height as u32) * scale,
                width: brick_width as u32 * scale,
                height: brick_height as u32 * scale,
                color: brick_colors[row],
            });
            fb.draw(&rect);
        }
    }

    // Draw paddle
    let paddle_width: u8 = 12;
    let paddle_height: u8 = 2;
    let paddle_y: u8 = 60;
    let paddle = DrawCommand::Rectangle(Rectangle {
        x: state.paddle_pos as u32 * scale,
        y: paddle_y as u32 * scale,
        width: paddle_width as u32 * scale,
        height: paddle_height as u32 * scale,
        color: WHITE,
    });
    fb.draw(&paddle);

    // Draw ball
    let ball_size: u8 = 2;
    let ball = DrawCommand::Rectangle(Rectangle {
        x: state.ball_pos_x as u32 * scale,
        y: state.ball_pos_y as u32 * scale,
        width: ball_size as u32 * scale,
        height: ball_size as u32 * scale,
        color: WHITE,
    });
    fb.draw(&ball);

    fb.pixels
}

impl Game for Breakout {
    const NAME: &'static str = "Breakout";
    const WIDTH: usize = 640;
    const HEIGHT: usize = 640;
    const FPS: usize = 30;

    fn new(_args: Vec<String>) -> (u64, Vec<u32>) {
        let state = Breakout {
            bricks: (1 << BRICKS) - 1,
            paddle_pos: 26, // (64-12)/2
            ball_pos_x: 31, // (64-2)/2
            ball_pos_y: 57, // just above paddle
            ball_vel: 1,    // up-right
        };
        let state_u64 = to_u64(&state);
        let fb = draw(&state);
        (state_u64, fb)
    }

    fn update(state_u64: u64, input: &[Key]) -> (u64, Vec<u32>) {
        let mut state = from_u64(state_u64);
        let paddle_width: u8 = 12;

        // Move paddle
        if input.contains(&Key::Left) {
            if state.paddle_pos > 0 {
                state.paddle_pos -= 2;
            }
        }
        if input.contains(&Key::Right) {
            if state.paddle_pos < 64 - paddle_width {
                state.paddle_pos += 2;
            }
        }

        // Move ball
        let (dx, dy) = match state.ball_vel {
            0 => (-1, -1), // up-left
            1 => (1, -1),  // up-right
            2 => (-1, 1),  // down-left
            _ => (1, 1),   // down-right (3)
        };

        let old_ball_x = state.ball_pos_x;
        let old_ball_y = state.ball_pos_y;

        state.ball_pos_x = (state.ball_pos_x as i8 + dx) as u8;
        state.ball_pos_y = (state.ball_pos_y as i8 + dy) as u8;

        // Collision
        let ball_size: u8 = 2;

        // Left/Right walls
        if (state.ball_pos_x == 0 && dx < 0) || (state.ball_pos_x >= 64 - ball_size && dx > 0) {
            state.ball_vel ^= 1;
            state.ball_pos_x = old_ball_x;
        }

        // Top wall
        if state.ball_pos_y == 0 && dy < 0 {
            state.ball_vel ^= 2;
            state.ball_pos_y = old_ball_y;
        }

        // Bottom wall (lose)
        if state.ball_pos_y >= 64 - ball_size && dy > 0 {
            // Reset ball
            state.ball_pos_x = 31;
            state.ball_pos_y = 57;
            state.ball_vel = 1;
        }

        // Paddle
        let paddle_y: u8 = 60;
        if dy > 0 && state.ball_pos_y + ball_size >= paddle_y && old_ball_y + ball_size < paddle_y {
            if state.ball_pos_x + ball_size > state.paddle_pos
                && state.ball_pos_x < state.paddle_pos + paddle_width
            {
                state.ball_vel ^= 2;
                state.ball_pos_y = old_ball_y;
            }
        }

        // Bricks
        let brick_width: u8 = 8;
        let brick_height: u8 = 4;
        if state.ball_pos_y < (BRICK_ROWS * brick_height as usize) as u8 {
            let brick_col = (state.ball_pos_x + ball_size / 2) / brick_width;
            let brick_row = (state.ball_pos_y + ball_size / 2) / brick_height;
            let brick_index = (brick_row as usize * BRICK_COLS) + brick_col as usize;

            if brick_index < BRICKS && (state.bricks >> brick_index) & 1 == 1 {
                state.bricks &= !(1 << brick_index);

                // Check for side collision vs top/bottom collision
                let brick_x = brick_col * brick_width;
                let brick_y = brick_row * brick_height;

                let overlap_x =
                    (old_ball_x + ball_size > brick_x) && (old_ball_x < brick_x + brick_width);

                if overlap_x {
                    state.ball_vel ^= 2; // Vertical collision
                    state.ball_pos_y = old_ball_y;
                } else {
                    state.ball_vel ^= 1; // Horizontal collision
                    state.ball_pos_x = old_ball_x;
                }
            }
        }

        let new_state_u64 = to_u64(&state);
        let fb = draw(&state);
        (new_state_u64, fb)
    }
}

fn main() {
    bitwise_games::run_game::<Breakout>();
}
