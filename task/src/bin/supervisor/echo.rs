#![no_std]
#![no_main]
//! echo — supervisor 域里的回显服务（载荷逐字节 +1）。
//!
//! 协议与内核闭包版 echo 一致（见 `docs/dispatch.md`）：
//!   请求 = [0..8] 调用方回信 token（LE） + [8..64] 载荷
//!   回复 = 同布局（前 8 字节清零）
//!
//! 入口门闩由内核在 boot 期放进本任务权限表**索引 0**（与 shell 的目录门闩同款
//! 根授予），用 `Collect` 取回——不需要向用户态传任何整数。
//!
//! 已知边界：用户态拿不到 hole 的 wait key（内核键 = `HoleMeta` 地址、命名空间
//! asid 0，见 `kernel/src/work/mail/hole.rs::pull_key`），故本服务对入口 hole 用
//! `HolePie::pull` 的短 spin 轮询而非 park。服务一旦有请求即回，空闲时占满所在核
//! ——v1 演示可接受；正式通道（暴露 hole 等待键 / wait-on-hole 原语）是后续议题。

extern crate alloc;

use task::env::mail::{self, HolePie};

/// hole 单消息字节数（与内核 `HOLE_MSG_LEN` 一致）。
const MSG_LEN: usize = 64;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 入口门闩：内核在 boot 期放在权限表索引 0。
    let (token, _) = match mail::collect(0) {
        Ok(v) => v,
        Err(_) => task::env::control::panic(1),
    };
    if token == 0 {
        task::env::control::panic(2);
    }
    let entry = HolePie::from_token(token);

    let mut buf = [0u8; MSG_LEN];
    loop {
        // 空槽由 HolePie::pull 内部短 spin 重试；Err 只在 hole 死亡时出现。
        if entry.pull(&mut buf).is_err() {
            continue;
        }
        let reply_token = u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0u8; 8]));
        buf[0..8].fill(0);
        for b in buf[8..].iter_mut() {
            *b = b.wrapping_add(1);
        }
        // 回信 hole 属调用方：其 pie 由调用方 `Accord` 授到本任务表，按 token 寻址。
        let reply = HolePie::from_token(reply_token);
        let _ = reply.push(&buf);
    }
}
