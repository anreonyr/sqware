//! IO 域：`IOCall::*` 转发。

use env::{EnvResult, IOCall, IOCallRet, VirtAddr};

// 硬不变量：`IOCall::Get` 非阻塞，故 `try_get` 是唯一读入口（调用方自担轮询/
//             避忙等策略——`term::readline` 即在其上加重绘与 1ms 让出）。

pub fn put(s: &str) -> EnvResult<()> {
    let r = IOCall::Put {
        len: s.len(),
        buf: VirtAddr::new(s.as_ptr() as usize),
    }
    .call()?;
    match r {
        IOCallRet::Put(()) => Ok(()),
        _ => unreachable!(),
    }
}

pub fn try_get() -> Option<u8> {
    let r = IOCall::Get.call().ok()?;
    match r {
        IOCallRet::Get(b) => Some(b),
        _ => unreachable!(),
    }
}
