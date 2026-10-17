//! Production supply retention with an independently stateful external accessor.
#![allow(dead_code)]
extern crate alloc;

#[cfg(test)]
#[path = "../../../crates/resource/src/retention/mod.rs"]
mod retention;

#[cfg(test)]
mod accessor;
#[cfg(test)]
mod ordering;
#[cfg(test)]
mod platform;
#[cfg(test)]
mod references;
