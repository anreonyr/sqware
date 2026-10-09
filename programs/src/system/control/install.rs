use super::{
    identity::Roster,
    instance::hook,
    lifecycle,
    endpoint::{request as answer, Entries},
    unit::{Control, start::Input, verdict},
};
use crate::system::app::life::Status;
use ::schedule::{Dispatch, Resources};
use alloc::{collections::VecDeque, sync::Arc};

pub(crate) struct Configuration {
    pub inputs: alloc::vec::Vec<Input>,
}
pub(crate) fn install(
    resources: &mut Resources<'static>,
    status: Arc<Status>,
    config: Configuration,
) -> Result<(), crate::system::app::InstallError> {
    let mut control = Control::new(status);
    control.inputs = config.inputs;
    resources
        .insert(control)?
        .insert(Roster::default())?
        .insert(hook::Active::default())?
        .insert(Dispatch::<hook::Key, &'static str>::new())?
        .insert(lifecycle::Startup::new())?
        .insert(lifecycle::Operations::new())?
        .insert(lifecycle::Active(None))?
        .insert(Dispatch::<lifecycle::Key, verdict::Fail>::new())?
        .insert(Entries::new())?
        .insert(super::endpoint::construction::Construction::new()?)?
        .insert(answer::Inbox(VecDeque::new()))?
        .insert(answer::Buffer(alloc::vec![0; env::PAGE_SIZE]))?;
    Ok(())
}
