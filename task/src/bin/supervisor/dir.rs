#![no_std]
#![no_main]
//! dir — 服务目录（S 态 supervisor 域）。
//!
//! 目录是**普通 Service**：启动期自建请求门闩（`UnsealHole`），经启动期握手把它在
//! root 侧的句柄交出去（root 负责分发给客户端），随后进入「pull 请求 → 认人 →
//! 处理 → push 回复」循环；槽空时内核 `Wait` 挂起本任务（park），被对侧 push 唤醒
//! ——空闲 0% CPU（见 `docs/ipc.md` §16）。
//!
//! **自开门闩是硬规则**：客户端用 `MailCall::Owned` 从门闩副本的 `owner` 求目录
//! task id；若由他人代开，客户端会把回信 hole 授给代开者（见 `docs/dispatch.md`）。
//!
//! **认人**：请求 `[49..57]` 是调用方 `Accord` 给本域的回信 pie token；`Owned`
//! 报出它的 `vestor`——内核在 `Accord` 时赋值，消息体伪造不了。没带有效回信 pie
//! 即无身份（caller = 0）：Register 不看身份，Unregister/Replace/Connect 一律拒绝。
//!
//! 注册表逻辑在 `task::core::directory`；协议规范见 `docs/dispatch.md`。

extern crate alloc;

use env::Permission;
use env::dispatch::{MSG_LEN, REPLY_AT};
use task::core::directory::{Directory, vestor_of};
use task::core::handshake::{self, Pier, Quay};
use task::env::mail::HolePie;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 靠泊：认父域开的报到孔。
    let quay = match handshake::moor() {
        Ok(q) => q,
        Err(_) => task::env::control::panic(1),
    };
    // 2. 自建请求门闩——服务自己开自己的门；它同时是收配给的通道。
    let entry = match HolePie::unseal(MSG_LEN) {
        Ok(h) => h,
        Err(_) => task::env::control::panic(2),
    };
    // 3. 交给父域。**必须带 VEST**——root 要接着把它分发给各客户端（客户端只拿 R|W）。
    let sire = match task::env::task::sire() {
        Ok(t) => t,
        Err(_) => task::env::control::panic(3),
    };
    let hole = match entry.accord(
        sire.get(),
        Permission::READ | Permission::WRITE | Permission::VEST,
    ) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(4),
    };
    // 4. 报到：把本门闩在父侧的句柄交给 root。
    if Quay::new(hole).push(&quay).is_err() {
        task::env::control::panic(5);
    }
    // 5. 收配给：目录不需要 root 给任何门闩，收到空配给即握手完成。
    if Pier::pull(&entry).is_err() {
        task::env::control::panic(6);
    }

    let mut dir = Directory::new();
    let mut msg = [0u8; MSG_LEN];
    loop {
        // 槽空 → 挂起让出 CPU；Err 只在门闩死亡时出现（本域自开的 hole 不会被封印）。
        if entry.pull(&mut msg).is_err() {
            continue;
        }
        let reply_token =
            u64::from_le_bytes(msg[REPLY_AT..REPLY_AT + 8].try_into().unwrap_or([0u8; 8]));
        // 回信 pie 在本域表里 → 其 vestor 即本次请求的 caller（0 = 无授与人）。
        let reply = vestor_of(reply_token);
        let out = dir.serve(reply.unwrap_or(0), &msg).encode();
        let Some(_) = reply else {
            // 无回信通道（fire-and-forget 或 token 不在表里）：回复无处可去，丢弃。
            continue;
        };
        // 推回调用方自带的回信 hole；槽满则挂起等对侧取走，Dead 即丢。
        let _ = HolePie::from_token(reply_token).push(&out);
    }
}
