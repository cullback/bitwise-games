# Game ideas

## [x] 2048

- 16 cells
- 4 bits per cell
- Represent up to 2^15=32768
- Derive rng from state
- Derive score from board state (diverges from original game)

## [x] 15 puzzle

- State is the lex rank of the board permutation: an index into the
  16! ≈ 2.09e13 arrangements of {0..16} (0 = empty, 1..15 = tiles)
- ~44 bits used; top 20 bits sit at zero
- Only half of those permutations are solvable
  (perm_parity XOR empty_row_parity is invariant under legal slides)

## [x] Breakout

- 64x64 logical board, scaled up for display
- 40 bits: brick bitmap (5x8 bricks)
- 6 bits: paddle x position
- 12 bits: ball position (6 bits each for x, y)
- 2 bits: ball velocity (4 diagonal directions)
- 4 bits: free

## connect four

- 6x7 board = 42 cells
- 7*3=21 bit heightmap
- 42 x/o bitboard
- 63 bits total

## Hangman

- 26 bitmap for guessed letters

## Lights out

- 8x8 board
- 64 cells
- How to initialize random state?

## Game of life

- 8x8 board
- 64 cells

## Snake

- Want to maximize snake body length
- 8x8 board
- 6 bit head position
- 2 bit current direction
- array of 2 bit directions
- len/score derived from body
- 4 bit apple position, use last 2 bits of length
- 64 - 6 - 4- 2x

## Space invaders

- at least as hard as breakout

## Endless runner

- e.g. gravity guy, jetpack joyride
- Height
- Varying difficulty

## Flappy bird

- height
- Velocity

## Simon

- four colors, 2 bits per level
- Only 32 levels
- How to identify start?
- start with a larger part of memory being rng

## Wolfenstein

- x,y position
- direction
- health, bullets, enemies

## Lunar lander

- x,y position
- x,y velocity
- angle
- fuel?

## Don't see a way

### Minesweeper

- May not be enough
- 8x8

### Tetris
