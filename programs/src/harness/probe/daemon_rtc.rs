//! Phase3a Root harness: wrap the production app frame, let the ordinary daemon
//! recover the real RTC target, and kill only the old RTC main once after the
//! transient controller's authenticated private request.

mod control;
mod request;
pub use control::run as controller;
mod root;

use ::resource::raw::Hole;
use ::schedule::{
    BuildError, Cursor, Dispatch, Invocation, Plan, Progress, Res, ResMut, RunError, Schedule,
};

use crate::system::app;
use crate::system::control::unit::Control;
use root::Root;

/// Private notification from the transient controller after its real time reply.
pub(crate) const REQUEST: env::Mark = env::Mark::of("daemon-rtc/request");

const CONTROLLER: &str = "daemon-ctl-rtc";
const RTC: &str = "rtc";

/// Run the actual RTC restart scene from the deployment bootstrap image.
pub fn run() -> Result<(), env::Reason> {
    let boot = app::bootstrap::take().map_err(|why| why.code())?;
    crate::debug::put(&alloc::format!(
        "rtc-root: root task={}", env::unit::self_id().get(),
    ));
    let programs = app::scene::programs(&boot.catalog).map_err(|_| 9usize)?;
    let mut resources = app::install::resources(boot).map_err(|_| 9usize)?;
    resources
        .write::<crate::system::control::Startup>()
        .map_err(|_| 9usize)?
        .configure(programs);
    resources.insert(Root::new()).map_err(|_| 9usize)?;
    resources.insert(Cursor::default()).map_err(|_| 9usize)?;
    resources
        .insert(Dispatch::<u8, app::Fault>::new())
        .map_err(|_| 9usize)?;
    app::supervise(resources, frame().map_err(|_| 9usize)?)
}

fn frame() -> Result<Plan<app::Fault>, BuildError> {
    let mut plan = Schedule::sequence();
    plan.system("rtc-root.prepare", prepare)?;
    plan.system("rtc-root.budget", budget)?;
    plan.subplans(
        "rtc-root.system",
        select,
        alloc::vec![(0u8, app::schedule::frame()?)],
        finish,
    )?;
    plan.system("rtc-root.check", check)?;
    plan.build()
}

fn prepare(mut root: ResMut<Root>, control: Res<Control>) -> Result<Progress, app::Fault> {
    if root.controller.is_none() {
        root.controller = control.task(CONTROLLER);
    }
    if root.old.is_none() {
        let Some(task) = control.task(RTC) else {
            return Ok(Progress::Done);
        };
        let team = control.task_team(task).ok_or(app::Fault::Room)?;
        if !control.live(task) {
            return Err(app::Fault::Room);
        }
        root.old = Some((task, team));
        crate::debug::put(&alloc::format!(
            "rtc-root: old task={} team={}",
            task.get(),
            team.get(),
        ));
    }
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

fn check(mut root: ResMut<Root>, control: Res<Control>) -> Result<Progress, app::Fault> {
    capture(&mut root, &control)?;
    observe_changes(&mut root, &control)?;
    Ok(Progress::Done)
}

fn capture(root: &mut Root, control: &Control) -> Result<(), app::Fault> {
    if root.killed {
        return Ok(());
    }
    let Some(controller) = root.controller else {
        return Ok(());
    };
    let Ok(channel) = ipc::session::establish::find(controller, REQUEST) else {
        return Ok(());
    };
    let mut bytes = [0u8; request::Request::LEN];
    match Hole::from_raw(channel).pull(&mut bytes, env::Wait::POLL) {
        Ok((len, from)) if from == controller => {
            let request = request::Request::decode(&bytes[..len]).ok_or(app::Fault::Room)?;
            let (old_task, old_team) = root.old.ok_or(app::Fault::Room)?;
            if request.old_task != old_task
                || request.before == 0
                || control.task(RTC) != Some(old_task)
                || control.task_team(old_task) != Some(old_team)
                || !control.live(old_task)
            {
                return Err(app::Fault::Room);
            }
            crate::debug::put(&alloc::format!(
                "rtc-root: request controller={} old task={} before={}",
                controller.get(),
                old_task.get(),
                request.before,
            ));
            env::unit::slay_task(old_task).map_err(|_| app::Fault::Room)?;
            root.killed = true;
            crate::debug::put(&alloc::format!(
                "rtc-root: kill old task={} team={} before={}",
                old_task.get(),
                old_team.get(),
                request.before,
            ));
            Ok(())
        }
        Ok(_) => Err(app::Fault::Room),
        Err(_) => Ok(()),
    }
}

fn observe_changes(root: &mut Root, control: &Control) -> Result<(), app::Fault> {
    if !root.killed {
        return Ok(());
    }
    let Some((old_task, old_team)) = root.old else {
        return Ok(());
    };
    let current = control
        .task(RTC)
        .and_then(|task| control.task_team(task).map(|team| (task, team)));
    match current {
        Some((task, team)) if task != old_task => {
            if team == old_team {
                return Err(app::Fault::Room);
            }
            if !root.retired {
                if control.task_team(old_task).is_some() || control.live(old_task) {
                    return Err(app::Fault::Room);
                }
                root.retired = true;
                crate::debug::put(&alloc::format!(
                    "rtc-root: retired old task={} team={}",
                    old_task.get(),
                    old_team.get(),
                ));
            }
            if root.new.is_none() {
                root.new = Some((task, team));
                crate::debug::put(&alloc::format!(
                    "rtc-root: new task={} team={}",
                    task.get(),
                    team.get(),
                ));
            }
        }
        None if !root.retired => {
            if control.task_team(old_task).is_none() {
                root.retired = true;
                crate::debug::put(&alloc::format!(
                    "rtc-root: retired old task={} team={}",
                    old_task.get(),
                    old_team.get(),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}
