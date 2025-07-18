pub mod draw_command;
pub mod frame_buffer;
mod game;
mod input;
mod output;

pub use game::Game;
use minifb::{Key, Window, WindowOptions};
use std::env;
use std::time::{Duration, Instant};

pub fn run_game<T: Game>() {
    let mut window = Window::new(T::NAME, T::WIDTH, T::HEIGHT, WindowOptions::default()).unwrap();

    let millis_per_frame = (1000 / T::FPS) as u64;
    window.set_target_fps(T::FPS);

    let args: Vec<String> = env::args().collect();
    let mut state = T::new(args);
    let frame_duration = Duration::from_millis(millis_per_frame);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let frame_start = Instant::now();

        // let (framebuffer, new_state) = game.update(state, &keys);
        // state = new_state;
        let framebuffer = vec![0; T::WIDTH * T::HEIGHT];

        window
            .update_with_buffer(&framebuffer, T::WIDTH, T::HEIGHT)
            .unwrap();

        let elapsed = frame_start.elapsed();
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        }
    }
}
