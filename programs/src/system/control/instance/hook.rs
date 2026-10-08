//! Instance lifecycle extension points executed as bounded schedule subplans.
use crate::system::control::core::unit::State;
use crate::system::control::{Fail, unit::Control};
use ::schedule::{BuildError, Dispatch, Invocation, Plan, Progress, Res, ResMut, Schedule};
use env::{TaskId, Wait, unit};
#[derive(Clone, Copy, PartialEq)]
pub enum Key {
    Prepare,
    Retire,
}
#[derive(Default)]
pub struct Active {
    pub task: Option<TaskId>,
    next: usize,
}
pub fn plan(
    children: alloc::vec::Vec<(Key, Plan<&'static str>)>,
) -> Result<Plan<Fail>, BuildError> {
    let mut plan = Schedule::sequence();
    plan.system("budget", budget)?;
    plan.subplans("hooks", select, children, finish)?;
    plan.build()
}
pub fn budget(
    control: Res<Control>,
    mut active: ResMut<Active>,
    mut dispatch: ResMut<Dispatch<Key, &'static str>>,
) -> Result<Progress, Fail> {
    active.next = 0;
    dispatch
        .begin(control.instances.len())
        .map_err(|_| Fail::Room)?;
    Ok(Progress::Done)
}
pub fn select(
    mut control: ResMut<Control>,
    mut active: ResMut<Active>,
    mut dispatch: ResMut<Dispatch<Key, &'static str>>,
) -> Result<Progress, Fail> {
    while let Some(item) = control.instances.get_mut(active.next) {
        active.next += 1;
        let key = match item.state {
            State::Starting => Key::Prepare,
            State::Stopping if unit::join(item.task, Wait::POLL).unwrap_or(true) => Key::Retire,
            _ => continue,
        };
        active.task = Some(item.task);
        dispatch
            .select(Invocation {
                key,
                cursor: core::mem::take(&mut item.hook),
            })
            .map_err(|_| Fail::Room)?;
        return Ok(Progress::Done);
    }
    dispatch.stop().map_err(|_| Fail::Room)?;
    Ok(Progress::Done)
}
pub fn finish(
    mut control: ResMut<Control>,
    active: Res<Active>,
    mut dispatch: ResMut<Dispatch<Key, &'static str>>,
) -> Result<Progress, Fail> {
    let completion = dispatch.take_result().map_err(|_| Fail::Room)?;
    let invocation = completion.invocation;
    let item = control
        .instances
        .iter_mut()
        .find(|item| Some(item.task) == active.task)
        .ok_or(Fail::Room)?;
    match completion.result {
        Ok(Progress::Pending) => item.hook = invocation.cursor,
        Ok(Progress::Done) => {
            item.hook.reset();
            match invocation.key {
                Key::Prepare => item.state = State::Debarked,
                Key::Retire => {
                    item.team = None;
                    item.state = State::Dead;
                }
            }
        }
        Err(error) => {
            programs::debug::put(&alloc::format!(
                "control: instance {} hook failed {:?}",
                item.task.get(),
                error
            ));
            item.stop();
            item.hook.reset();
        }
    }
    Ok(Progress::Done)
}
pub fn reclaim(control: Res<Control>, active: Res<Active>) -> Result<Progress, &'static str> {
    let item = control
        .instances
        .iter()
        .find(|item| Some(item.task) == active.task)
        .ok_or("instance hook target")?;
    let Some(team) = item.team else {
        return Ok(Progress::Done);
    };
    match unit::oust(team) {
        Ok(()) => Ok(Progress::Done),
        Err(error) if error.source == env::UnitFail::Busy => Ok(Progress::Pending),
        Err(error) if error.source == env::UnitFail::Denied => Ok(Progress::Done),
        Err(_) => Err("instance reclaim"),
    }
}
