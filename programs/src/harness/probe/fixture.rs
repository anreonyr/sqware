use crate::system::app::Fault as Fail;
use crate::system::app::schedule;
use crate::system::publication::RuntimeNamespace as Resources;
use crate::system::{
    app::bootstrap::Boot,
    control::identity::Roster,
    control::{
        lifecycle::{Action, Active, Key, Operations, schedule as lifecycle},
        unit::verdict,
        unit::{Control, start},
    },
};
use crate::unit::{Died, UnitFile};

use ::schedule::{
    Cursor, Dispatch, Invocation, Plan, Progress, Res, ResMut, Resources as Registry, Schedule,
};
pub struct Fault {
    pub armed: bool,
    pub task: Option<env::TaskId>,
    pub road: Option<system_api::operator::path::PathBuf>,
}
pub(crate) struct Fixture {
    pub resources: Registry<'static>,
    plans: [Plan<Fail>; 4],
    cursors: [Cursor; 4],
}
fn supply(
    active: Res<Active>,
    roster: Res<Roster>,
    control: Res<Control>,
) -> Result<Progress, verdict::Fail> {
    let job = active.0.as_ref().ok_or(verdict::Fail::Unknown)?;
    let program = start::program_of(&job.request.name)?;
    let task = control
        .task(&job.request.name)
        .ok_or(verdict::Fail::Unknown)?;
    super::identity::supply_to(roster.authority(), program, task)
        .map_err(|_| verdict::Fail::NotReady)?;
    Ok(Progress::Done)
}
fn probe(
    fault: Res<Fault>,
    mut dispatch: ResMut<Dispatch<(), verdict::Fail>>,
) -> Result<Progress, verdict::Fail> {
    dispatch
        .begin(usize::from(fault.armed))
        .map_err(|_| verdict::Fail::NotReady)?;
    Ok(Progress::Done)
}
fn select_probe(
    mut dispatch: ResMut<Dispatch<(), verdict::Fail>>,
) -> Result<Progress, verdict::Fail> {
    dispatch
        .select(Invocation {
            key: (),
            cursor: Default::default(),
        })
        .map_err(|_| verdict::Fail::NotReady)?;
    Ok(Progress::Done)
}
fn inject(
    active: Res<Active>,
    resources: Res<Resources>,
    mut fault: ResMut<Fault>,
    mut dispatch: ResMut<Dispatch<(), verdict::Fail>>,
) -> Result<Progress, verdict::Fail> {
    dispatch
        .take_result()
        .map_err(|_| verdict::Fail::NotReady)?;
    if fault.armed {
        fault.armed = false;
        fault.task = active.0.as_ref().and_then(|job| job.execution.task);
        fault.road = fault.task.and_then(|task| resources.runtime_road(task));
        return Err(verdict::Fail::NotReady);
    }
    Ok(Progress::Done)
}
impl Fixture {
    pub(crate) fn new(boot: Boot) -> Result<Self, ()> {
        let mut resources = crate::system::app::install::resources(boot).map_err(|_| ())?;
        resources
            .insert(Fault {
                armed: false,
                task: None,
                road: None,
            })
            .map_err(|_| ())?;
        resources
            .insert(Dispatch::<(), verdict::Fail>::new())
            .map_err(|_| ())?;
        let mut start = schedule::startup().map_err(|_| ())?;
        let mut children =
            lifecycle::lifecycle(crate::system::launch::activation::hooks().map_err(|_| ())?)
                .map_err(|_| ())?;
        let at = children
            .iter()
            .position(|(key, _)| *key == Key::Embark)
            .unwrap();
        let (_, embark) = children.remove(at);
        let mut plan = Schedule::new();
        plan.add_system("fixture.supply", 0u8, supply)
            .map_err(|_| ())?;
        plan.add_plan("embark", 1, embark).map_err(|_| ())?;
        plan.add_system("fixture.probe", 2, probe).map_err(|_| ())?;
        plan.add_subplans(
            "fixture.failure",
            3,
            select_probe,
            alloc::vec![(
                (),
                schedule::maintenance()
                    .map_err(|_| ())?
                    .map_error(|_| verdict::Fail::NotReady)
            )],
            inject,
        )
        .map_err(|_| ())?;
        children.push((Key::Embark, plan.build().map_err(|_| ())?));
        let mut plans = [
            schedule::maintenance()
                .map_err(|_| ())?
                .map_error(|_| Fail::Publication),
            lifecycle::actions(children).map_err(|_| ())?,
            schedule::frame().map_err(|_| ())?,
            schedule::shutdown().map_err(|_| ())?,
        ];
        start.prepare(&resources);
        for plan in &mut plans {
            plan.prepare(&resources);
        }
        start
            .advance(&mut Cursor::default(), &resources)
            .map_err(|_| ())?;
        Ok(Self {
            resources,
            plans,
            cursors: core::array::from_fn(|_| Cursor::default()),
        })
    }
    pub(crate) fn assemble(&mut self, program: &UnitFile) -> Result<(), Died> {
        self.action(program.name(), Action::Mint)
            .map_err(|_| start::E_PROGRAM)?;
        self.action(program.name(), Action::Embark { parent: None })
            .map_err(|_| start::E_PROGRAM)?;
        Ok(())
    }
    pub(crate) fn action(&mut self, name: &str, action: Action) -> Result<Option<env::TaskId>, ()> {
        self.resources
            .write::<Operations>()
            .map_err(|_| ())?
            .submit(name.into(), action)
            .map_err(|_| ())?;
        loop {
            if self.plans[1]
                .advance(&mut self.cursors[1], &self.resources)
                .map_err(|_| ())?
                == Progress::Done
            {
                self.cursors[1].reset();
            }
            self.progress().map_err(|_| ())?;
            {
                let mut operations = self.resources.write::<Operations>().map_err(|_| ())?;
                if let Some(result) = operations.take_local_completion(name) {
                    return result.map_err(|_| ());
                }
            }
            execution::room::park(core::time::Duration::from_millis(1)).map_err(|_| ())?;
        }
    }
    pub(crate) fn progress(&mut self) -> Result<(), &'static str> {
        if self.plans[0]
            .advance(&mut self.cursors[0], &self.resources)
            .map_err(|_| "fixture maintenance")?
            == Progress::Done
        {
            self.cursors[0].reset();
        }
        Ok(())
    }
    pub(crate) fn settle(&mut self) {
        self.resources
            .write::<crate::system::app::policy::Flow>()
            .unwrap()
            .settling = true;
    }
    pub(crate) fn supervise(&mut self) -> Result<(), Fail> {
        while !self
            .resources
            .read::<crate::system::app::policy::Flow>()
            .map_err(|_| Fail::Room)?
            .done
        {
            if self.plans[2]
                .advance(&mut self.cursors[2], &self.resources)
                .map_err(|error| {
                    programs::debug::put(&alloc::format!("fixture: frame {:?}", error));
                    Fail::Shutdown
                })?
                == Progress::Done
            {
                self.cursors[2].reset();
            }
        }
        while self.plans[3]
            .advance(&mut self.cursors[3], &self.resources)
            .map_err(|_| Fail::Shutdown)?
            == Progress::Pending
        {}
        Ok(())
    }
}
