use core::time::Duration;

use ::resource::raw::Hole;
use env::{PieToken, TaskId, Wait};
use ipc::session::{Session, establish};
use system_api::control::{self as ctl, Fail as ControlFail, State};
use system_client::control::{Face, Observe};
use system_client::operator;

use crate::harness::probe::daemon_root::Request;
use crate::harness::probe::daemon_target::api;
use crate::harness::probe::daemon_target::ask::Ask;
use crate::harness::probe::daemon_target::beat::Beat;

const MS: usize = 3000;
const ACK_MS: usize = 10_000;
const TOTAL_NS: u64 = 20_000_000_000;
const STABLE_NS: u64 = 600_000_000;
const STEP_MS: u64 = 50;

pub fn run() -> Result<(), &'static str> {
    let session = Session::open(env::unit::sire(), operator::BERTH, Wait::AtMost(MS))
        .map_err(|_| "daemon-stop: operator session")?;
    let tree = operator::Face::from(&session);
    let until = env::chrono::clock().saturating_add(TOTAL_NS);
    outsider(&tree, env::unit::self_id())?;
    let (old_task, old_helper, before) = observe_old(&tree, until)?;
    crate::debug::put(&alloc::format!(
        "daemon-ctl-stop: observed old task={} helper={} before={}",
        old_task.get(),
        old_helper.get(),
        before,
    ));
    let ack =
        establish::lend(env::unit::sire(), super::ACK).map_err(|_| "daemon-stop: ack hole")?;
    request((old_task, old_helper, before), ack.seed())?;
    let mut byte = [0u8; 1];
    let (len, from) = Hole::from_raw(ack.rx())
        .pull(&mut byte, Wait::AtMost(ACK_MS))
        .map_err(|_| "daemon-stop: ack receive")?;
    if (len, from, byte) != (1, env::unit::sire(), [1]) {
        return Err("daemon-stop: ack source");
    }
    crate::debug::put(&alloc::format!(
        "daemon-ctl-stop: ack from={} old task={} helper={}",
        from.get(),
        old_task.get(),
        old_helper.get(),
    ));
    stable(&tree, old_task)?;
    Ok(())
}

fn outsider(tree: &operator::Face, source: TaskId) -> Result<(), &'static str> {
    let wait = Wait::AtMost(MS);
    let service = tree
        .resolve(system_api::namespace::SERVICE, wait)
        .map_err(|_| "daemon-stop: service namespace")?;
    let observe = tree
        .tile_from(service, ctl::SUPERVISION, wait)
        .and_then(|tile| tile.token(wait))
        .map_err(|_| "daemon-stop: observe face")?;
    let recover = tree
        .tile_from(service, ctl::RECOVER, wait)
        .and_then(|tile| tile.token(wait))
        .map_err(|_| "daemon-stop: recover face")?;
    let observe = Observe::of(observe).map_err(|_| "daemon-stop: observe client")?;
    let recover = Face::of(recover).map_err(|_| "daemon-stop: recover client")?;
    if observe.host() != env::unit::sire() || recover.host() != env::unit::sire() {
        return Err("daemon-stop: outsider face host");
    }
    if !observe_denied(&observe)? {
        return Err("daemon-stop: outsider observe not denied");
    }
    if !matches!(
        recover.recover("fixturetarget".into(), wait),
        Err(ControlFail::Denied)
    ) {
        return Err("daemon-stop: outsider recover not denied");
    }
    crate::debug::put(&alloc::format!(
        "daemon-ctl-stop: outsider source={} observe={} recover={} denied=Denied",
        source.get(),
        observe.host().get(),
        recover.host().get(),
    ));
    Ok(())
}

fn observe_denied(face: &Observe) -> Result<bool, &'static str> {
    let until = env::chrono::clock().saturating_add(MS as u64 * 1_000_000);
    let mut pending = face
        .begin_observe(0, Wait::AtMost(MS))
        .map_err(|_| "daemon-stop: observe begin")?;
    loop {
        match face.poll_observe(&mut pending) {
            Ok(Some(_)) => return Ok(false),
            Err(ControlFail::Denied) => return Ok(true),
            Ok(None) if env::chrono::clock() < until => park(),
            Ok(None) => return Ok(false),
            Err(_) => return Ok(false),
        }
    }
}

fn observe_old(tree: &operator::Face, until: u64) -> Result<(TaskId, TaskId, u64), &'static str> {
    while env::chrono::clock() < until {
        if let Some(entry) = lookup(tree, Wait::AtMost(MS))? {
            if let Some(task) = establish::opened_by(entry) {
                if let Ok(beat) = heartbeat(entry, task) {
                    if beat.now > 0 {
                        return Ok((task, beat.helper, beat.now));
                    }
                }
            }
        }
        park();
    }
    Err("daemon-stop: old heartbeat")
}

