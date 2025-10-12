
# Game loop

1. convert u64 to state object
2. process inputs and update state
  - helps make the game feel more responsive instead of updating at end
3. render based on state

First update the paddle position based on input, then check for collisions.
Why?
1. Causality and input-first logic

Game ticks simulate time moving forward. Inputs are interpreted as actions taken in that frame.
So the input (e.g., move paddle left/right) affects the world before any collision detection happens.
It matches player expectations: when they press a key, the paddle moves, and then the world responds accordingly.
2. Collision depends on new positions

The paddle must be in its new position before we can accurately check if a ball hits it.
Otherwise, the ball could miss the paddle visually but still be treated as a hit, or vice versa.
