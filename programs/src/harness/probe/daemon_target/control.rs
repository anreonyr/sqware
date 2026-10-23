use core::time::Duration;

use ::resource::raw::Hole;
use env::{PieToken, TaskId, Wait, pie};
use ipc::session::{Session, establish};
use system_client::operator;

use crate::harness::probe::daemon_root::Request;

use super::api;
use super::ask::Ask;
use super::beat::Beat;

const MS: usize = 3000;
const TOTAL_NS: u64 = 15_000_000_000;

pub fn run() -> Result<(), &'static str> {
    let session = Session::open(env::unit::sire(), operator::BERTH, Wait::AtMost(MS))
        .map_err(|_| "daemon-target: operator session")?;
    let tree = operator::Face::from(&session);
    let until = env::chrono::clock().saturating_add(TOTAL_NS);
    let (old_task, old_helper, before) = observe_old(&tree, until)?;
    crate::debug::put(&alloc::format!(
        "daemon-target: observed old task={} helper={} before={}",
        old_task.get(),
        old_helper.get(),
        before,
    ));
    request(old_task, old_helper, before)?;
    let (new_task, new_helper, after) =
        observe_new(&tree, (old_task, old_helper, before), until)?;
    crate::debug::put(&alloc::format!(
        "daemon-target: proof old task={} helper={} before={} new task={} helper={} after={}",
        old_task.get(),
        old_helper.get(),
        before,
        new_task.get(),
        new_helper.get(),
        after,
    ));
    Ok(())
}

fn observe_old(tree: &operator::Face, until: u64) -> Result<(TaskId, TaskId, u64), &'static str> {
    while env::chrono::clock() < until {
        if let Some(entry) = lookup(tree) {
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
    Err("daemon-target: old heartbeat")
}

fn observe_new(
    tree: &operator::Face,
    old: (TaskId, TaskId, u64),
    until: u64,
) -> Result<(TaskId, TaskId, u64), &'static str> {
    let (old_task, old_helper, before) = old;
    while env::chrono::clock() < until {
        if let Some(entry) = lookup(tree) {
            if let Some(task) = establish::opened_by(entry) {
                if task != old_task {
                    if let Ok(beat) = heartbeat(entry, task) {
                        if beat.helper != old_helper && beat.now >= before {
                            return Ok((task, beat.helper, beat.now));
                        }
                    }
                }
            }
        }
        park();
    }
    Err("daemon-target: fresh heartbeat")
}

fn lookup(tree: &operator::Face) -> Option<PieToken> {
    let wait = Wait::AtMost(MS);
    let service = tree.resolve(system_api::namespace::SERVICE, wait).ok()?;
    tree.tile_from(service, api::ENTRY_PATH, wait)
        .ok()?
        .token(wait)
        .ok()
}

fn heartbeat(entry: PieToken, target: TaskId) -> Result<Beat, &'static str> {
    let (back, seed) =
        establish::lend_out(entry, api::BACK).map_err(|()| "daemon-target: heartbeat back")?;
    let ask = Ask::of(seed, target);
    let mut frame = [0u8; Ask::LEN];
    let Some(n) = ask.encode(&mut frame) else {
        let _ = pie::release(back, env::ReleaseMode::Revoke);
        return Err("daemon-target: heartbeat frame");
    };
    if Hole::from_raw(entry)
        .push(&frame[..n], Wait::AtMost(MS))
        .is_err()
    {
        let _ = pie::release(back, env::ReleaseMode::Revoke);
        return Err("daemon-target: heartbeat send");
    }
    let mut bytes = [0u8; Beat::LEN];
    let answer = Hole::from_raw(back).pull(&mut bytes, Wait::AtMost(MS));
    let _ = pie::release(back, env::ReleaseMode::Revoke);
    let (len, from) = answer.map_err(|_| "daemon-target: heartbeat reply")?;
    if from != target {
        return Err("daemon-target: heartbeat source");
    }
    let beat = Beat::decode(&bytes[..len]).ok_or("daemon-target: heartbeat decode")?;
    if beat.op != api::BEAT
        || beat.target != target
        || beat.helper == TaskId::new(0)
        || beat.helper == target
    {
        return Err("daemon-target: heartbeat content");
    }
    Ok(beat)
}

fn request(old_task: TaskId, old_helper: TaskId, before: u64) -> Result<(), &'static str> {
    let entry = establish::give(env::unit::sire(), super::REQUEST)
        .map_err(|_| "daemon-target: request channel")?;
    let request = Request {
        old_task,
        old_helper,
        before,
    };
    let mut bytes = [0u8; Request::LEN];
    let len = request
        .encode(&mut bytes)
        .ok_or("daemon-target: request frame")?;
    Hole::from_raw(entry)
        .push(&bytes[..len], Wait::AtMost(MS))
        .map_err(|_| "daemon-target: request send")
}

fn park() {
    let _ = execution::room::park(Duration::from_millis(1));
}
