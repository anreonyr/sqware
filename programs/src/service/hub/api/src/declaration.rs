//! Hub constants owned by the provider API, shared by declaration and runtime.

use env::Mark;

/// Failure number for the hub's startup/resident steps.
pub const E_HUB: env::Reason = 28;

pub const CHANNEL: &str = "hub";

/// The hub publishes this role at the end of startup.
pub const READY: &str = "hub-ready";
pub const READY_MARK: Mark = Mark::of(READY);
pub const CHANNEL_MARK: Mark = Mark::of(CHANNEL);
pub const INVENTORY_MARK: Mark = Mark::of("hub/machine-inventory");