fn lookup(tree: &operator::Face, wait: Wait) -> Result<Option<PieToken>, &'static str> {
    let service = tree
        .resolve(system_api::namespace::SERVICE, wait)
        .map_err(|_| "daemon-stop: lookup namespace")?;
    match tree.tile_from(service, api::ENTRY_PATH, wait) {
        Ok(tile) => match tile.token(wait) {
            Ok(entry) => Ok(Some(entry)),
            Err(system_api::operator::Fail::Unknown) => Ok(None),
            Err(_) => Err("daemon-stop: lookup token"),
        },
        Err(system_api::operator::Fail::Unknown) => Ok(None),
        Err(_) => Err("daemon-stop: lookup entry"),
    }
}

fn heartbeat(entry: PieToken, target: TaskId) -> Result<Beat, &'static str> {
    let (back, seed) =
        establish::lend_out(entry, api::BACK).map_err(|()| "daemon-stop: heartbeat back")?;
    let ask = Ask::of(seed, target);
    let mut frame = [0u8; Ask::LEN];
    let Some(n) = ask.encode(&mut frame) else {
        let _ = env::pie::release(back, env::ReleaseMode::Revoke);
        return Err("daemon-stop: heartbeat frame");
    };
    if Hole::from_raw(entry)
        .push(&frame[..n], Wait::AtMost(MS))
        .is_err()
    {
        let _ = env::pie::release(back, env::ReleaseMode::Revoke);
        return Err("daemon-stop: heartbeat send");
    }
    let mut bytes = [0u8; Beat::LEN];
    let answer = Hole::from_raw(back).pull(&mut bytes, Wait::AtMost(MS));
    let _ = env::pie::release(back, env::ReleaseMode::Revoke);
    let (len, from) = answer.map_err(|_| "daemon-stop: heartbeat reply")?;
    if from != target {
        return Err("daemon-stop: heartbeat source");
    }
    let beat = Beat::decode(&bytes[..len]).ok_or("daemon-stop: heartbeat decode")?;
    if beat.op != api::BEAT
        || beat.target != target
        || beat.helper == TaskId::new(0)
        || beat.helper == target
    {
        return Err("daemon-stop: heartbeat content");
    }
    Ok(beat)
}

fn request(
    old: (TaskId, TaskId, u64),
    ack: PieToken,
) -> Result<(), &'static str> {
    let (old_task, old_helper, before) = old;
    let entry = establish::give(env::unit::sire(), super::REQUEST)
        .map_err(|_| "daemon-stop: request channel")?;
    let request = Request {
        old_task,
        old_helper,
        before,
        ack,
    };
    let mut bytes = [0u8; Request::LEN];
    let len = request
        .encode(&mut bytes)
        .ok_or("daemon-stop: request frame")?;
    Hole::from_raw(entry)
        .push(&bytes[..len], Wait::AtMost(MS))
        .map_err(|_| "daemon-stop: request send")
}

fn stable(tree: &operator::Face, old_task: TaskId) -> Result<(), &'static str> {
    let wait = Wait::AtMost(MS);
    let service = tree
        .resolve(system_api::namespace::SERVICE, wait)
        .map_err(|_| "daemon-stop: stable namespace")?;
    let road = ctl::DIR
        .try_join(ctl::Grant::State.name())
        .ok_or("daemon-stop: state path")?;
    let entry = tree
        .tile_from(service, &road, wait)
        .and_then(|tile| tile.token(wait))
        .map_err(|_| "daemon-stop: stable state face")?;
    let face = Face::of(entry).map_err(|_| "daemon-stop: stable state client")?;
    if face.host() != env::unit::sire() {
        return Err("daemon-stop: state face host");
    }
    let start = env::chrono::clock();
    let until = start.saturating_add(STABLE_NS);
    let mut checks = 0usize;
    let mut missing = 0usize;
    while env::chrono::clock() < until {
        if !matches!(
            face.service("fixturetarget".into()).state(wait),
            Ok(State::Dead)
        ) {
            return Err("daemon-stop: target resurrected");
        }
        checks += 1;
        match lookup(tree, Wait::AtMost(25))? {
            None => missing += 1,
            Some(entry) if establish::opened_by(entry) == Some(old_task) => {
                return Err("daemon-stop: stale target entry");
            }
            Some(_) => return Err("daemon-stop: replacement target entry"),
        }
        park_for(STEP_MS);
    }
    if checks < 2 || missing != checks {
        return Err("daemon-stop: target state unstable");
    }
    crate::debug::put(&alloc::format!(
        "daemon-ctl-stop: stable dead checks={} missing={} target=fixturetarget elapsed_ns={}",
        checks,
        missing,
        env::chrono::clock().saturating_sub(start),
    ));
    Ok(())
}

fn park() {
    let _ = execution::room::park(Duration::from_millis(1));
}

fn park_for(millis: u64) {
    let _ = execution::room::park(Duration::from_millis(millis));
}
