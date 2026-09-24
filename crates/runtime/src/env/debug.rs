//! Debug 域：`DebugCall::*` 转发（借内核的 DBCN 打印/读入）。
//!
//! 它们是"服务还没起来的嘴"，不是一条新的控制台通路——生产里域打印仍归 console 服务
//! （纪律，不是编译期的事：见 `env::fid::DebugCall` 那条"不设构建门"的理由）。
//!
//! 每一个函数封一次 envcall，零逻辑，与 `env::chrono`/`env::room` 同形。

use env::{DBCN_MAX, DebugCall, DebugCallRet, EnvResult};

/// 把一个字符串写进调试控制台（`len == 0` ⇒ `Denied`；返**写出去的字节数**）。
///
/// **超 [`DBCN_MAX`] 的那一段被内核截断**（本层照 `s.len()` 递上去，不预截）：
/// 一件长东西只印出前 `DBCN_MAX` 字节，返回值就是那个数。**这不是错误**——与
/// [`get`] 的"多出即拒"不同形（两处的实测读法见 `env::fid::DebugCall`）。
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

/// 开关**报文对账**（此后每次一次往返都打两帧的十六进制）。
///
/// 为什么留在 ABI 上而不是编译期开关：布局错位只有**真实字节**能证，而这类病一旦出现
/// 就要能在**同一个产物**上打开对账再跑一遍。
pub fn trace(on: bool) -> EnvResult<()> {
    set_local(on);
    let call = DebugCall::SetTrace { on: on as usize };
    match call.call()? {
        DebugCallRet::SetTrace(()) => Ok(()),
        _ => unreachable!(),
    }
}

static TRACE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// 本域的对账开关。域是独立地址空间，这份静态因此**每域一份**：内核那份只由 envcall
/// 打开，而"哪个域要打"是调用方在 `trace()` 里当场决定的。
///
/// **今天零读者**：原先读它的是 `Port::call`，那一层已搬去各协议。
pub fn tracing() -> bool {
    TRACE.load(core::sync::atomic::Ordering::Relaxed)
}

/// 同 [`trace`]，但只动本地那份（内核侧那份由 envcall 写）。
pub fn set_local(on: bool) {
    TRACE.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// 把一段字节以十六进制打进调试控制台（每行一小段）——**对账用**。
///
/// 布局错位这类病的唯一证词就是真实字节；读数形如 `port send: 01 00 ...`。
pub fn hexdump(tag: &str, bytes: &[u8]) {
    const LINE: usize = 16;
    let mut s = alloc::string::String::new();
    for chunk in bytes.chunks(LINE) {
        s.clear();
        s.push_str(tag);
        s.push_str(": ");
        for b in chunk {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0xf) as usize] as char);
            s.push(' ');
        }
        s.push('\n');
        let _ = put(&s);
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";
