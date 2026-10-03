use crate::system::{control::{core::verdict, serve::{self, unit::Control, lifecycle::{Action, Active, Key, Operations, Request}, resource::Resources}}, identity::serve::install::Roster, run::bootstrap::Boot};
use crate::system::control::serve::schedule;
use crate::unit::{Died, UnitFile};
use protocol::common::schedule::{Resources as Registry, Res, ResMut, Plan, Schedule, Progress, Cursor, Dispatch, Invocation};
pub struct Fault { pub armed: bool, pub task: Option<env::TaskId>, pub road: Option<protocol::common::path::PathBuf> }
pub struct Fixture { pub resources: Registry<'static>, plans: [Plan<serve::Fail>; 4] }
fn supply(active: Res<Active>, roster: Res<Roster>, control: Res<Control>) -> Result<Progress, verdict::Fail> {
    let job = active.0.as_ref().ok_or(verdict::Fail::Unknown)?;
    let program = serve::start::program_of(&job.request.name)?;
    let task = control.task(&job.request.name).ok_or(verdict::Fail::Unknown)?;
    super::identity::supply_to(roster.authority(), program, task).map_err(|_| verdict::Fail::NotReady)?;
    Ok(Progress::Done)
}
fn probe(fault: Res<Fault>, mut dispatch: ResMut<Dispatch<(), verdict::Fail>>) -> Result<Progress, verdict::Fail> {
    dispatch.budget = usize::from(fault.armed); Ok(Progress::Done)
}
fn select_probe(mut dispatch: ResMut<Dispatch<(), verdict::Fail>>) -> Result<Progress, verdict::Fail> {
    dispatch.current = Some(Invocation { key: (), cursor: Default::default() }); Ok(Progress::Done)
}
fn inject(active: Res<Active>, resources: Res<Resources>, mut fault: ResMut<Fault>) -> Result<Progress, verdict::Fail> {
    if fault.armed {
        fault.armed = false;
        fault.task = active.0.as_ref().and_then(|job| job.execution.task);
        fault.road = fault.task.and_then(|task| resources.runtime_road(task));
        return Err(verdict::Fail::NotReady);
    }
    Ok(Progress::Done)
}
impl Fixture {
    pub fn new(boot: Boot) -> Result<Self, ()> {
        let mut resources = crate::system::control::serve::run::resources(boot).map_err(|_| ())?;
        resources.insert(Fault { armed: false, task: None, road: None }).map_err(|_| ())?;
        resources.insert(Dispatch::<(), verdict::Fail>::new()).map_err(|_| ())?;
        let mut start = schedule::startup().map_err(|_| ())?;
        let mut children = schedule::lifecycle().map_err(|_| ())?;
        let at = children.iter().position(|(key, _)| *key == Key::Embark).unwrap();
        let (_, embark) = children.remove(at);
        let mut plan = Schedule::new(); plan.add_system("fixture.supply", 0u8, supply).map_err(|_| ())?;
        plan.add_plan("embark", 1, embark).map_err(|_| ())?;
        plan.add_system("fixture.probe", 2, probe).map_err(|_| ())?;
        plan.add_subplans("fixture.failure", 3, select_probe, alloc::vec![((), schedule::maintenance().map_err(|_| ())?.map_error(|_| verdict::Fail::NotReady))], inject).map_err(|_| ())?;
        children.push((Key::Embark, plan.build().map_err(|_| ())?));
        let plans = [schedule::maintenance().map_err(|_| ())?.map_error(|_| serve::Fail::Publication), schedule::actions(children).map_err(|_| ())?, schedule::frame().map_err(|_| ())?, schedule::shutdown().map_err(|_| ())?];
        start.advance(&mut Cursor::default(), &resources).map_err(|_| ())?;
        Ok(Self { resources, plans })
    }
    pub fn assemble(&mut self, program: &UnitFile) -> Result<(), Died> {
        self.action(program.name(), Action::Mint).map_err(|_| serve::start::E_PROGRAM)?;
        self.action(program.name(), Action::Embark { parent: None }).map_err(|_| serve::start::E_PROGRAM)?;
        Ok(())
    }
    pub fn action(&mut self, name: &str, action: Action) -> Result<Option<env::TaskId>, ()> {
        self.resources.write::<Operations>().map_err(|_| ())?.push(Request { name: name.into(), action, back: None }).map_err(|_| ())?;
        loop {
            self.plans[1].advance(&mut Cursor::default(), &self.resources).map_err(|_| ())?;
            self.progress().map_err(|_| ())?;
            {
                let mut operations = self.resources.write::<Operations>().map_err(|_| ())?;
                if let Some(at) = operations.0.iter().position(|job| job.complete && job.operation.request.name == name) {
                    let job = operations.0.remove(at).ok_or(())?;
                    return if job.operation.failure.is_some() { Err(()) } else { Ok(job.operation.execution.task) };
                }
            }
            runtime::env::room::sleep(core::time::Duration::from_millis(1)).map_err(|_| ())?;
        }
    }
    pub fn progress(&mut self) -> Result<(), &'static str> {
        self.plans[0].advance(&mut Cursor::default(), &self.resources).map_err(|_| "fixture maintenance")?; Ok(())
    }
    pub fn supervise(&mut self) -> Result<(), serve::Fail> {
        let mut cursor = Cursor::default();
        while !self.resources.read::<serve::frame::Flow>().map_err(|_| serve::Fail::Room)?.done {
            if self.plans[2].advance(&mut cursor, &self.resources).map_err(|error| { protocol::debug::put(&alloc::format!("fixture: frame {:?}", error)); serve::Fail::Shutdown })? == Progress::Done { cursor.reset(); }
        }
        let mut cursor = Cursor::default();
        while self.plans[3].advance(&mut cursor, &self.resources).map_err(|_| serve::Fail::Shutdown)? == Progress::Pending {}
        Ok(())
    }
}
