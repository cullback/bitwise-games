use minifb::{Key, Window, WindowOptions};

const WIDTH: usize = 640;
const HEIGHT: usize = 360;

fn main() {
    let mut buffer: Vec<u32> = vec![0; WIDTH * HEIGHT];

    let mut window = Window::new(
        "Rectangle - ESC to exit",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )
    .unwrap_or_else(|e| {
        panic!("{}", e);
    });

    window.set_target_fps(30);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Draw a rectangle
        for y in 100..200 {
            for x in 100..300 {
                buffer[y * WIDTH + x] = 0x00_FF_00_00; // Red
            }
        }

        // We unwrap here as we want this code to exit if it fails. Real applications may want to handle this in a different way
        window.update_with_buffer(&buffer, WIDTH, HEIGHT).unwrap();

        let keys = window.get_keys();
        if !keys.is_empty() {
            println!("Keys pressed: {:?}", keys);
        }
    }
}
