#![no_std]
#![no_main]
//! root — 根服务域：产生系统里的**所有**域，并作为唯一的根授予源头。
//!
//! 内核 boot 只装它一个（`boot::spawn_root`）；此后 shell / echo / dir 全部由本域
//! 产生，权限也由本域分发。**退出即关机**：本域退出 ⇒ `doom` 级联扑杀全部子域
//! ⇒ 全部任务回收 ⇒ `conductor::done` 自然停机（srst）。无需外部 timeout。
//!
//! 流程：
//!   1. 启动参数 = [清单视图 VA, 清单长度]（boot 只读映射的 initrd 区）；
//!   2. 解析清单（`manifest`），跳过自己；
//!   3. 建目录请求 hole（root 自持）→ 建 dir 域 → 给 dir 自己的请求门闩 → 放行；
//!   4. 建 echo 入口 hole → 建 echo 域 → 给它入口门闩（带 VEST）+ 目录门闩 → 放行；
//!   5. 建 shell 域 → 给目录门闩，启动参数带**目录 task id** → 放行；
//!   6. `Join(shell)`：用户会话结束 → 本域退出 → 级联 → 停机。
//!
//! 「目录 task id 经启动参数告知」取代了旧 boot 期的 `vestor` 内标（那是内核侧
//! 特权，用户态 `Accord` 只会把授与人写成 vestor）——见 `docs/root.md`。

extern crate alloc;

use env::{Permission, TaskId, TeamId};
use task::env::mail::HolePie;
use task::env::task as utask;
use task::env::{control::panic, io::put, room::exit};

mod manifest;

/// 目录请求 hole 的 mtu（= dispatch `MSG_LEN`）。
const MSG_LEN: usize = 64;

fn say(s: &str) {
    let _ = put(s);
}

/// 清单里按名取程序 → `Build` 装域 → `Spawn` 产**未放行**的引导线程。
fn build_spawn(entries: &[manifest::Entry<'_>], name: &str, args: &[usize]) -> TaskId {
    let Some(e) = entries.iter().find(|e| e.name == name) else {
        say("root: missing program ");
        say(name);
        say("\n");
        panic(1);
    };
    let team: TeamId = match utask::build(e.elf, e.kind, e.name) {
        Ok(t) => t,
        Err(_) => panic(2),
    };
    match utask::spawn(team, 0, args, 0) {
        Ok(t) => t,
        Err(_) => panic(3),
    }
}

/// 等目标回收（`Join` 的复探模式：挂起过的那一次只当「醒了一次」）。
fn wait_dead(task: TaskId) {
    loop {
        if utask::join(task, 0).unwrap_or(true) {
            return;
        }
        let _ = utask::join(task, usize::MAX);
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 清单视图（boot 只读映射的 initrd 区；长度经启动参数告知）
    let args = utask::args();
    let (view, len) = match args {
        [va, len, ..] => (*va as *const u8, *len),
        _ => {
            say("root: no manifest args\n");
            panic(4);
        }
    };
    // SAFETY: boot 把 initrd 区只读映射到该 VA，长度即 region.size。
    let blob = unsafe { core::slice::from_raw_parts(view, len) };
    let entries = match manifest::parse(blob) {
        Some(e) => e,
        None => {
            say("root: malformed manifest\n");
            panic(5);
        }
    };

    // 2. 目录请求 hole：root 自持——唯一的根授予源头
    let dir_req = match HolePie::unseal(MSG_LEN) {
        Ok(h) => h,
        Err(_) => panic(6),
    };

    // 3. dir 域：先建（clients 需要它的 task id），给它自己的请求门闩（索引 0）
    let dir_task = build_spawn(&entries, "dir", &[]);
    if dir_req
        .accord(dir_task.get(), Permission::READ | Permission::WRITE)
        .is_err()
    {
        panic(7);
    }
    if utask::hatch(dir_task).is_err() {
        panic(8);
    }

    // 4. echo 域：入口 hole（root 建）→ 自己带 VEST 的副本（索引 0）+ 目录门闩（索引 1）
    let echo_entry = match HolePie::unseal(MSG_LEN) {
        Ok(h) => h,
        Err(_) => panic(9),
    };
    let echo_task = build_spawn(&entries, "echo", &[dir_task.get()]);
    if echo_entry
        .accord(
            echo_task.get(),
            Permission::READ | Permission::WRITE | Permission::VEST,
        )
        .is_err()
    {
        panic(10);
    }
    if dir_req
        .accord(echo_task.get(), Permission::READ | Permission::WRITE)
        .is_err()
    {
        panic(11);
    }
    if utask::hatch(echo_task).is_err() {
        panic(12);
    }

    // 5. shell 域：目录门闩（索引 0）；启动参数 [目录 task id]
    let shell_task = build_spawn(&entries, "shell", &[dir_task.get()]);
    if dir_req
        .accord(shell_task.get(), Permission::READ | Permission::WRITE)
        .is_err()
    {
        panic(13);
    }
    if utask::hatch(shell_task).is_err() {
        panic(14);
    }

    // 6. 用户会话结束（shell 退出或崩溃）→ 本域退出 → doom 级联 → 全部回收 → 停机
    wait_dead(shell_task);
    say("root: session over, shutting down\n");
    exit()
}
