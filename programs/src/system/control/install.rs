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
) -> Result<(), &'static str> {
    macro_rules! put {
        ($value:expr) => {
            resources
                .insert($value)
                .map_err(|_| "Control resource capacity")?
        };
    }
    let mut control = Control::new(status);
    control.inputs = config.inputs;
    put!(control);
    put!(Roster::default());
    put!(hook::Active::default());
    put!(Dispatch::<hook::Key, &'static str>::new());
    put!(lifecycle::Startup::new());
    put!(lifecycle::Operations::new());
    put!(lifecycle::Active(None));
    put!(Dispatch::<lifecycle::Key, verdict::Fail>::new());
    put!(Entries::new());
    put!(super::endpoint::construction::Construction::new()?);
    put!(answer::Inbox(VecDeque::new()));
    put!(answer::Buffer(alloc::vec![0; env::PAGE_SIZE]));
    Ok(())
}
