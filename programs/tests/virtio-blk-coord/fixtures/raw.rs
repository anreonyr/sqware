pub use super::handoff::Hole;

use env::{PieInfo, PieToken};
use crate::fixtures::state;

pub fn inspect(token: PieToken) -> Result<PieInfo, ()> {
    state::inspect(token)
}

pub fn open(token: PieToken) -> Result<(usize, usize), env::PoleFail> {
    state::open(token)
}
