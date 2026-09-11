#![no_std]
#![no_main]
//! echo — supervisor 域里的回显服务（载荷逐字节 +1），启动后**自注册**到目录。
//!
//! 启动流程：
//!   1. 靠泊 → 自建**控制孔**（与入口门闩分离）交给父域 → 报到。
//!   2. 收配给：目录请求门闩的句柄；目录 id = `Owned(门闩).owner`（资源开辟者）。
//!   3. 自建入口门闩（**服务自开**——客户端靠它的 owner 找到本域）。
//!   4. `Directory::register`：把入口门闩交给目录保管（`Accord` 带 VEST）并登记名字。
//!      名字由父域在启动期**预约**给本域，故注册必成；不是预约者则被拒。
//!   5. 进入服务循环：pull entry、+1 载荷、push 到调用方自带的 reply。
//!
//! 协议规范见 `docs/dispatch.md`；服务调用载荷 56 字节（`MSG_LEN` - 8 字节回信 token）。

extern crate alloc;

// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use env::Permission;
use protocol::dispatch::MSG_LEN;
use protocol::dispatch::client::Directory;
use runtime::core::handshake::{self, Pier, Quay};
use runtime::env::mail::{AnyPie as _, HolePie};

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 靠泊 + 自建控制孔（只给父域；与入口门闩分离——父域拿不到请求队列）。
    let up = match handshake::moor() {
        Ok(u) => u,
        Err(_) => runtime::env::room::exit_with(1),
    };
    let down = match HolePie::unseal(handshake::MTU) {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(2),
    };
    // 2. 自建入口门闩——服务自己开自己的门。
    let entry = match HolePie::unseal(MSG_LEN) {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(3),
    };
    let sire = match runtime::env::task::sire() {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(4),
    };
    let at_parent = match down.accord(sire, Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(5),
    };
    if Quay::new(at_parent).push(&up).is_err() {
        runtime::env::room::exit_with(6);
    }

    // 3. 收配给：目录请求门闩（由 dir 亲授，root 只转达）+ 目录身份。
    let pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(7),
    };
    // 4. 注册：入口门闩落进目录权限表（目录要能再转授给客户端），名字记在目录里。
    let dir = match Directory::open(HolePie::from_token(pier.token())) {
        Ok(d) => d,
        Err(_) => runtime::env::room::exit_with(8),
    };
    if dir.register("echo", &entry).is_err() {
        runtime::env::room::exit_with(9);
    }

    // 5. 服务循环：pull 请求、+1 载荷、push 到客户端自带的 reply token。
    let mut req_buf = [0u8; MSG_LEN];
    loop {
        if entry.pull(&mut req_buf).is_err() {
            continue;
        }
        let reply_token = usize::from_le_bytes(req_buf[0..8].try_into().unwrap_or([0u8; 8]));
        req_buf[0..8].fill(0);
        for b in req_buf[8..].iter_mut() {
            *b = b.wrapping_add(1);
        }
        let client_reply = HolePie::from_token(reply_token);
        let _ = client_reply.push(&req_buf);
    }
}
