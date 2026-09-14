//! Debug 域：`DebugCall::*` 转发（借内核的 DBCN 打印/读入）。
//!
//! 它们是"服务还没起来的嘴"，不是一条新的控制台通路——生产里域打印仍归 console 服务
//! （纪律，不是编译期的事：见 `env::fid::DebugCall` 那条"不设构建门"的理由）。
//!
//! 每一个函数封一次 envcall，零逻辑，与 `env::chrono`/`env::room` 同形。

use env::{DBCN_MAX, DebugCall, DebugCallRet, EnvResult};

/// 把一个字符串写进调试控制台（`> DBCN_MAX` ⇒ `Denied`，**不截断**）。
pub fn put(s: &str) -> EnvResult<usize> {
    let bytes = s.as_bytes();
    let call = DebugCall::Put {
        buf: env::VirtAddr::new(bytes.as_ptr() as usize),
        len: bytes.len(),
    };
    match call.call()? {
        DebugCallRet::Put(n) => Ok(n),
        _ => unreachable!(),
    }
}

/// 从调试控制台读一段字节写进 `buf`，返**实际写入的字节数**。
///
/// **可能阻塞**（SBI 的 console read 语义）：单核上会挂住整机，直到串口来字节。
/// 敢不敢在这儿等，是调用方的判断（见 `env::fid::DebugCall::Get` 的注）。
pub fn get(buf: &mut [u8]) -> EnvResult<usize> {
    let len = buf.len().min(DBCN_MAX);
    let call = DebugCall::Get {
        buf: env::VirtAddr::new(buf.as_mut_ptr() as usize),
        len,
    };
    match call.call()? {
        DebugCallRet::Get(n) => Ok(n),
        _ => unreachable!(),
    }
}
