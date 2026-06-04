pub mod bits;
pub mod draw_command;
pub mod frame_buffer;
mod game;
pub mod permutation;

#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "server")]
mod server;

pub use game::Game;

#[cfg(feature = "desktop")]
pub use desktop::run_game;

#[cfg(all(feature = "server", not(feature = "desktop")))]
pub use server::run_game;
