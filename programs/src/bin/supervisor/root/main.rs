#![no_std]
#![no_main]

//! root — 装配者：产生域，并在用户会话结束后收场。
//!
//! 这是**清理后的最小装配**：系统里只有一个子域（`echo`），而且它启动时
//! **什么都不用配**——`echo` 走 `env` 的调试面，不依赖服务、不依赖设备、不依赖别的域。
//! 于是本域只剩四件事：
//!
//! ```text
//! 1  启动参数 → 清单视图（boot 只读借映的 initrd 区）→ 找到 echo
//! 2  解封建域权（`UnsealNole` 有 S 态门，故只有 root 持有）
//! 3  Build + Spawn + Hatch（Spawn 产的是**未放行**的引导线程）
//! 4  等它退场 → 本域退出 ⇒ doom 级联 ⇒ 全部回收 ⇒ 自然停机（srst）
//! ```
//!
//! **退出即关机**，故门的两条硬判据（自行退出 + 无 panic）照旧成立，不需要外接 timeout。
//!
//! # 这里**故意**没有的东西
//!
//! 设备直连打印 · 门铃自检 · 目录 · 驱动接线与门闩转授 · 监视/重发/他杀 —— 它们都属于
//! "服务"那一层，而这一版没有服务。清单解析仍在 `manifest`（格式由 `kernel/build.rs`
//! 打包，内核不解释它）。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use env::TaskId;
use runtime::env::mail::NolePie;
use runtime::env::room::exit_with;
use runtime::env::task as utask;

mod manifest;

/// 本域唯一的子域。
const ECHO: &str = "echo";

/// 失败编号：指"死在装配的哪一步"（沿用旧树那套小整数编号的意思）。
const E_ARGS: usize = 1;
const E_MANIFEST: usize = 2;
const E_PROGRAM: usize = 3;
const E_BUILD_RIGHT: usize = 4;
const E_BUILD: usize = 5;
const E_SPAWN: usize = 6;
const E_HATCH: usize = 7;
const E_EXIT: usize = 0;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 清单：启动参数 = [清单视图 VA, 清单长度, …]；boot 把 initrd 区只读借映进本域。
    let (view, len) = match utask::args() {
        [va, len, ..] => (*va as *const u8, *len),
        _ => exit_with(E_ARGS),
    };
    // SAFETY: boot 把 initrd 区只读映射到该 VA，长度即 region.size；本域只读。
    let blob = unsafe { core::slice::from_raw_parts(view, len) };
    let Some(entries) = manifest::parse(blob) else {
        exit_with(E_MANIFEST);
    };
    let Some(program) = entries.iter().find(|e| e.name == ECHO) else {
        exit_with(E_PROGRAM);
    };

    // 2. 建域权：本域是唯一持有者，此后一直用它。
    let Ok(build) = NolePie::unseal() else {
        exit_with(E_BUILD_RIGHT);
    };

    // 3. 装域 → 产引导线程 → 放行。
    let Ok(team) = utask::build(program.elf, program.kind, program.name, &build) else {
        exit_with(E_BUILD);
    };
    let Ok(child) = utask::spawn(team, 0, &[], 0) else {
        exit_with(E_SPAWN);
    };
    if utask::hatch(child).is_err() {
        exit_with(E_HATCH);
    }

    // 4. 等它走。本域退出 ⇒ 级联扑杀 ⇒ 全部任务回收 ⇒ 停机。
    join_done(child);
    exit_with(E_EXIT)
}

/// 等目标结束。`join(tid, 0)` 返真 ⇒ **收尾已完成**（内核契约：退出钩子已跑完），
/// 故挂起过的那一次只当"醒了一次"，须复探。
fn join_done(tid: TaskId) {
    loop {
        if utask::join(tid, 0).unwrap_or(true) {
            return;
        }
        let _ = utask::join(tid, usize::MAX);
    }
}
