//! Application screen views.
//!
//! Contains the two main screens of the application:
//! - [`WaitingScreen`]: Displayed when waiting for a game connection
//! - [`ActiveScreen`]: Displayed when actively connected and capturing data

mod active;
mod waiting;

pub use active::*;
pub use waiting::*;
