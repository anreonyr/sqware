//! Shared Root harness for an ordinary-daemon recovery scene.
//!
//! Immutable scene data selects the observed target, controller and recovery or
//! stop handshake around the production application frame.

mod mode;
mod request;
mod root;
mod scene;
mod stop;
mod stop_state;

pub(crate) use mode::Mode;
pub(crate) use request::Request;
pub(crate) use scene::Scene;

use ::resource::raw::Hole;
use ::schedule::{
    BuildError, Cursor, Dispatch, Invocation, Plan, Progress, Res, ResMut, RunError, Schedule,
};

use crate::system::app;
use crate::system::control::unit::Control;
use root::Root;

pub(crate) fn run(settings: Scene) -> Result<(), env::Reason> {
    let boot = app::bootstrap::take().map_err(|why| why.code())?;
    crate::debug::put(&alloc::format!(
        "{}: root task={}",
        settings.root,
        env::unit::self_id().get(),
    ));
    let deployment = crate::harness::deployment(&boot.catalog).map_err(|_| 9usize)?;
    let declarations = crate::harness::declarations();
    let programs = app::scene::programs(&boot.catalog, declarations, deployment)
        .map_err(|_| 9usize)?;
    let mut resources = app::install::resources(boot, declarations, deployment)
        .map_err(|_| 9usize)?;
    resources
        .write::<crate::system::control::Startup>()
        .map_err(|_| 9usize)?
        .configure(programs);
    let (stop, actor) = if settings.mode == Mode::Stop {
        let (stop, actor) = stop::install(&settings);
        (Some(stop), Some(actor))
    } else {
        (None, None)
    };
    resources.insert(settings).map_err(|_| 9usize)?;
    resources.insert(Root::new()).map_err(|_| 9usize)?;
    if let Some(stop) = stop {
        resources.insert(stop).map_err(|_| 9usize)?;
    }
    resources.insert(Cursor::default()).map_err(|_| 9usize)?;
    resources
        .insert(Dispatch::<u8, app::Fault>::new())
        .map_err(|_| 9usize)?;
    let result = app::supervise(resources, frame(settings.mode).map_err(|_| 9usize)?);
    if result.is_ok() {
        if let Some(actor) = actor {
            actor.join();
        }
    }
    result
}

fn frame(mode: Mode) -> Result<Plan<app::Fault>, BuildError> {
    let mut plan = Schedule::sequence();
    plan.system("daemon-root.prepare", prepare)?;
    if mode == Mode::Stop {
        plan.system("daemon-root.stop.authorize", stop::authorize)?;
        plan.system("daemon-root.stop.deliver", stop::deliver)?;
    }
    plan.system("daemon-root.budget", budget)?;
    plan.subplans(
        "daemon-root.system",
        select,
        alloc::vec![(0u8, app::schedule::frame()?)],
        finish,
    )?;
    plan.system("daemon-root.check", check)?;
    if mode == Mode::Stop {
        plan.system("daemon-root.stop.release", stop::release)?;
        plan.system("daemon-root.stop.reap", stop::reap)?;
        plan.system("daemon-root.stop.queue", stop::queue)?;
        plan.system("daemon-root.stop.acknowledge", stop::acknowledge)?;
    }
    plan.build()
}

