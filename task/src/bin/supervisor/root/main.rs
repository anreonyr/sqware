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
//!   3. 开一条**报到孔**（所有子域共用）；逐子域串行握手：
//!      `Build` + `Spawn`(Held) → 报到孔副本 `Accord` 给它 → `Hatch` → 收 `Quay`
//!      （子域自建孔在父侧的句柄，校验「授与人 = 该子域」）→ 往那条孔 push `Pier`；
//!      - dir 在 `Quay` 里交出的就是它的请求门闩（root 留作分发源，故它要带 VEST）；
//!      - 客户端在 `Pier` 里收到目录请求门闩的句柄，目录身份由 `Owned` 从门闩自身的
//!        `owner` 求得——**报文里没有任何整数身份**（`docs/root.md`）；
//!   4. `Join(shell)`：用户会话结束 → 本域退出 → 级联 → 停机。

extern crate alloc;

use env::{Permission, TaskId, TeamId};
use task::core::handshake::{self, Pier, Quay};
use task::env::mail::{self, HolePie};
use task::env::task as utask;
use task::env::{control::panic, io::put, room::exit};

mod manifest;

fn say(s: &str) {
    let _ = put(s);
}

/// 清单里按名取程序 → `Build` 装域 → `Spawn` 产**未放行**的引导线程（启动参数为空）。
fn build_spawn(entries: &[manifest::Entry<'_>], name: &str) -> TaskId {
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
    match utask::spawn(team, 0, &[], 0) {
        Ok(t) => t,
        Err(_) => panic(3),
    }
}

/// 串行握手：给子域一份报到孔副本 → 放行 → 收报到（校验授与人）→ 往它自建的孔里
/// 配给。返子域自建孔在**父侧**的句柄。
///
/// `pier` = 本次配给的载荷（0 = 无）。
fn shake(quay: &HolePie, child: TaskId, pier: u64) -> u64 {
    if quay
        .accord(child.get(), Permission::READ | Permission::WRITE)
        .is_err()
    {
        panic(4);
    }
    if utask::hatch(child).is_err() {
        panic(5);
    }
    let reported = match Quay::pull(quay) {
        Ok(q) => q,
        Err(_) => panic(6),
    };
    // 自证：这枚孔必须真是该子域授出来的（vestor 只能由授出者写）。
    let vestor = match mail::owned(reported.hole()) {
        Ok((vestor, _owner)) => vestor.get(),
        Err(_) => panic(7),
    };
    if vestor != child.get() {
        say("root: report not from child\n");
        panic(8);
    }
    if Pier::new(pier)
        .push(&HolePie::from_token(reported.hole()))
        .is_err()
    {
        panic(9);
    }
    reported.hole()
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
            panic(10);
        }
    };
    // SAFETY: boot 把 initrd 区只读映射到该 VA，长度即 region.size。
    let blob = unsafe { core::slice::from_raw_parts(view, len) };
    let entries = match manifest::parse(blob) {
        Some(e) => e,
        None => {
            say("root: malformed manifest\n");
            panic(11);
        }
    };

    // 2. 报到孔：一条，所有子域共用（串行握手，故无争用）
    let quay = match handshake::dock() {
        Ok(q) => q,
        Err(_) => panic(12),
    };

    // 3. dir：先建（客户端要它的门闩），它的请求门闩即 root 的分发源
    let dir_task = build_spawn(&entries, "dir");
    let dir_hole = HolePie::from_token(shake(&quay, dir_task, 0));

    // 4. echo / shell：配给「目录请求门闩」的 R|W 副本
    let mut shell_task = TaskId(0);
    for name in ["echo", "shell"] {
        let child = build_spawn(&entries, name);
        let token = match dir_hole.accord(child.get(), Permission::READ | Permission::WRITE) {
            Ok(t) => t,
            Err(_) => panic(13),
        };
        shake(&quay, child, token);
        if name == "shell" {
            shell_task = child;
        }
    }

    // 5. 用户会话结束（shell 退出或崩溃）→ 本域退出 → doom 级联 → 全部回收 → 停机
    wait_dead(shell_task);
    say("root: session over, shutting down\n");
    exit()
}
