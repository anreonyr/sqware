//! Production's `order::sync` is `fence iorw, iorw`, which assembles only on riscv64.
//!
//! On the host there is no device bus to order against: the driver and the fake device
//! are one thread, so the program order of its stores already is the order the device
//! sees. That makes this shim the weakest link in the fixture - it is a no-op, not a
//! fence - which is why `tests.rs` claims nothing about weak ordering.

pub fn sync() {}
