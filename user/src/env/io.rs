//! IO 域：`IOCall::*` 转发。

use core::time::Duration;

use ubi::{IOCall, IOCallRet, EnvResult, VirtAddr};

use crate::env::room;

// 硬不变量：put / try_put 共用 IOCall::Put（best-effort 直写），差异在错误传播；
//             IOCall::Get 已非阻塞，try_get 直接复用。

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

pub fn try_put(s: &str) -> EnvResult<()> {
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

pub fn get() -> u8 {
    loop {
        if let Some(b) = try_get() {
            return b;
        }
        let _ = room::sleep(Duration::from_millis(1));
    }
}
