use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use core::time::Duration;

use ::resource::raw::Hole;
use ::schedule::{Progress, Res, ResMut};
use env::{HoleCondition, Permission, TaskId, Wait, pie};
use execution::unit::task;
use ipc::session::establish;
use system_api::control::{Grant, State};
use system_client::control::Face;

use super::stop_state::StopState;
use super::{Root, Scene};
use crate::system::app::Fault;
use crate::system::control::identity::Roster;
use crate::system::control::namespace::Endpoints;
use crate::system::control::unit::Control;
use crate::system::control::unit::table::State as InternalState;

const ACTOR_NS: u64 = 15_000_000_000;
const RPC_MS: usize = 5000;

pub(super) struct Stop {
    pub actor: TaskId,
    pub state: Arc<StopState>,
    pub root: &'static str,
    pub target: &'static str,
    pub controller: &'static str,
    pub authorized: bool,
}

pub(super) fn install(settings: &Scene) -> (Stop, task::Join<()>) {
    let state = Arc::new(StopState::new());
    let signal = state.clone();
    let root = env::unit::self_id();
    let target = settings.target;
    let worker = task::spawn(move || run_actor(root, target, signal));
    let stop = Stop {
        actor: worker.id(),
        state,
        root: settings.root,
        target: settings.target,
        controller: settings.controller,
        authorized: false,
    };
    (stop, worker)
}

fn run_actor(root: TaskId, target: &'static str, signal: Arc<StopState>) {
    let until = env::chrono::clock().saturating_add(ACTOR_NS);
    while !(signal.armed.load(Ordering::Acquire) && signal.kill.load(Ordering::Acquire)) {
        if env::chrono::clock() >= until {
            signal.failed.store(true, Ordering::Release);
            crate::debug::put(&alloc::format!(
                "daemon-stop-actor: target={target} wait=timeout"
            ));
            return;
        }
        let _ = execution::room::park(Duration::from_millis(1));
    }
    let Some(ruin) = establish::find(root, Grant::Ruin.mark())
        .ok()
        .and_then(|token| Face::of(token).ok())
    else {
        signal.failed.store(true, Ordering::Release);
        crate::debug::put(&alloc::format!("daemon-stop-actor: target={target} ruin=denied"));
        return;
    };
    let Some(state) = establish::find(root, Grant::State.mark())
        .ok()
        .and_then(|token| Face::of(token).ok())
    else {
        signal.failed.store(true, Ordering::Release);
        crate::debug::put(&alloc::format!("daemon-stop-actor: target={target} state=denied"));
        return;
    };
    if ruin
        .service(target.into())
        .ruin(Wait::AtMost(RPC_MS))
        .is_err()
    {
        signal.failed.store(true, Ordering::Release);
        crate::debug::put(&alloc::format!(
            "daemon-stop-actor: target={target} reply=failed"
        ));
        return;
    }
    signal.ruin.store(true, Ordering::Release);
    crate::debug::put(&alloc::format!("daemon-stop-actor: target={target} reply=ok"));
    match state.service(target.into()).state(Wait::AtMost(RPC_MS)) {
        Ok(State::Dead) => {
            signal.dead.store(true, Ordering::Release);
            crate::debug::put(&alloc::format!("daemon-stop-actor: target={target} state=Dead"));
        }
        Ok(other) => {
            signal.failed.store(true, Ordering::Release);
            crate::debug::put(&alloc::format!(
                "daemon-stop-actor: target={target} state={other:?}"
            ));
        }
        Err(fail) => {
            signal.failed.store(true, Ordering::Release);
            crate::debug::put(&alloc::format!(
                "daemon-stop-actor: target={target} state={fail:?}"
            ));
        }
    }
}

pub(super) fn authorize(
    mut stop: ResMut<Stop>,
    mut roster: ResMut<Roster>,
    mut control: ResMut<Control>,
) -> Result<Progress, Fault> {
    if stop.authorized || stop.state.armed.load(Ordering::Acquire) {
        return Ok(Progress::Done);
    }
    if roster.control().is_none() || control.task(stop.controller).is_none() {
        return Ok(Progress::Done);
    }
    roster
        .inherit(stop.actor, env::unit::self_id())
        .map_err(|_| Fault::Room)?;
    control
        .register_internal(stop.actor)
        .map_err(|_| Fault::Room)?;
    stop.authorized = true;
    crate::debug::put(&alloc::format!(
        "{}: actor task={} identity=inherited",
        stop.root,
        stop.actor.get(),
    ));
    Ok(Progress::Done)
}

