# Conventions

- `Game::new` and `Game::update` must be pure functions. No `SystemTime::now()`,
  no env reads, no I/O. Any nondeterminism (e.g. a scramble seed) comes in
  through `args` so the same inputs produce the same state.
