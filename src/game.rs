use minifb::Key;

pub trait Game {
    const NAME: &'static str;
    const FPS: usize;
    const WIDTH: usize;
    const HEIGHT: usize;

    fn new(args: Vec<String>) -> u64;
    fn update(&self, state: u64, keys: &[Key]) -> (Vec<u32>, u64);
}
