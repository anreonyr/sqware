use crate::{PieToken, Wait};
use mold::Envcall;

pub use super::tole_await_reply::ToleAwaitReply;
pub use super::tole_event_fail::ToleEventFail;
pub use super::tole_event_source::ToleEventSource;
pub use super::tole_fail::ToleFail;
pub type ToleResult<T> = Result<T, erra::Error<ToleFail>>;

#[derive(Envcall)]
#[call(class = 12, fail = ToleFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToleCall {
    #[ret(PieToken)]
    Unseal { shared: bool },
    #[ret(())]
    Seal { token: PieToken },
    #[ret(())]
    Attach {
        tole: PieToken,
        source: ToleEventSource,
    },
    #[ret(())]
    Detach {
        tole: PieToken,
        source: ToleEventSource,
    },
    #[ret3(ToleAwaitReply)]
    Await { tole: PieToken, millis: Wait },
}
