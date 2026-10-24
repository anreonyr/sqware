#![cfg(test)]
#![allow(dead_code)]
extern crate alloc;

#[path = "../../src/service/daemon/core/decision.rs"]
mod decision;
#[path = "../../src/service/daemon/core/startup.rs"]
mod startup;

mod cases;
