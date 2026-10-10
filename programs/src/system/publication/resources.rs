use super::{Mounts, book::Publications, living::Living, names, runtime};
use crate::system::operator::management::Tree;
use ::schedule::{Dispatch, Resources};
use alloc::{collections::VecDeque, vec::Vec};
pub(crate) fn install(
    resources: &mut Resources<'static>,
) -> Result<env::PieToken, crate::system::app::InstallError> {
    let entry = env::pie::unseal(env::UnsealArgs::hole(
        system_api::control::publication::ENTRY,
    ))?;

    resources
        .insert(super::Entry(entry))?
        .insert(Mounts(Vec::new()))?
        .insert(Tree::default())?
        .insert(Publications::new())?
        .insert(runtime::Resources::new())?
        .insert(runtime::Runtimes {
            requests: Vec::new(),
            seen: 0,
            checked: Vec::new(),
        })?
        .insert(names::Names::new())?
        .insert(names::Registrations {
            requests: Vec::new(),
            seen: 0,
            dirty: false,
        })?
        .insert(Living::new())?
        .insert(super::Inbox(VecDeque::new()))?
        .insert(super::Request(None, None))?
        .insert(super::Outcome(None))?
        .insert(super::Decision::Unset)?
        .insert(super::Kind { road: None })?
        .insert(Dispatch::<u8, &'static str>::new())?;
    Ok(entry)
}
