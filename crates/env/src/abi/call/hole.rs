//! Bounded message queues and counted notifications.
use crate::{Mark, Oversize, PieToken, PullOutcome, TaskId, VirtAddr, Wait};
use mold::Envcall;

pub use super::hole_condition::HoleCondition;
pub use super::hole_fail::HoleFail;
pub use super::hole_limits::HoleLimits;
pub type HoleResult<T> = Result<T, erra::Error<HoleFail>>;

#[derive(Envcall)]
#[call(class = 5, fail = HoleFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HoleCall {
    #[ret(PieToken)]
    Unseal { mark: Mark, limits: HoleLimits },
    #[ret(())]
    Seal { token: PieToken },
    #[ret(())]
    Push {
        token: PieToken,
        msg: VirtAddr,
        len: usize,
    },
    #[ret3(PullOutcome)]
    Pull {
        token: PieToken,
        buf: VirtAddr,
        max: usize,
        oversize: Oversize,
    },
    #[ret3((usize, TaskId, usize))]
    Peek { token: PieToken },
    #[ret(bool)]
    Wait {
        token: PieToken,
        condition: HoleCondition,
        millis: Wait,
    },
    #[ret(())]
    Ring { token: PieToken },
    #[ret(())]
    Hush { token: PieToken },
}
