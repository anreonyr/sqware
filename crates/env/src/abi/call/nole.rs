//! Notifications without indexed signal bits.
use crate::{PieToken, Wait};
use mold::Envcall;

pub use super::nole_fail::NoleFail;
pub type NoleResult<T> = Result<T, erra::Error<NoleFail>>;

#[derive(Envcall)]
#[call(class = 11, fail = NoleFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoleCall {
    #[ret(PieToken)]
    Unseal,
    #[ret(())]
    Seal { token: PieToken },
    #[ret(bool)]
    Wait { token: PieToken, millis: Wait },
    #[ret(())]
    Ring { token: PieToken },
    #[ret(())]
    Hush { token: PieToken },
}
