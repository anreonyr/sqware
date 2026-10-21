//! Shared-memory mappings and indexed notifications.
use crate::{FlatReference, PieToken, VirtAddr, Wait};
use mold::Envcall;

pub use super::pole_bit::Bit;
pub use super::pole_condition::PoleCondition;
pub use super::pole_fail::PoleFail;
pub type PoleResult<T> = Result<T, erra::Error<PoleFail>>;

#[derive(Envcall)]
#[call(class = 10, fail = PoleFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PoleCall {
    #[ret(PieToken)]
    Unseal {
        reference: FlatReference,
        shared: bool,
    },
    #[ret(())]
    Seal { token: PieToken },
    #[ret((VirtAddr, usize))]
    Open { token: PieToken },
    #[ret(())]
    Shut { token: PieToken },
    #[ret(bool)]
    Wait {
        token: PieToken,
        condition: PoleCondition,
        millis: Wait,
    },
    #[ret(())]
    Ring { token: PieToken, bit: Bit },
    #[ret(())]
    Hush { token: PieToken, bit: Bit },
}
