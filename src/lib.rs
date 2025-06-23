pub trait Game {
    fn new() -> u64;
    fn update(&self, state: u64) -> u64;
}
