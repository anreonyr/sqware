use crate::service::hub::bridge::Activation;
use crate::system::common::machine::Machine;
use crate::system::control::{core::publication::Publications,
    serve::{living::Living, resource::Resources, unit::Control}};
use crate::system::identity::serve::{install::Roster, names::Names};
use crate::system::operator::serve::install::Tree;
use env::PieToken;
use protocol::common::schedule::{BuildError, Cursor, Plan, Progress, RunError, Resources as Registry};

pub struct Cycle { plan: Plan<&'static str>, living: Living }
impl Cycle {
    pub fn new() -> Result<Self, BuildError> {
        Ok(Self { plan: super::schedule::cycle()?, living: Living::new() })
    }
    pub fn poll(&mut self, control: &Control, roster: &Roster, machine: &Machine,
        activation: &Option<Activation>, entry: PieToken, publications: &mut Publications,
        resources: &mut Resources, names: &mut Names, tree: &mut Tree) -> Result<(), &'static str> {
        let mut registry = Registry::new();
        registry.observe(control).map_err(|_| "cycle Control resource")?;
        registry.observe(roster).map_err(|_| "cycle Identity resource")?;
        registry.observe(machine).map_err(|_| "cycle machine resource")?;
        registry.observe(activation).map_err(|_| "cycle activation resource")?;
        registry.observe(&entry).map_err(|_| "cycle entry resource")?;
        registry.borrow(publications).map_err(|_| "cycle publication resource")?;
        registry.borrow(resources).map_err(|_| "cycle runtime resource")?;
        registry.borrow(names).map_err(|_| "cycle names resource")?;
        registry.borrow(tree).map_err(|_| "cycle Operator resource")?;
        registry.borrow(&mut self.living).map_err(|_| "cycle living resource")?;
        match self.plan.advance(&mut Cursor::default(), &registry) {
            Ok(Progress::Done) => Ok(()),
            Ok(Progress::Pending) => Err("cycle unexpectedly pending"),
            Err(RunError::Step(why)) => Err(why),
            Err(RunError::Resource(_)) => Err("cycle resource access"),
        }
    }
}
