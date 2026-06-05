# Bitwise Games

A library for games that only use 64 bits of internal state.

Inspired by pico-8

- 64 bits of state
- 6 buttons + mouse
- 128x128 display, 16 color

A game is defined by two pure functions.

```rust
/// Initialize the state from command line args
fn new(args: &[&str]) -> u64;

/// Compute next state based on current state and pressed keys
fn next(state: u64, keys_pressed: Vec<Bool>) -> (u64, Vec<u32>);
```

## Links

- <https://github.com/zesterer/the-bitwise-challenge>
- <https://www.andreinc.net/2022/05/01/4-integers-are-enough-to-write-a-snake-game>
