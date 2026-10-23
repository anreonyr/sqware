//! Startup carrier owned by Assembly from prepare until retirement.

use super::Jobs;
use env::{Permission, PieToken, TaskId, Wait};
use resource::port::{Access, Policy, To};
use resource::raw::Hole;
use system_api::bootstrap::{Frame, MARK, ROLE, VERSION};
use wire::Message;

pub(crate) struct Carrier {
    hole: Option<PieToken>,
    until: u64,
    receipt: Option<To>,
    pushed: bool,
    restricted: bool,
}

pub(crate) fn advance(jobs: &mut Jobs, system: TaskId) -> Result<schedule::Progress, &'static str> {
    if until(jobs).is_some_and(|until| env::chrono::clock() >= until) {
        return Err("bootstrap timeout");
    }
    match prepare(jobs, system) {
        Ok(()) => Ok(schedule::Progress::Done),
        Err(error) => {
            if until(jobs).is_some_and(|until| env::chrono::clock() < until) {
                Ok(schedule::Progress::Pending)
            } else {
                Err(error)
            }
        }
    }
}

fn until(jobs: &Jobs) -> Option<u64> {
    jobs.active_hook_reference().and_then(|reference| {
        jobs.launches.iter()
            .find(|launch| launch.instance.reference == reference)
            .and_then(|launch| launch.installation.bootstrap.as_ref())
            .filter(|carrier| !carrier.restricted)
            .map(|carrier| carrier.until)
    })
}

pub(crate) fn prepare(jobs: &mut Jobs, system: TaskId) -> Result<(), &'static str> {
    let provider = env::unit::self_id();
    let reference = jobs.active_hook_reference().ok_or("bootstrap reference")?;
    let launch = jobs
        .launches
        .iter_mut()
        .find(|launch| launch.instance.reference == reference)
        .ok_or("bootstrap launch")?;
    let child = launch.instance.task;
    if launch.installation.bootstrap.is_none() {
        launch.installation.bootstrap = Some(Carrier {
            hole: None,
            until: env::chrono::clock()
                .saturating_add(crate::support::timing::BOOT_MS as u64 * 1_000_000),
            receipt: None,
            pushed: false,
            restricted: false,
        });
    }
    let carrier = launch
        .installation
        .bootstrap
        .as_mut()
        .ok_or("bootstrap carrier")?;
    let hole = match carrier.hole {
        Some(hole) => hole,
        None => {
            let hole = env::hole::unseal(MARK, env::HoleLimits::default())
                .map_err(|_| "bootstrap hole")?;
            carrier.hole = Some(hole);
            hole
        }
    };
    if !carrier.pushed {
        let frame = Frame {
            version: VERSION,
            role: ROLE,
            provider,
            system,
        };
        let mut bytes = <Frame as Message>::EMPTY;
        let len = frame.store(&mut bytes).ok_or("bootstrap frame")?;
        Hole::from_raw(hole)
            .push(&bytes[..len], Wait::POLL)
            .map_err(|_| "bootstrap push")?;
        carrier.pushed = true;
    }
    if carrier.receipt.is_none() {
        let receipt = resource::port::ship(hole, child, Access::FETCH, Policy::NONE)
            .map_err(|_| "bootstrap ship")?;
        carrier.receipt = Some(receipt);
    }
    if !carrier.restricted {
        env::pie::narrow(hole, Permission::STORE)
            .map_err(|_| "bootstrap restrict")?;
        carrier.restricted = true;
    }
    Ok(())
}

pub(crate) fn release(jobs: &mut Jobs) -> Result<(), &'static str> {
    let reference = jobs.active_hook_reference().ok_or("bootstrap reference")?;
    let launch = jobs
        .launches
        .iter_mut()
        .find(|launch| launch.instance.reference == reference)
        .ok_or("bootstrap launch")?;
    let Some(carrier) = launch.installation.bootstrap.take() else {
        return Ok(());
    };
    let Some(hole) = carrier.hole else {
        return Ok(());
    };
    match env::pie::release(hole, env::ReleaseMode::Revoke) {
        Ok(()) => Ok(()),
        Err(_) => {
            launch.installation.bootstrap = Some(carrier);
            Err("bootstrap release")
        }
    }
}
