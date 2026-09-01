//! IO 域：`IOCall::*` 转发。

use core::time::Duration;

use ubi::{IOCall, UArgs, UResult, Ucall, UcallBuilder};

use crate::env::room;

// 硬不变量：put / try_put 共用 IOCall::Put（best-effort 直写），差异在错误传播；
//             IOCall::Get 已非阻塞，try_get 直接复用。

pub fn put(s: &str) -> UResult<()> {
    let args = UArgs { a0: s.len(), a1: s.as_ptr() as usize, ..UArgs::default() };
    let _ = UcallBuilder::new(Ucall::IO(IOCall::Put)).args(args).call()?;
    Ok(())
}

pub fn try_put(s: &str) -> UResult<()> {
    let args = UArgs { a0: s.len(), a1: s.as_ptr() as usize, ..UArgs::default() };
    UcallBuilder::new(Ucall::IO(IOCall::Put)).args(args).call()?;
    Ok(())
}

pub fn try_get() -> Option<u8> {
    let (v0, _) = UcallBuilder::new(Ucall::IO(IOCall::Get)).call().ok()?;
    Some(v0 as u8)
}

pub fn get() -> u8 {
    loop {
        if let Some(b) = try_get() { return b; }
        let _ = room::sleep(Duration::from_millis(1));
    }
}