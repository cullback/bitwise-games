use crate::Game;
use minifb::{Key, Window, WindowOptions};
use std::env;
use std::time::{Duration, Instant};

pub fn run_game<T: Game>() {
    let mut window = Window::new(T::NAME, T::WIDTH, T::HEIGHT, WindowOptions::default()).unwrap();

    let args: Vec<String> = env::args().collect();
    let (mut game_state, mut framebuffer) = T::new(args);

    let frame_duration = Duration::from_millis(1000 / T::FPS as u64);
    let mut prev_held: Vec<Key> = Vec::new();

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let frame_start = Instant::now();

        let held = window.get_keys();
        let pressed: Vec<Key> = held
            .iter()
            .filter(|k| !prev_held.contains(k))
            .copied()
            .collect();
        (game_state, framebuffer) = T::update(game_state, &held, &pressed);
        prev_held = held;

        window
            .update_with_buffer(&framebuffer, T::WIDTH, T::HEIGHT)
            .unwrap();

        let elapsed = frame_start.elapsed();
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        }
    }
}
