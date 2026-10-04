//! Console discovery and scheduled terminal IO.

mod frame;
pub mod run;
mod schedule;
mod server;

pub const MS: usize = 1000;
pub const E_NO_CONSOLE: env::Reason = 1;
pub const E_TERMINAL: env::Reason = 2;
