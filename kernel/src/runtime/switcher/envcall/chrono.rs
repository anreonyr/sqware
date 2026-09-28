use env::ChronoCall;

use crate::runtime::chrono::{clock, timer};
use crate::runtime::switcher::context::{Gprs, TrapContext};

pub(super) fn dispatch(frame: &mut TrapContext, call: ChronoCall) {
    match call {
        ChronoCall::Ticks => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        ChronoCall::Clock => {
            let ns = clock::uptime().as_nanos().min(u64::MAX as u128) as u64;
            frame.gpr.set_x(Gprs::A0, ns as usize);
        }
    }
}