# Game ideas

## 2048

- 16 cells
- 4 bits per cell
- Represent up to 2^15=32768
- Derive rng from state

## 15 puzzle

- 16 cells
- 4 bits per cell
- numbers 1 to 15
- One empty cell

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

- 8x8 board
- 6 bit head position
- 2 bit current direction
- array of 2 bit directions
- len/score derived from body
- 4 bit apple position, use last 2 bits of length

## Breakout

- paddle + ball position
- Bit array for block field

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
