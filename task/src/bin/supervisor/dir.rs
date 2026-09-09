#![no_std]
#![no_main]
//! dir — 服务目录（S 态 supervisor 域）。
//!
//! 目录是**普通 Service**：启动后 `Collect(0)` 取自己的请求门闩（boot 根授予），
//! 进入「pull 请求 → 认人 → 处理 → push 回复」循环；槽空时内核 `Wait` 挂起本任务
//! （park），被对侧 push 唤醒——空闲 0% CPU（见 `docs/ipc.md` §16）。
//!
//! **认人**：请求 `[49..57]` 是调用方 `Accord` 给本域的回信 pie token；扫自己的
//! 权限表（`Collect`）取该 pie 的 `vestor`——内核在 `Accord` 时赋值，消息体伪造
//! 不了。没带有效回信 pie 即无身份（caller = 0）：Register 不看身份，
//! Unregister/Replace/Connect 一律拒绝。
//!
//! 注册表逻辑在 `task::core::directory`；协议规范见 `docs/dispatch.md`。

extern crate alloc;

use env::dispatch::{MSG_LEN, REPLY_AT};
use task::core::directory::{Directory, vestor_of};
use task::env::mail::{self, HolePie};

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 自己的请求门闩：boot 放在权限表索引 0（与 shell 取目录门闩同款根授予）。
    let (entry_tok, _perm, _vestor) = match mail::collect(0) {
        Ok(v) => v,
        Err(_) => task::env::control::panic(1),
    };
    if entry_tok == 0 {
        task::env::control::panic(2);
    }
    let entry = HolePie::from_token(entry_tok);

    let mut dir = Directory::new();
    let mut msg = [0u8; MSG_LEN];
    loop {
        // 槽空 → 挂起让出 CPU；Err 只在门闩死亡时出现（boot 建的 hole 不会被封印）。
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
