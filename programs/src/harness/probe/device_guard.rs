//! Hold the real DMA binding while the deployment guard is armed, then let the
//! test Root kill the protected domain main task through the real Kernel path.

mod guard;

#[path = "../../driver/uart/dev/uart.rs"]
#[allow(dead_code)]
mod uart;

use ::resource::dock::{Dock, View};
use ::schedule::{
    BuildError, Cursor, Dispatch, Invocation, Plan, Progress, Res, ResMut, RunError, Schedule,
};

use crate::system::app;
use crate::system::control::unit::Control;
use guard::Guard;

/// Private notification from the scene controller after its real block request
/// is submitted and reported.
pub(crate) const REQUEST: env::Mark = env::Mark::of("device-guard/request");

const CONTROLLER: &str = "device-ctl-guard";
const TARGETS: [&str; 3] = ["pci", "iommu", "recovery"];

/// Run the guard death scene from the deployment bootstrap image.
pub fn run() -> Result<(), env::Reason> {
    let boot = app::bootstrap::take().map_err(|why| why.code())?;
    let target = match boot.catalog.deployment() {
        "iommu-guard-pci" => "pci",
        "iommu-guard-iommu" => "iommu",
        "iommu-guard-recovery" => "recovery",
        _ => return Err(9usize),
    };
    crate::debug::put(&alloc::format!(
        "guard: root task={}", env::unit::self_id().get(),
    ));
    let programs = app::scene::programs(&boot.catalog).map_err(|_| 9usize)?;
    let uart = open_uart(&boot)?;
    let mut resources = app::install::resources(boot).map_err(|_| 9usize)?;
    resources
        .write::<crate::system::control::Startup>()
        .map_err(|_| 9usize)?
        .configure(programs);
    resources
        .insert(Guard {
            uart,
            target,
            submitted: false,
            armed: None,
            ready: false,
        })
        .map_err(|_| 9usize)?;
    resources.insert(Cursor::default()).map_err(|_| 9usize)?;
    resources
        .insert(Dispatch::<u8, app::Fault>::new())
        .map_err(|_| 9usize)?;
    app::supervise(resources, frame().map_err(|_| 9usize)?)
}

fn open_uart(boot: &app::bootstrap::Boot) -> Result<View, env::Reason> {
    let devices = boot.machine.devices().ok_or(9usize)?;
    let device = devices
        .iter()
        .find(|device| device.class == "ns16550a")
        .ok_or(9usize)?;
    let token = boot.accounts.token(device.resource).ok_or(9usize)?;
    Ok(Dock::open(token).map_err(|_| 9usize)?.view())
}

/// One root tick: capture the controller receipt, advance the production frame
/// once, then poll the real UART. Returning `Done` resets the root cursor while
/// the app-frame cursor survives as its own Cursor resource.
fn frame() -> Result<Plan<app::Fault>, BuildError> {
    let mut root = Schedule::sequence();
    root.system("guard.prepare", prepare)?;
    root.system("guard.budget", budget)?;
    root.subplans(
        "guard.system",
        select,
        alloc::vec![(0u8, app::schedule::frame()?)],
        finish,
    )?;
    root.system("guard.trigger", trigger)?;
    root.build()
}

fn prepare(mut guard: ResMut<Guard>, control: Res<Control>) -> Result<Progress, app::Fault> {
    observe(&control, &mut guard)?;
    capture(&control, &mut guard)?;
    Ok(Progress::Done)
}

fn budget(mut dispatch: ResMut<Dispatch<u8, app::Fault>>) -> Result<Progress, app::Fault> {
    dispatch.begin(1).map_err(|_| app::Fault::Room)?;
    Ok(Progress::Done)
}

fn select(
    mut turns: ResMut<Cursor>,
    mut dispatch: ResMut<Dispatch<u8, app::Fault>>,
) -> Result<Progress, app::Fault> {
    dispatch
        .select(Invocation {
            key: 0,
            cursor: core::mem::take(&mut *turns),
        })
        .map_err(|_| app::Fault::Room)?;
    Ok(Progress::Done)
}

