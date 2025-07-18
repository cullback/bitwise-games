/*

- 64x64 board
- 40 bits: bricks for 5x8 bricks
-  6 bits: paddle position
- 12 bits: ball position, 6 bits each for x and y
-  2 bits: ball velocity. Ball can have 4 directions
-  4 bits: free

*/
use bitwise_games::Game;
use minifb::Key;

struct Breakout {
    paddle: u32,
    ball_x: u32,
    ball_y: u32,
}

fn from_u64(state: u64) -> Breakout {
    todo!()
}

fn to_u64(state: Breakout) -> u64 {
    todo!()
}

impl Game for Breakout {
    const NAME: &'static str = "Breakout";
    const WIDTH: usize = 640;
    const HEIGHT: usize = 640;
    const FPS: usize = 30;
    fn new(args: Vec<String>) -> (u64, Vec<u32>) {
        todo!()
    }

    fn update(state: u64, input: &[Key]) -> (u64, Vec<u32>) {
        todo!()
    }
}

fn main() {}
