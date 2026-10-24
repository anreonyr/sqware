//! Root harness for the actual RTC restart scene.

mod control;
pub use control::run as controller;

use crate::harness::probe::daemon_root::{self, Scene};

/// Private notification from the transient controller after its real time reply.
pub(crate) const REQUEST: env::Mark = env::Mark::of("daemon-rtc/request");

const SCENE: Scene = Scene {
    root: "rtc-root",
    target: "rtc",
    controller: "daemon-ctl-rtc",
    request: REQUEST,
    done: "daemon-rtc: done",
    helper: false,
    mode: crate::harness::probe::daemon_root::Mode::Recover,
    ack: None,
};

/// Run the actual RTC restart scene from the deployment bootstrap image.
pub fn run() -> Result<(), env::Reason> {
    daemon_root::run(SCENE)
}

/// Completion note shared by the Root bin and the independent checker.
pub fn done() -> &'static str {
    SCENE.done
}
