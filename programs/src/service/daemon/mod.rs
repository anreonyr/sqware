//! One resident user-mode observer for the declared supervision group.

mod adapt;
mod core;

/// The daemon could not open its scoped faces or lost that scope.
pub const E_DAEMON: env::Reason = 13;

pub fn run() -> Result<(), &'static str> {
    adapt::run()
}