pub(super) fn deliver(
    stop: Res<Stop>,
    endpoints: Res<Endpoints>,
) -> Result<Progress, Fault> {
    if !stop.authorized || stop.state.armed.load(Ordering::Acquire) {
        return Ok(Progress::Done);
    }
    let grants = [Grant::Ruin, Grant::State];
    if grants
        .iter()
        .any(|grant| endpoints.face(*grant).is_none())
    {
        return Ok(Progress::Done);
    }
    for grant in grants {
        let face = endpoints.face(grant).ok_or(Fault::Room)?;
        pie::accord(face, stop.actor, Permission::STORE, grant.mark())
            .map_err(|_| Fault::Room)?;
    }
    stop.state.armed.store(true, Ordering::Release);
    crate::debug::put(&alloc::format!(
        "{}: actor task={} faces=ruin,state",
        stop.root,
        stop.actor.get(),
    ));
    Ok(Progress::Done)
}

pub(super) fn release(root: Res<Root>, stop: Res<Stop>) -> Result<Progress, Fault> {
    if root.killed {
        stop.state.kill.store(true, Ordering::Release);
    }
    Ok(Progress::Done)
}

pub(super) fn reap(mut root: ResMut<Root>) -> Result<Progress, Fault> {
    if !root.killed || root.reaped {
        return Ok(Progress::Done);
    }
    let Some((old, _)) = root.old else {
        return Err(Fault::Room);
    };
    if matches!(
        env::unit::join_task(old, Wait::AtMost(1)),
        Ok(true)
    ) {
        root.reaped = true;
        return Ok(Progress::Done);
    }
    if env::chrono::clock() >= root.deadline {
        return Err(Fault::Room);
    }
    Ok(Progress::Pending)
}

pub(super) fn queue(
    mut root: ResMut<Root>,
    endpoints: Res<Endpoints>,
    stop: Res<Stop>,
) -> Result<Progress, Fault> {
    if !root.killed || root.queued {
        return Ok(Progress::Done);
    }
    if !root.reaped {
        return Ok(Progress::Pending);
    }
    let Some(face) = endpoints.face(Grant::Ruin) else {
        return if env::chrono::clock() >= root.deadline {
            Err(Fault::Room)
        } else {
            Ok(Progress::Pending)
        };
    };
    if matches!(
        Hole::from_raw(face).wait(HoleCondition::Pull, Wait::AtMost(1)),
        Ok(true)
    ) {
        root.queued = true;
        crate::debug::put(&alloc::format!("{}: actor request queued", stop.root));
        return Ok(Progress::Done);
    }
    if env::chrono::clock() >= root.deadline {
        Err(Fault::Room)
    } else {
        Ok(Progress::Pending)
    }
}

pub(super) fn acknowledge(
    mut root: ResMut<Root>,
    control: Res<Control>,
    stop: Res<Stop>,
) -> Result<Progress, Fault> {
    if root.ack_sent {
        return Ok(Progress::Done);
    }
    if !root.queued {
        return Ok(Progress::Done);
    }
    if stop.state.failed.load(Ordering::Acquire) {
        return Err(Fault::Room);
    }
    let ready = stop.state.ruin.load(Ordering::Acquire)
        && stop.state.dead.load(Ordering::Acquire)
        && control.run_requested(stop.target) == Some(false)
        && control.task(stop.target).is_none()
        && matches!(control.state(stop.target.into()), Ok(InternalState::Dead));
    if !ready {
        return if env::chrono::clock() >= root.deadline {
            Err(Fault::Room)
        } else {
            Ok(Progress::Done)
        };
    }
    let ack = root.ack.ok_or(Fault::Room)?;
    Hole::from_raw(ack)
        .push(&[1], Wait::AtMost(100))
        .map_err(|_| Fault::Room)?;
    root.ack_sent = true;
    crate::debug::put(&alloc::format!(
        "{}: ack controller={} run=false state=dead",
        stop.root,
        root.controller.map_or(0, |task| task.get()),
    ));
    Ok(Progress::Done)
}
