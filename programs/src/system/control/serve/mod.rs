pub mod answer;
pub mod material;
pub mod publication;
pub mod reap;
pub mod resource;
pub mod start;
pub mod task;
pub mod unit;
pub mod watch;

use crate::service::hub::bridge::Activation;
use crate::system::control::core::{publication::Publications, verdict as core};
use crate::system::identity::serve::{install::Roster, names::Names};
use crate::system::operator::serve::install::Tree;
use alloc::vec::Vec;
use env::Wait;
use material::Supplies;
use protocol::{debug, system::control as ccall};
use resource::Resources;
use runtime::env::{chrono::clock, unit as utask};
use start::Images;
use unit::Control;
use watch::Watch;

#[derive(Debug)]
pub enum Fail {
    Room,
    Publication,
    Dead,
    Wait,
    Idle,
    Shutdown,
}
pub fn run(
    watch: &mut Watch,
    control: &mut Control,
    roster: &Roster,
    supplies: &mut Supplies,
    activation: &mut Option<Activation>,
    images: &Images,
    publications: &mut Publications,
    resources: &mut Resources,
    names: &mut Names,
    tree: &mut Tree,
    initial: &[&'static crate::unit::UnitFile],
) -> Result<(), Fail> {
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(runtime::PAGE_SIZE).is_err() {
        debug!("system: no room");
        return Err(Fail::Room);
    }
    buf.resize(runtime::PAGE_SIZE, 0);
    let mut operations = lifecycle::Operations::new();
    let mut plans = crate::system::run::schedule::lifecycle().map_err(|_| Fail::Room)?;
    let mut boot_at = 0;
    if let Some(program) = initial.first() {
        operations.push(lifecycle::Request { name: program.name().into(), action: lifecycle::Action::Mint, back: None }).map_err(|_| Fail::Room)?;
    }
    let mut settling = false;
    let mut forced = false;
    let mut quiet_at = clock();
    let mut owed = control.table.living().count();
    loop {
        if let Err(why) = crate::system::run::cycle::poll(
            control,
            roster,
            &supplies.machine,
            activation,
            images.entry,
            publications,
            resources,
            names,
            tree,
        ) {
            debug::put(&alloc::format!("system: {why}"));
            return Err(Fail::Publication);
        }
        for task in [
            control.task("operator").unwrap(),
            control.task("identity").unwrap(),
        ] {
            if utask::join(task, Wait::POLL).unwrap_or(true) {
                debug::put("system: internal task ended; terminating team");
                return Err(Fail::Dead);
            }
        }
        reap::sweep(control, roster, &operations);
        for i in 0..ccall::Grant::ALL.len() {
            let Some(face) = watch.faces[i] else {
                continue;
            };
            answer::serve_face(control, &mut operations, ccall::Grant::ALL[i], face, &mut buf);
        }
        driver::poll(&mut plans, &mut operations, control, roster, supplies, activation, images)?;
        // Retire names and publications before acknowledging a completed action.
        crate::system::run::cycle::poll(control, roster, &supplies.machine, activation,
            images.entry, publications, resources, names, tree).map_err(|_| Fail::Publication)?;
        let finished = operations.0.iter().find(|job| job.complete && job.operation.request.back.is_none() && !settling)
            .map(|job| (job.operation.request.action, job.operation.failure));
        driver::reply(&mut operations);
        if boot_at < initial.len() {
            if let Some((action, failure)) = finished {
                if failure.is_some() { return Err(Fail::Shutdown); }
                let next = match action {
                    lifecycle::Action::Mint => Some(lifecycle::Action::Embark { parent: None }),
                    lifecycle::Action::Embark { .. } => { boot_at += 1; Some(lifecycle::Action::Mint) },
                    _ => None,
                };
                if let (Some(action), Some(program)) = (next, initial.get(boot_at)) {
                    operations.push(lifecycle::Request { name: program.name().into(), action, back: None }).map_err(|_| Fail::Room)?;
                }
            }
        }

        let armed = watch.sync(control, images.entry, names, activation);
        let living = control.table.living().count();
        if living < owed {
            owed = living;
            quiet_at = clock();
        }
        if !settling && boot_at == initial.len() && operations.0.is_empty() && control.due() {
            driver::ruin_rest(control, &mut operations).map_err(|_| Fail::Room)?;
            settling = true;
            quiet_at = clock();
        }
        if settling && control.done() && operations.0.is_empty() {
            return if forced { Err(Fail::Idle) } else { Ok(()) };
        }
        if clock() - quiet_at >= IDLE_NS {
            if !settling && core::walking(&control.table) {
                debug::put(&alloc::format!(
                    "system: idle {}ms with walkers alive; forcing shutdown",
                    IDLE_MS
                ));
                forced = true;
                driver::ruin_rest(control, &mut operations).map_err(|_| Fail::Room)?;
                settling = true;
                quiet_at = clock();
            } else if settling {
                debug::put(&alloc::format!(
                    "system: idle {}ms while settling; {} still alive",
                    IDLE_MS,
                    owed
                ));
                return Err(Fail::Idle);
            } else {
                quiet_at = clock();
            }
        }
        if !armed {
            watch
                .pile
                .await_(Wait::AtMost(RETRY_MS))
                .map_err(|_| Fail::Wait)?;
            continue;
        }
        let wait = if !operations.0.is_empty() { Wait::AtMost(1) } else { bound(settling, core::walking(&control.table), quiet_at) };
        watch.pile.await_(wait).map_err(|_| Fail::Wait)?;
    }
}
const RETRY_MS: usize = 10;

const IDLE_MS: usize = 10_000;

const IDLE_NS: u64 = IDLE_MS as u64 * 1_000_000;

fn bound(settling: bool, walking: bool, quiet_at: u64) -> Wait {
    if !settling && !walking {
        return Wait::Forever;
    }
    let left = IDLE_NS.saturating_sub(clock() - quiet_at);
    Wait::AtMost(left.div_ceil(1_000_000).max(1) as usize)
}

pub mod lifecycle;

pub(crate) mod driver;
