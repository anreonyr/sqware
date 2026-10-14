use crate::core::work::pend::Pie;
use env::wire::Field;
use env::Name;

pub(crate) mod ledger;
mod failure;
mod frozen;
mod registry;

use failure::Failure;
pub(crate) use frozen::Frozen;
pub(crate) use registry::Registry;

pub(crate) type RegisterError = Failure<(Name, Pie)>;
pub(crate) type FreezeError = Failure<Registry>;
pub(crate) type GrantError = Failure<Frozen>;

fn valid(name: Name, pie: &Pie) -> bool {
    let mut bytes = [0u8; env::NAME_LEN];
    name.store(&mut bytes);
    Name::fetch(&bytes) == Some(name)
        && name.kind() == pie.mail().kind()
        && pie.alive()
        && pie.sire().is_none()
        && pie.heir().is_none()
        && !pie.permission().is_empty()
}
