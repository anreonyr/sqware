#![allow(dead_code)]
extern crate alloc;
extern crate env as abi_env;
extern crate lisp;
extern crate pipe_api;
extern crate shell_api;
extern crate stream;
extern crate system_api;

extern crate self as env;
extern crate self as ipc;
extern crate self as pipe_client;
extern crate self as system_client;

pub use abi_env::{
    ExitCause, JoinReply, PieToken, Reason, TaskExit, TaskId, TeamId, ToleEventSource, UnitFail,
    UnitTarget, Wait,
};
pub use pipe_api::Direction;

pub mod backend;
pub mod chrono;
pub mod control;
pub mod native;
pub mod rpc;
pub mod unit;

mod client;
mod creating;
mod pipe;
mod port;

pub use client::Client;
pub use creating::Creating;
pub use pipe::Pipe;
pub use port::Port;

#[path = "../../src/user/shell/core/mod.rs"]
pub mod core;

pub mod adapt;

#[cfg(test)]
mod cases;
