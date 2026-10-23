use core::time::Duration;
use ipc::session::establish;
use system_api::control::Fail;

use super::backoff::Backoff;
use super::intake;
use super::observe::observe;
use super::recover::recover;
use crate::service::daemon::core::{Decision, Startup, decide};
use crate::unit::READY_MARK;

const STARTUP_MS: usize = 5000;
const IDLE_MS: usize = 100;

pub fn run() -> Result<(), &'static str> {
    let faces = intake::open()?;
    establish::lend(env::unit::sire(), READY_MARK).map_err(|_| "daemon: ready mark")?;
    let mut startup = Startup::new(millis(STARTUP_MS));
    let mut backoff = Backoff::new();
    let mut index = 0u32;
    loop {
        match observe(&faces.observe, index) {
            Ok(reply) => {
                startup.observed();
                match decide(&reply) {
                    Decision::End => {
                        backoff.reset();
                        index = 0;
                        pause(IDLE_MS);
                    }
                    Decision::Advance => {
                        backoff.reset();
                        index = index.saturating_add(1);
                    }
                    Decision::Recover => {
                        if recover(&faces.recover, reply.name).is_err() {
                            backoff.wait();
                        } else {
                            backoff.reset();
                            pause(IDLE_MS);
                        }
                    }
                }
            }
            Err(Fail::Denied) => {
                if startup.deny_is_fatal(env::chrono::clock()) {
                    return Err("daemon: scope denied");
                }
                backoff.wait();
            }
            Err(_) => backoff.wait(),
        }
    }
}

fn millis(millis: usize) -> u64 {
    env::chrono::clock().saturating_add((millis as u64).saturating_mul(1_000_000))
}

fn pause(millis: usize) {
    let _ = execution::room::park(Duration::from_millis(millis as u64));
}
