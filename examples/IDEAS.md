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
- some bits for index of hidden word

## Lights out

- 8x8 board
- 64 cells
- How to initialize random state?

## Conway's game of life

- 8x8 board
- 64 cells
- hold z and click cells to pause sim and construct?
- x to clear?

## [x] Snake

- 8x8 grid; 128×128 pixel display upscaled by client
- 6 bits head pos + 2 bits head dir + 3 bits apple entropy + 53 bits body
- body[0] is implied by head + direction; body[1..L] are L/S/R turns
  packed as a varlen base-3 integer
- Apple cell = `rng::next((length << 3) | apple_bits) % 64`, with
  spawn picking the first of 8 candidates that doesn't land on the
  snake; ~0.9% fallback collision rate at max length
- Death sentinel: body_int = (3^34 − 1)/2 (just past the largest valid
  varlen). Body shape lost on death (snake collapses to length 2 +
  X eyes); Z or X restarts
- Max snake length: 35

## Space invaders

- at least as hard as breakout

## Jetpack joyride

- ~4 bits animation flip key frames
- 6 bit height
- 4 bit velocity
- 32 bit counter

## Endless runner

- e.g. gravity guy, jetpack joyride, dino run
- Height
- Varying difficulty

## Flappy bird

- height
- Velocity

## Simon

- four colors, 2 bits per level
- use rng to generate the sequence. ~10 bits?
- we need a counter for how many guessed right out of how many
- 8 bits (255 levels)? starts at 1
- 8 bits: current counter
- 1 bit: replay mode / play mode
- use arrow keys for the four options, add colors to them

## Wolfenstein / Doom / Raycaster

- x,y position
- direction
- health, bullets, enemies, doors

## Lunar lander

- x,y position
- x,y velocity
- angle
- fuel?

## Pinball

- 32x8?
- ball x,y velocity
- ball position
- paddle animation x2

## Don't see a way

### Minesweeper

- May not be enough
- seed to represent board - 8 bits?
- 56 components we can click / flag, binary, either flag or reveal
- how to deal with incorrect flag?
- we could get a larger board than 8x8 if we set up components correctly

### Tetris

### Wordle

- 6 chances to guess a 5 letter word
- guess must be a valid word, we can store as entry
- need full keyboard entry?
- suppose an 8 bit word list
- 6 * 8 indexes = 48 bits
- maybe the working stored word is done with index + counter

initially

`xxxxx`

type a letter, find first index that matches
3 bits

- 6 * 10 indices = 60
- 1024 word list, :/

index 0 is blank. tells us which word we're working on
we start typing a letter, it picks first index that matches and increments counter. type more and more letters. if it typing a letter would create an invalid word, we flash a message that its invalid and don't increment counter

if counter = 0, take first 0 guess index
user types letter, matches onto a valid word, inserts the index into the first non zero guess slot, counter increments to 1

type, type, type, until counter = 5, user presses enter
if correct, counter goes back to zero, keeps the index in the slot with the guessed word.