fn finish(
    mut turns: ResMut<Cursor>,
    mut dispatch: ResMut<Dispatch<u8, app::Fault>>,
) -> Result<Progress, app::Fault> {
    let completion = dispatch.take_result().map_err(|_| app::Fault::Room)?;
    let mut cursor = completion.invocation.cursor;
    match completion.result {
        Ok(Progress::Done) => {
            cursor.reset();
        }
        Ok(Progress::Pending) => {}
        Err(RunError::Step(fault)) => return Err(fault),
        Err(_) => return Err(app::Fault::Room),
    }
    *turns = cursor;
    Ok(Progress::Done)
}

fn trigger(mut guard: ResMut<Guard>, control: Res<Control>) -> Result<Progress, app::Fault> {
    observe(&control, &mut guard)?;
    capture(&control, &mut guard)?;
    let name = guard.target;
    if let Some((task, team)) = guard.armed {
        if guard.submitted && !guard.ready {
            if !TARGETS.iter().all(|name| control.guard_held(name)) {
                return Err(app::Fault::Room);
            }
            crate::debug::put("guard: critical domains protected=3");
            crate::debug::put(&alloc::format!(
                "guard: ready target={name} task={} team={}",
                task.get(),
                team.get(),
            ));
            guard.ready = true;
        }
    }
    let mut bytes = [0u8; 64];
    let len = uart::drain(guard.uart, &mut bytes);
    if !bytes[..len].contains(&b'G') {
        let _ = execution::room::park(core::time::Duration::from_millis(1));
        return Ok(Progress::Done);
    }
    let Some((task, team)) = guard.armed else {
        return Err(app::Fault::Room);
    };
    if !guard.submitted || !guard.ready {
        return Err(app::Fault::Room);
    }
    if target(&control, name)? != Some((task, team))
        || !TARGETS.iter().all(|name| control.guard_held(name))
        || !control.guard_held(name)
        || control.task(name) != Some(task)
        || control.task_team(task) != Some(team)
        || !control.live(task)
    {
        return Err(app::Fault::Room);
    }
    crate::debug::put(&alloc::format!(
        "guard: trigger target={name} task={} team={}",
        task.get(),
        team.get(),
    ));
    env::unit::slay_task(task).map_err(|_| app::Fault::Room)?;
    crate::debug::put("guard: slay returned; waiting for the protected stop");
    loop {
        let _ = execution::room::park(core::time::Duration::from_millis(100));
    }
}

fn capture(control: &Control, guard: &mut Guard) -> Result<(), app::Fault> {
    if guard.submitted {
        return Ok(());
    }
    let Some(controller) = control.task(CONTROLLER) else {
        return Ok(());
    };
    let Ok(request) = ipc::session::establish::find(controller, REQUEST) else {
        return Ok(());
    };
    let mut bytes = [0u8; 1];
    match resource::raw::Hole::from_raw(request).pull(&mut bytes, env::Wait::POLL) {
        Ok((1, from)) if from == controller && bytes == [1] => {
            guard.submitted = true;
            crate::debug::put(&alloc::format!(
                "guard: submitted controller={}",
                controller.get(),
            ));
            Ok(())
        }
        Ok(_) => Err(app::Fault::Room),
        Err(_) => Ok(()),
    }
}

fn target(
    control: &Control,
    name: &str,
) -> Result<Option<(env::TaskId, env::TeamId)>, app::Fault> {
    if !control.guard_held(name) {
        return Ok(None);
    }
    let task = control.task(name).ok_or(app::Fault::Room)?;
    let team = control.task_team(task).ok_or(app::Fault::Room)?;
    Ok(Some((task, team)))
}

fn observe(control: &Control, guard: &mut Guard) -> Result<(), app::Fault> {
    let Some(identity) = target(control, guard.target)? else {
        return Ok(());
    };
    if let Some(prior) = guard.armed {
        if prior != identity {
            return Err(app::Fault::Room);
        }
    } else {
        let (task, team) = identity;
        crate::debug::put(&alloc::format!(
            "guard: armed target={} task={} team={}",
            guard.target, task.get(), team.get(),
        ));
        guard.armed = Some(identity);
    }
    Ok(())
}
