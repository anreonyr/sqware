use super::{
    endpoint::{Entries, request as answer},
    identity::Roster,
    lifecycle,
    unit::{Control, start::Input, verdict},
};
use crate::system::app::life::Status;
use ::schedule::{Dispatch, Resources};
use alloc::{collections::VecDeque, sync::Arc};

pub(crate) struct Configuration {
    pub inputs: alloc::vec::Vec<Input>,
    pub targets: &'static [crate::unit::Target],
}
pub(crate) fn install(
    resources: &mut Resources<'static>,
    status: Arc<Status>,
    config: Configuration,
) -> Result<(), crate::system::app::InstallError> {
    let mut control = Control::new(status, config.targets);
    control.inputs = config.inputs;
    resources
        .insert(control)?
        .insert(Roster::default())?
        .insert(Dispatch::<super::instance::hook::Key, &'static str>::new())?
        .insert(lifecycle::Startup::new())?
        .insert(lifecycle::Active(None))?
        .insert(Dispatch::<lifecycle::Key, verdict::Fail>::new())?
        .insert(super::endpoint::construction::Construction::new()?)?
        .insert(answer::Inbox(VecDeque::new()))?
        .insert(answer::Buffer(alloc::vec![0; env::PAGE_SIZE]))?;
    super::namespace::install(resources)?;
    Ok(())
}