fn prepare(
    settings: Res<Scene>,
    mut root: ResMut<Root>,
    control: Res<Control>,
) -> Result<Progress, app::Fault> {
    if root.controller.is_none() {
        root.controller = control.task(settings.controller);
    }
    if root.old.is_none() {
        let Some(task) = control.task(settings.target) else {
            return Ok(Progress::Done);
        };
        let team = control.task_team(task).ok_or(app::Fault::Room)?;
        if !control.live(task) {
            return Err(app::Fault::Room);
        }
        root.old = Some((task, team));
        crate::debug::put(&alloc::format!(
            "{}: old task={} team={}",
            settings.root,
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

fn check(
    settings: Res<Scene>,
    mut root: ResMut<Root>,
    control: Res<Control>,
) -> Result<Progress, app::Fault> {
    capture(&settings, &mut root, &control)?;
    observe_changes(&settings, &mut root, &control)?;
    Ok(Progress::Done)
}

fn capture(settings: &Scene, root: &mut Root, control: &Control) -> Result<(), app::Fault> {
    if root.killed {
        return Ok(());
    }
    let Some(controller) = root.controller else {
        return Ok(());
    };
    let Ok(channel) = ipc::session::establish::find(controller, settings.request) else {
        return Ok(());
    };
    let mut bytes = [0u8; Request::LEN];
    match Hole::from_raw(channel).pull(&mut bytes, env::Wait::POLL) {
        Ok((len, from)) if from == controller => {
            let request = Request::decode(&bytes[..len]).ok_or(app::Fault::Room)?;
            let (old_task, old_team) = root.old.ok_or(app::Fault::Room)?;
            let ack = if settings.mode == Mode::Stop {
                let info = resource::raw::inspect(request.ack).map_err(|_| app::Fault::Room)?;
                if !info.alive
                    || info.owner != controller
                    || info.vestor != controller
                    || info.kind != env::PieKind::Hole
                    || Some(info.mark) != settings.ack
                    || !info.permission.contains(env::Permission::STORE)
                {
                    return Err(app::Fault::Room);
                }
                Some(request.ack)
            } else {
                if request.ack != env::PieToken::NONE {
                    return Err(app::Fault::Room);
                }
                None
            };
            let helper = if settings.helper {
                request.old_helper != env::TaskId::new(0) && request.old_helper != old_task
            } else {
                request.old_helper == env::TaskId::new(0)
            };
            if request.old_task != old_task
                || request.before == 0
                || !helper
                || control.task(settings.target) != Some(old_task)
                || control.task_team(old_task) != Some(old_team)
                || !control.live(old_task)
            {
                return Err(app::Fault::Room);
            }
            if settings.mode == Mode::Stop {
                crate::debug::put(&alloc::format!(
                    "{}: request controller={} old task={} helper={} before={} ack={}",
                    settings.root,
                    controller.get(),
                    old_task.get(),
                    request.old_helper.get(),
                    request.before,
                    request.ack.get(),
                ));
            } else if settings.helper {
                crate::debug::put(&alloc::format!(
                    "{}: request controller={} old task={} helper={} before={}",
                    settings.root,
                    controller.get(),
                    old_task.get(),
                    request.old_helper.get(),
                    request.before,
                ));
            } else {
                crate::debug::put(&alloc::format!(
                    "{}: request controller={} old task={} before={}",
                    settings.root,
                    controller.get(),
                    old_task.get(),
                    request.before,
                ));
            }
            env::unit::slay_task(old_task).map_err(|_| app::Fault::Room)?;
            root.ack = ack;
            root.deadline = env::chrono::clock().saturating_add(10_000_000_000);
            root.reaped = false;
            root.queued = false;
            root.ack_sent = false;
            root.killed = true;
            crate::debug::put(&alloc::format!(
                "{}: kill old task={} team={} before={}",
                settings.root,
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

fn observe_changes(
    settings: &Scene,
    root: &mut Root,
    control: &Control,
) -> Result<(), app::Fault> {
    if !root.killed {
        return Ok(());
    }
    let Some((old_task, old_team)) = root.old else {
        return Ok(());
    };
    let current = control
        .task(settings.target)
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
                    "{}: retired old task={} team={}",
                    settings.root,
                    old_task.get(),
                    old_team.get(),
                ));
            }
            if root.new.is_none() {
                root.new = Some((task, team));
                crate::debug::put(&alloc::format!(
                    "{}: new task={} team={}",
                    settings.root,
                    task.get(),
                    team.get(),
                ));
            }
        }
        None if !root.retired => {
            if control.task_team(old_task).is_none() {
                root.retired = true;
                crate::debug::put(&alloc::format!(
                    "{}: retired old task={} team={}",
                    settings.root,
                    old_task.get(),
                    old_team.get(),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}
