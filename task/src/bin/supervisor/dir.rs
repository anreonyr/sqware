#![no_std]
#![no_main]
//! dir — 服务目录（S 态 supervisor 域，**两个线程**）。
//!
//! ```text
//! 主线程      H 服务循环（pull 请求 → 认人 → 处理 → push 回复）+ 注册表
//! 控制线程    pull(C) → H.accord(who, R|W) → push(上行孔, Referred)   —— 不碰注册表
//! ```
//!
//! **为什么两个线程**：目录要同时听两条输入通道（客户端的请求孔 `H`、父域的引入
//! 孔 `C`），而 `Wait` 一次只能等一条孔——单线程 park 在 `H` 上就接不到引入请求。
//! 控制面与数据面分开，主线程的循环一字未改。
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

use core::sync::atomic::{AtomicUsize, Ordering};

use env::dispatch::{MSG_LEN, REPLY_AT};
use env::{Permission, TeamId};
use task::core::directory::{Directory, vestor_of};
use task::core::handshake::{self, Quay, Refer, Referred};
use task::env::mail::HolePie;
use task::env::task as utask;

/// 控制线程的三枚门闩（主线程写、控制线程读；`Hatch` 是同步点）。
///
/// 同域两线程共享地址空间，但**门闩是 per-task 的**：主线程 `Accord` 出去拿到的是
/// 对方表里的 token，只能经共享内存交接。`Spawn` 恒产 `Held`，故「先 `Accord`、再写
/// 静态、最后 `Hatch`」的次序天然成立——控制线程读到的必然是写好的值。
static CTRL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];

/// 控制线程：接引入请求 → 亲授目录请求门闩 → 回报。不碰注册表。
#[unsafe(no_mangle)]
extern "C" fn control_main() -> ! {
    let entry = HolePie::from_token(CTRL[0].load(Ordering::Relaxed));
    let control = HolePie::from_token(CTRL[1].load(Ordering::Relaxed));
    let up = HolePie::from_token(CTRL[2].load(Ordering::Relaxed));
    loop {
        let refer = match Refer::pull(&control) {
            Ok(r) => r,
            // 控制孔死亡（root 已走）：无处可听，硬失败。
            Err(_) => task::env::control::panic(20),
        };
        let token = entry
            .accord(refer.who().get(), Permission::READ | Permission::WRITE)
            .unwrap_or(0);
        if Referred::new(token).push(&up).is_err() {
            task::env::control::panic(21);
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 靠泊：认父域开的上行孔。
    let up = match handshake::moor() {
        Ok(u) => u,
        Err(_) => task::env::control::panic(1),
    };
    // 2. 自建请求门闩——服务自己开自己的门。
    let entry = match HolePie::unseal(MSG_LEN) {
        Ok(h) => h,
        Err(_) => task::env::control::panic(2),
    };
    // 3. 自建控制孔（只给父域），与请求门闩分离——父域拿不到请求队列。
    let control = match HolePie::unseal(handshake::MTU) {
        Ok(h) => h,
        Err(_) => task::env::control::panic(3),
    };
    let sire = match utask::sire() {
        Ok(t) => t,
        Err(_) => task::env::control::panic(4),
    };
    let at_parent = match control.accord(sire.get(), Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(5),
    };

    // 4. 控制线程：先产（Held，不跑）→ 交接三枚门闩 → 放行。
    let entry_va = control_main as extern "C" fn() -> ! as usize;
    let ctrl = match utask::spawn(TeamId(0), entry_va, &[], 0) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(6),
    };
    let h2 = match entry.accord(
        ctrl.get(),
        Permission::READ | Permission::WRITE | Permission::VEST,
    ) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(7),
    };
    let c2 = match control.accord(ctrl.get(), Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(8),
    };
    let u2 = match up.accord(ctrl.get(), Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(9),
    };
    CTRL[0].store(h2, Ordering::Relaxed);
    CTRL[1].store(c2, Ordering::Relaxed);
    CTRL[2].store(u2, Ordering::Relaxed);
    if utask::hatch(ctrl).is_err() {
        task::env::control::panic(10);
    }

    // 5. 报到：把控制孔在父侧的句柄交给 root（root 据此转达引入请求）。
    if Quay::new(at_parent).push(&up).is_err() {
        task::env::control::panic(11);
    }

    // 6. 服务循环。
    let mut dir = Directory::new();
    let mut msg = [0u8; MSG_LEN];
    loop {
        // 槽空 → 挂起让出 CPU；Err 只在门闩死亡时出现（本域自开的 hole 不会被封印）。
        if entry.pull(&mut msg).is_err() {
            continue;
        }
        let reply_token =
            usize::from_le_bytes(msg[REPLY_AT..REPLY_AT + 8].try_into().unwrap_or([0u8; 8]));
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
