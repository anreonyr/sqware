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
//!   3. **串行握手**（每个子域一次往返）：
//!      `Build` + `Spawn`(Held) → `dock`（开上行孔并授副本）→ `Hatch` → 收 `Quay`
//!      （子域自建控制孔在父侧的句柄，校验「授与人 = 该子域」）→ 往那条孔 push `Pier`；
//!   4. 客户端的目录能力由 **dir 亲授**：`Refer{who, name}` 转达 → `Referred{token}`
//!      收结果 → `Pier{token}` 配给。**名字也在此预约**——目录的名字空间由本域
//!      播种，子域只能注册预约给它的名字（见 `docs/dispatch.md`）。本域手里没有
//!      任何服务孔（见 `docs/root.md`）。
//!   5. `Join(shell)`：用户会话结束 → 本域退出 → 级联 → 停机。

extern crate alloc;

use env::{Name, TaskId, TeamId};
use task::core::handshake::{self, Pier, Quay, Refer, Referred};
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

/// 开上行孔（`dock`）→ 放行。返上行孔（root 侧）。
fn launch(child: TaskId) -> HolePie {
    let up = match handshake::dock(child) {
        Ok(u) => u,
        Err(_) => panic(4),
    };
    if utask::hatch(child).is_err() {
        panic(5);
    }
    up
}

/// 收报到：自证「这枚控制孔真是该子域授出来的」，返它在 root 侧的句柄。
fn report(child: TaskId, up: &HolePie) -> HolePie {
    let quay = match Quay::pull(up) {
        Ok(q) => q,
        Err(_) => panic(6),
    };
    let vestor = match mail::owned(quay.hole()) {
        Ok((vestor, _owner)) => vestor.get(),
        Err(_) => panic(7),
    };
    if vestor != child.get() {
        say("root: report not from child\n");
        panic(8);
    }
    HolePie::from_token(quay.hole())
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
            panic(9);
        }
    };
    // SAFETY: boot 把 initrd 区只读映射到该 VA，长度即 region.size。
    let blob = unsafe { core::slice::from_raw_parts(view, len) };
    let entries = match manifest::parse(blob) {
        Some(e) => e,
        None => {
            say("root: malformed manifest\n");
            panic(10);
        }
    };

    // 2. dir：先建（客户端要它的门闩）。它的控制孔即后续引入请求的通道。
    let dir_task = build_spawn(&entries, "dir");
    let up_dir = launch(dir_task);
    let control_dir = report(dir_task, &up_dir);

    // 3. echo / shell：请 dir 亲授目录请求门闩的 R|W 副本，再配给客户端；
    //    同时把名字预约给该子域（目录只接受预约者的注册）。
    let mut shell_task = TaskId(0);
    for name in ["echo", "shell"] {
        let child = build_spawn(&entries, name);
        let up = launch(child);
        let down = report(child, &up);
        let refer = match Name::new(name) {
            Ok(n) => Refer::named(child, n),
            Err(_) => panic(15),
        };
        if refer.push(&control_dir).is_err() {
            panic(11);
        }
        let referred = match Referred::pull(&up_dir) {
            Ok(r) => r,
            Err(_) => panic(12),
        };
        if referred.token() == 0 {
            say("root: directory refused to grant\n");
            panic(13);
        }
        if Pier::new(referred.token()).push(&down).is_err() {
            panic(14);
        }
        if name == "shell" {
            shell_task = child;
        }
    }

    // 4. 用户会话结束（shell 退出或崩溃）→ 本域退出 → doom 级联 → 全部回收 → 停机
    wait_dead(shell_task);
    say("root: session over, shutting down\n");
    exit()
}
