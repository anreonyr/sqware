#![no_std]
#![no_main]
//! echo — supervisor 域里的回显服务（载荷逐字节 +1），启动后**自注册**到目录。
//!
//! 启动流程：
//!   1. 靠泊 → 自建**控制孔**（与入口门闩分离）交给父域 → 报到。
//!   2. 收配给：目录请求门闩的句柄；目录 id = `Owned(门闩).owner`（资源开辟者）。
//!   3. 自建入口门闩（**服务自开**——客户端靠它的 owner 找到本域）。
//!   4. `entry.accord(dir_id, R|W|VEST)` → 目录侧 entry 副本（token 在 Register 回填）。
//!   5. 自造 reply hole，accord 给目录 → reply_target。
//!   6. 构造 `Request::Register { name, entry }`，把 reply_target 写进 `[49..57]`，push。
//!   7. 从自己 reply pull，等 `Reply::Ok`——失败则 panic。
//!   8. 进入服务循环：pull entry、+1 载荷、push 到调用方自带的 reply。
//!
//! 协议规范见 `docs/dispatch.md`；服务调用载荷 56 字节（`MSG_LEN` - 8 字节回信 token）。

extern crate alloc;

use env::Permission;
use env::dispatch::{self, Name, Reply, Request};
use task::core::handshake::{self, Pier, Quay};
use task::env::mail::{self, HOLE_MTU_MAX, HolePie};

/// dispatch 协议载荷定 64 字节。
const MSG_LEN: usize = dispatch::MSG_LEN;

/// 等 Register 回复的上界（毫秒）——有上界才不会因目录漏回而永久挂起。
const REPLY_TIMEOUT_MS: usize = 1000;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 靠泊 + 自建控制孔（只给父域；与入口门闩分离——父域拿不到请求队列）。
    let up = match handshake::moor() {
        Ok(u) => u,
        Err(_) => task::env::control::panic(1),
    };
    let down = match HolePie::unseal(handshake::MTU) {
        Ok(h) => h,
        Err(_) => task::env::control::panic(2),
    };
    // 2. 自建入口门闩——服务自己开自己的门。
    let entry = match HolePie::unseal(MSG_LEN) {
        Ok(h) => h,
        Err(_) => task::env::control::panic(3),
    };
    let sire = match task::env::task::sire() {
        Ok(t) => t,
        Err(_) => task::env::control::panic(4),
    };
    let at_parent = match down.accord(sire.get(), Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(5),
    };
    if Quay::new(at_parent).push(&up).is_err() {
        task::env::control::panic(6);
    }

    // 3. 收配给：目录请求门闩（由 dir 亲授，root 只转达）+ 目录身份。
    let pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => task::env::control::panic(7),
    };
    let dir_entry = HolePie::from_token(pier.token());
    let dir_id = match mail::owned(pier.token()) {
        Ok((_vestor, owner)) => owner.get(),
        Err(_) => task::env::control::panic(8),
    };
    if dir_id == 0 {
        task::env::control::panic(9);
    }

    // 4. entry 副本落进目录权限表——目录 Register 处理器从 [33..41] 读 entry token
    //    然后 bind（owner = 该副本的 vestor = 本域）。subset = R|W|VEST：**必须带
    //    VEST**，后续 Connect 时目录要把这个 entry 再转授给客户端。
    let entry_target = match entry.accord(
        dir_id,
        Permission::READ | Permission::WRITE | Permission::VEST,
    ) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(10),
    };

    // 5. 自造 reply hole，accord 给目录作 per-caller 通道。
    let reply_mine = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => task::env::control::panic(11),
    };
    let reply_target = match reply_mine.accord(dir_id, Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(12),
    };

    // 6. 构造 Register 请求，push。
    let name = match Name::new("echo") {
        Ok(n) => n,
        Err(_) => task::env::control::panic(13),
    };
    let mut msg = Request::Register {
        name,
        entry: env::PieToken(entry_target),
    }
    .encode();
    msg[dispatch::REPLY_AT..dispatch::REPLY_AT + 8].copy_from_slice(&reply_target.to_le_bytes());
    if dir_entry.push(&msg).is_err() {
        task::env::control::panic(14);
    }

    // 7. 等 Ok 回复（有界等待；目录若回 Denied/Taken 或漏回都 panic）。
    let mut buf = [0u8; MSG_LEN];
    if reply_mine.pull_timeout(&mut buf, REPLY_TIMEOUT_MS).is_err() {
        task::env::control::panic(15);
    }
    match Reply::decode(&buf) {
        Ok(Reply::Ok) => {}
        _ => task::env::control::panic(16),
    }

    // 8. 服务循环：pull 请求、+1 载荷、push 到客户端自带的 reply token。
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
