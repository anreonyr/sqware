//! Transient ordinary controller: resolve the actual RTC entry, ask the real clock,
//! request one Root kill, then prove the fresh entry belongs to a replacement task.

use core::time::Duration;
use env::{PieToken, Wait};
use ipc::session::{Session, establish};
use system_client::operator;

use super::request::Request;
use crate::driver::rtc::{api, client};

const MS: usize = 3000;
const TOTAL_NS: u64 = 15_000_000_000;

pub fn run() -> Result<(), &'static str> {
    let session = Session::open(env::unit::sire(), operator::BERTH, Wait::AtMost(MS))
        .map_err(|_| "daemon-rtc: operator session")?;
    let tree = operator::Face::from(&session);
    let until = env::chrono::clock().saturating_add(TOTAL_NS);
    let (old_task, before) = observe_old(&tree, until)?;
    crate::debug::put(&alloc::format!(
        "daemon-rtc: observed old task={} before={}",
        old_task.get(),
        before,
    ));
    request(old_task, before)?;
    let (new_task, after) = observe_new(&tree, (old_task, before), until)?;
    crate::debug::put(&alloc::format!(
        "daemon-rtc: proof old task={} before={} new task={} after={}",
        old_task.get(),
        before,
        new_task.get(),
        after,
    ));
    Ok(())
}

fn observe_old(tree: &operator::Face, until: u64) -> Result<(env::TaskId, u64), &'static str> {
    while env::chrono::clock() < until {
        if let Some(entry) = lookup(tree) {
            if let Some(task) = establish::opened_by(entry) {
                if let Ok(before) = client::now(entry, Wait::AtMost(MS)) {
                    return Ok((task, before));
                }
            }
        }
        park();
    }
    Err("daemon-rtc: old RTC time")
}

fn observe_new(
    tree: &operator::Face,
    old: (env::TaskId, u64),
    until: u64,
) -> Result<(env::TaskId, u64), &'static str> {
    let (old_task, before) = old;
    while env::chrono::clock() < until {
        if let Some(entry) = lookup(tree) {
            if let Some(task) = establish::opened_by(entry) {
                if task != old_task {
                    if let Ok(after) = client::now(entry, Wait::AtMost(MS)) {
                        if after >= before {
                            return Ok((task, after));
                        }
                    }
                }
            }
        }
        park();
    }
    Err("daemon-rtc: fresh RTC time")
}

fn lookup(tree: &operator::Face) -> Option<PieToken> {
    let wait = Wait::AtMost(MS);
    let service = tree.resolve(system_api::namespace::SERVICE, wait).ok()?;
    tree.tile_from(service, api::ENTRY_PATH, wait)
        .ok()?
        .token(wait)
        .ok()
}

fn request(old_task: env::TaskId, before: u64) -> Result<(), &'static str> {
    let entry = establish::give(env::unit::sire(), super::REQUEST)
        .map_err(|_| "daemon-rtc: request channel")?;
    let request = Request { old_task, before };
    let mut bytes = [0u8; Request::LEN];
    let len = request
        .encode(&mut bytes)
        .ok_or("daemon-rtc: request frame")?;
    resource::raw::Hole::from_raw(entry)
        .push(&bytes[..len], Wait::AtMost(MS))
        .map_err(|_| "daemon-rtc: request send")
}

fn park() {
    let _ = execution::room::park(Duration::from_millis(1));
}
