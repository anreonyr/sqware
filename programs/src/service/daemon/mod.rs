//! One resident user-mode observer for the declared supervision group.

mod adapt;
mod core;

pub fn run() -> Result<(), &'static str> {
    adapt::run()
}
