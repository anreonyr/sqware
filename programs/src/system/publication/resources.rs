use super::{Mounts, book::Publications, living::Living, names, runtime};
use crate::system::operator::management::Tree;
use ::schedule::{Dispatch, Resources};
use alloc::{collections::VecDeque, vec::Vec};
pub(crate) fn install(resources: &mut Resources<'static>) -> Result<env::PieToken, &'static str> {
    let entry = env::pie::unseal(env::UnsealArgs::hole(system_api::control::publication::ENTRY))
        .map_err(|_| "publication entry")?;
    macro_rules! put {
        ($value:expr) => {
            resources
                .insert($value)
                .map_err(|_| "publication resource capacity")?
        };
    }
    put!(super::Entry(entry));
    put!(Mounts(Vec::new()));
    put!(Tree::default());
    put!(Publications::new());
    put!(runtime::Resources::new());
    put!(runtime::Runtimes {
        requests: Vec::new(),
        seen: 0,
        checked: Vec::new()
    });
    put!(names::Names::new());
    put!(names::Registrations {
        requests: Vec::new(),
        seen: 0,
        dirty: false
    });
    put!(Living::new());
    put!(super::Inbox(VecDeque::new()));
    put!(super::Request(None, None));
    put!(super::Outcome(None));
    put!(super::Decision::Unset);
    put!(super::Kind { road: None });
    put!(Dispatch::<u8, &'static str>::new());
    Ok(entry)
}
