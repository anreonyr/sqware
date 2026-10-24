mod injection;
mod state;

use schedule::{Progress, Res, ResMut, Schedule};

use crate::system::{app, control::unit::Control};
use state::State;

pub(crate) const REQUEST: env::Mark = env::Mark::of("device-confirmation/request");
pub(crate) const ACK: env::Mark = env::Mark::of("device-confirmation/ack");

pub fn run() -> Result<(), env::Reason> {
    let boot = app::bootstrap::take().map_err(|why| why.code())?;
    let primitive = boot.catalog.deployment() == "iommu-fail-reference";
    let before_stop = boot.catalog.deployment() == "iommu-fail-reset";
    let (party, op, target) = match boot.catalog.deployment() {
        "iommu-fail-invalidate" => (
            hub_api::PARTY_IOMMU,
            crate::driver::iommu::api::INVALIDATE,
            "iommu",
        ),
        "iommu-fail-reset" => (
            hub_api::PARTY_HUB,
            hub_api::coordination::RELEASE,
            "recovery",
        ),
        "iommu-fail-release" | "iommu-fail-reference" => (
            hub_api::PARTY_IOMMU,
            crate::driver::iommu::api::RELEASE,
            "iommu",
        ),
        _ => return Err(9),
    };
    let programs = app::scene::programs(&boot.catalog).map_err(|_| 9usize)?;
    let mut resources = app::install::resources(boot).map_err(|_| 9usize)?;
    resources
        .write::<crate::system::control::Startup>()
        .map_err(|_| 9usize)?
        .configure(programs);
    resources
        .insert(State {
            party,
            op,
            target,
            primitive,
            before_stop,
            injection: env::hole::unseal(
                env::Mark::of("confirmation/injection"),
                env::HoleLimits::default(),
            )
            .map_err(|_| 9usize)?,
            ack: None,
            phase: 0,
        })
        .map_err(|_| 9usize)?;
    let mut frame = Schedule::sequence();
    frame.system("confirmation", prepare).map_err(|_| 9usize)?;
    frame
        .plan("system", app::schedule::frame().map_err(|_| 9usize)?)
        .map_err(|_| 9usize)?;
    app::supervise(resources, frame.build().map_err(|_| 9usize)?)
}

fn prepare(mut state: ResMut<State>, control: Res<Control>) -> Result<Progress, app::Fault> {
    if state.phase == 2 {
        return Ok(Progress::Done);
    }
    let Some(controller) = control.task("device-ctl-confirmation") else {
        return Ok(Progress::Done);
    };
    let Ok(request) = ipc::session::establish::find(controller, REQUEST) else {
        return Ok(Progress::Done);
    };
    let mut message = [0u8; 1];
    let Ok((len, from)) =
        resource::raw::Hole::from_raw(request).pull(&mut message, env::Wait::POLL)
    else {
        return Ok(Progress::Done);
    };
    if (len, from, message[0]) != (1, controller, state.phase + 1) {
        return Err(app::Fault::Room);
    }
    let hub = control.task("hub").ok_or(app::Fault::Room)?;
    let coordinator = control.task("recovery").ok_or(app::Fault::Room)?;
    let target = control.task(state.target).ok_or(app::Fault::Room)?;
    let retainer = control.task("iommu").ok_or(app::Fault::Room)?;
    let bytes = injection::command(
        state.phase == 0,
        if state.before_stop { hub.get() } else { coordinator.get() },
        target.get(),
        if state.before_stop {
            injection::BEFORE_DEVICE_STOP
        } else if state.primitive {
            injection::REFERENCE_RELEASE
        } else {
            u64::from(state.op)
        },
        1,
        retainer.get(),
        if state.before_stop { control.task("pci").ok_or(app::Fault::Room)?.get() } else { 0 },
    );
    let injection = resource::raw::Hole::from_raw(state.injection);
    injection
        .push(&bytes, env::Wait::POLL)
        .map_err(|_| app::Fault::Room)?;
    let mut echo = [0u8; injection::LEN];
    let reply = injection
        .pull(&mut echo, env::Wait::POLL)
        .map_err(|_| app::Fault::Room)?;
    if reply != (bytes.len(), env::unit::self_id()) || echo != bytes {
        return Err(app::Fault::Room);
    }
    state.phase += 1;
    let ack = match state.ack {
        Some(ack) => ack,
        None => {
            let ack =
                ipc::session::establish::give(controller, ACK).map_err(|_| app::Fault::Room)?;
            state.ack = Some(ack);
            ack
        }
    };
    resource::raw::Hole::from_raw(ack)
        .push(
            &[
                state.party,
                state.op,
                if state.primitive {
                    crate::driver::iommu::api::fail_to_code(Some(
                        crate::driver::iommu::api::Fail::Unavailable,
                    ))
                } else {
                    0
                },
            ],
            env::Wait::POLL,
        )
        .map_err(|_| app::Fault::Room)?;
    Ok(Progress::Done)
}
