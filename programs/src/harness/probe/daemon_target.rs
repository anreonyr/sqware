//! Root harness for the ordinary non-DMA target with an actual helper task.

mod api;
mod ask;
mod beat;
mod control;
mod target;

pub use control::run as controller;
pub use target::run as fixture;

use crate::harness::probe::daemon_root::{self, Scene};

/// Private notification from the transient controller after its real heartbeat.
pub(crate) const REQUEST: env::Mark = env::Mark::of("daemon-target/request");

const SCENE: Scene = Scene {
    root: "target-root",
    target: "fixturetarget",
    controller: "daemon-ctl-target",
    request: REQUEST,
    done: "daemon-target: done",
    helper: true,
};

/// Run the ordinary target restart scene from the deployment bootstrap image.
pub fn run() -> Result<(), env::Reason> {
    daemon_root::run(SCENE)
}

/// Completion note shared by the Root bin and the independent checker.
pub fn done() -> &'static str {
    SCENE.done
}
