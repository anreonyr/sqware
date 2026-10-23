//! Root harness for the explicit-stop race and outsider-scope proof.

mod control;
pub use control::run as controller;

use crate::harness::probe::daemon_root::{self, Mode, Scene};

/// Private notification from the transient controller after its real heartbeat.
pub(crate) const REQUEST: env::Mark = env::Mark::of("daemon-stop/request");
/// Private ACK hole lent by the transient controller to Root.
pub(crate) const ACK: env::Mark = env::Mark::of("daemon-stop/ack");

const SCENE: Scene = Scene {
    root: "stop-root",
    target: "fixturetarget",
    controller: "daemon-ctl-stop",
    request: REQUEST,
    done: "daemon-stop: done",
    helper: true,
    mode: Mode::Stop,
    ack: Some(ACK),
};

/// Run the explicit-stop scene from the deployment bootstrap image.
pub fn run() -> Result<(), env::Reason> {
    daemon_root::run(SCENE)
}

/// Completion note shared by the Root bin and the independent checker.
pub fn done() -> &'static str {
    SCENE.done
}
