use crate::Game;
use minifb::{Key, Window, WindowOptions};
use std::collections::VecDeque;
use std::env;
use std::time::{Duration, Instant};

const BUFFER_CAP: usize = 4;

fn allowed(k: &Key) -> bool {
    matches!(
        k,
        Key::Up | Key::Down | Key::Left | Key::Right | Key::Z | Key::X
    )
}

pub fn run_game<T: Game>() {
    let mut window = Window::new(T::NAME, T::WIDTH, T::HEIGHT, WindowOptions::default()).unwrap();

    let args: Vec<String> = env::args().collect();
    let (mut game_state, mut framebuffer) = T::new(args);

    let frame_duration = Duration::from_millis(1000 / T::FPS as u64);
    let mut prev_held: Vec<Key> = Vec::new();
    let mut buffer: VecDeque<Key> = VecDeque::with_capacity(BUFFER_CAP);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let frame_start = Instant::now();

        let held: Vec<Key> = window.get_keys().into_iter().filter(allowed).collect();
        // Queue rising-edge presses (this frame's `held` minus last frame's).
        for k in &held {
            if !prev_held.contains(k) && buffer.len() < BUFFER_CAP {
                buffer.push_back(*k);
            }
        }
        let buffered: Vec<Key> = buffer.pop_front().into_iter().collect();

        (game_state, framebuffer) = T::update(game_state, &held, &buffered);
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
