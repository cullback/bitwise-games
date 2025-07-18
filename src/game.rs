use minifb::Key;

pub trait Game {
    const NAME: &'static str;
    const FPS: usize;
    const WIDTH: usize;
    const HEIGHT: usize;

    fn new(args: Vec<String>) -> (u64, Vec<u32>);
    fn update(state: u64, keys: &[Key]) -> (u64, Vec<u32>);
}
