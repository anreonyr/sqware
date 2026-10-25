//! Router constants owned by the provider API, shared by declaration and runtime.

use env::Mark;

/// Failure number for the router's startup/resident steps.
pub const E_ROUTER: env::Reason = 5;

/// Interrupt-controller compatible string; the device tree is the only source.
pub const PLIC_CLASS: &str = "sifive,plic-1.0.0";

pub const CHANNEL: &str = "router";
pub const READY_MARK: Mark = Mark::of("ready");
