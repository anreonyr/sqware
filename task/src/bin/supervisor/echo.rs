#![no_std]
#![no_main]
//! echo — supervisor 域里的回显服务（载荷逐字节 +1），启动后**自注册**到目录。
//!
//! 启动流程：
//!   1. Collect(0) 取自己入口门闩（内核 boot 预置的「原始自持」副本）。
//!   2. Collect(1) 取目录入口门闩 + 目录 task id（vestor）。
//!   3. entry.accord(dir_id, R|W) → 目录侧 entry 副本（token 在 Register 时回填）。
//!   4. 自造 reply hole，accord 给目录 → reply_target。
//!   5. 构造 `Request::Register { name, entry }`，把 reply_target 写进 `[49..57]`，push。
//!   6. 从自己 reply pull，等 `Reply::Ok`——失败则 panic。
//!   7. 进入服务循环：pull entry、+1 载荷、push 到调用方自带的 reply。
//!
//! 协议规范见 `docs/dispatch.md`；服务调用载荷 56 字节（`MSG_LEN` - 8 字节回信 token）。

extern crate alloc;

use env::Permission;
use env::dispatch::{self, Name, Reply, Request};
use task::env::mail::{self, HolePie};

/// dispatch 协议载荷定 64 字节。
const MSG_LEN: usize = dispatch::MSG_LEN;

/// 等 Register 回复的上界（毫秒）——有上界才不会因目录漏回而永久挂起。
const REPLY_TIMEOUT_MS: usize = 1000;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 自己的入口门闩（boot 预置在权限表索引 0）。
    let (entry_tok, _entry_perm, _entry_vestor) = match mail::collect(0) {
        Ok(v) => v,
        Err(_) => task::env::control::panic(1),
    };
    if entry_tok == 0 {
        task::env::control::panic(2);
    }
    let entry = HolePie::from_token(entry_tok);

    // 2. 目录入口门闩 + 目录 task id（boot 预置在权限表索引 1）。
    let (dir_entry_tok, _dir_perm, dir_id) = match mail::collect(1) {
        Ok(v) => v,
        Err(_) => task::env::control::panic(3),
    };
    if dir_entry_tok == 0 || dir_id.get() == 0 {
        task::env::control::panic(4);
    }
    let dir_entry = HolePie::from_token(dir_entry_tok);

    // 3. entry 副本落进目录权限表——目录 Register 处理器从 [33..41] 读 entry token
    //    然后 take_entry(me, token) 摘出。subset = R|W|VEST：**必须带 VEST**，
    //    后续 Connect 时目录要把这个 entry 再转授给 shell。
    let entry_target = match entry.accord(
        dir_id.get(),
        Permission::READ | Permission::WRITE | Permission::VEST,
    ) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(5),
    };

    // 4. 自造 reply hole，accord 给目录作 per-caller 通道。
    let reply_mine = match HolePie::unseal(mail::HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => task::env::control::panic(6),
    };
    let reply_target = match reply_mine.accord(dir_id.get(), Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(7),
    };

    // 5. 构造 Register 请求，push。
    let name = match Name::new("echo") {
        Ok(n) => n,
        Err(_) => task::env::control::panic(8),
    };
    let mut msg = Request::Register {
        name,
        entry: env::PieToken(entry_target),
    }
    .encode();
    msg[dispatch::REPLY_AT..dispatch::REPLY_AT + 8].copy_from_slice(&reply_target.to_le_bytes());
    if dir_entry.push(&msg).is_err() {
        task::env::control::panic(9);
    }

    // 6. 等 Ok 回复（有界等待；目录若回 Denied/Taken 或漏回都 panic）。
    let mut buf = [0u8; MSG_LEN];
    if reply_mine.pull_timeout(&mut buf, REPLY_TIMEOUT_MS).is_err() {
        task::env::control::panic(10);
    }
    match Reply::decode(&buf) {
        Ok(Reply::Ok) => {}
        _ => task::env::control::panic(11),
    }

    // 7. 服务循环：pull 请求、+1 载荷、push 到客户端自带的 reply token。
    let mut req_buf = [0u8; MSG_LEN];
    loop {
        if entry.pull(&mut req_buf).is_err() {
            continue;
        }
        let reply_token = u64::from_le_bytes(req_buf[0..8].try_into().unwrap_or([0u8; 8]));
        req_buf[0..8].fill(0);
        for b in req_buf[8..].iter_mut() {
            *b = b.wrapping_add(1);
        }
        let client_reply = HolePie::from_token(reply_token);
        let _ = client_reply.push(&req_buf);
    }
}
