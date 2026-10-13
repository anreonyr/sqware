use riscv::register::time;

use sbi::{TimerCall, ecall::SArgs, fid::Timer};

pub(crate) fn beat(interval: u64) {
    let next = (time::read() as u64).wrapping_add(interval);
    TimerCall::new(Timer::SetTimer)
        .args(SArgs {
            a0: next as usize,
            ..Default::default()
        })
        .call()
        .unwrap();
}
