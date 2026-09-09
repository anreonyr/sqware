#![no_std]
#![no_main]

//! shell — 命令解释器，经 Terminal（term 模块）显示、读命令并分发给系统能力。
//!
//! 分层：本 bin 是 Shell；`task::term::{Terminal, Readline}` 是渲壳 + 行编辑宿主。
//! **Terminal 是唯一 console 出口**——Shell 的一切输出经 `Terminal::put`、一切
//! 输入经 `Terminal::readline`，不再直接 `io::put`。
//!
//! 命令（系统能力巡演）：
//!   help  — 列命令
//!   clock — 读全局时钟（ChronoCall::Clock）
//!   ticks — 读 timebase 刻度
//!   alloc — 堆分配一页（MemoryCall::Allocate）
//!   echo  — 回显参数
//!   sleep — 阻塞 N 毫秒（RoomCall::Park）
//!   spawn — 派一个算 0..N 的闭包子任务并 join（TaskCall::Spawn）
//!   hole  — Hole 通道自测（unseal/push/pull/seal）
//!   req   — 走目录协议连接 echo 并调用一次（Connect + Service::call）
//!   dir   — 目录协议自省（Discover + Enumerate）
//!   exit  — 退出 shell（RoomCall::Reap）

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use core::time::Duration;

use task::core::service::{Directory, PAYLOAD_LEN};
use task::core::unit;
use task::env::{
    chrono::{self, clock},
    mail::{HolePie, HOLE_MTU_MAX},
    room::{self, sleep},
    task::{heir_at, heir_count},
};
use task::term::{Color, Readline, Terminal};

/// 按空白切词（保留空输入 = 空 Vec）。
fn split(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

/// 各系统能力命令。全部输出经 `term`（唯一 console 出口）。
/// 返回 false = 退出（exit 命令）。
fn exec(cmd: &str, args: &[String], term: &Terminal) -> bool {
    match cmd {
        "help" => {
            term.writeline(
                "help / clock / ticks / alloc / echo / sleep / spawn / heir / hole / req / dir / exit",
            );
        }
        "clock" => {
            let (s, n) = clock().unwrap_or((0, 0));
            term.writeline(&format!("clock {s}.{:09} sec", n));
        }
        "ticks" => {
            let t = chrono::ticks().unwrap_or(0);
            term.writeline(&format!("ticks {t}"));
        }
        "alloc" => {
            let addr = task::env::memory::allocate(4096).unwrap_or(0);
            term.writeline(&format!("alloc -> {addr:#x}"));
        }
        "echo" => {
            term.writeline(&args.join(" "));
        }
        "sleep" => {
            let ms = args
                .first()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            term.writeline(&format!("sleep {ms}ms"));
            let _ = sleep(Duration::from_millis(ms));
            term.writeline("woke");
        }
        "spawn" => {
            // 闭包 join：算 0..N。
            let n = args
                .first()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1000);
            let sum = unit::closure(move || {
                let mut acc: u64 = 0;
                for i in 0..n {
                    acc = acc.wrapping_add(i);
                }
                acc
            })
            .join();
            term.writeline(&format!("spawnjoin -> {sum}"));
        }
        "heir" => {
            // 血缘枚举：我生的子域（heir）——先 count 再逐个取 TeamId。
            let n = heir_count().unwrap_or(0);
            if n == 0 {
                term.writeline("heir: none");
            } else {
                term.writeline(&format!("heir: {n} children"));
                for i in 0..n {
                    let tid = heir_at(i).unwrap_or(env::TeamId::new(0));
                    term.writeline(&format!("  heir[{i}] = team {tid:?}"));
                }
            }
        }
        "hole" => {
            let msg = b"hi from shell";
            let pie = HolePie::unseal(HOLE_MTU_MAX).unwrap();
            let mut buf = [0u8; 64];
            let mut m = [0u8; 64];
            m[..msg.len()].copy_from_slice(msg);
            pie.push(&m).ok();
            pie.pull(&mut buf).ok();
            term.writeline(&format!(
                "hole got {:?}",
                core::str::from_utf8(&buf).unwrap_or("?")
            ));
            pie.seal().ok();
        }
        "req" => {
            // 走目录协议：Directory::open 取会话 → Connect("echo") 拿服务入口门闩
            // → Service::call 一次往返（echo 对载荷字节 +1）。
            let dir = match Directory::open() {
                Ok(d) => d,
                Err(e) => {
                    term.writeline(&format!("req dir err: {e:?}"));
                    return true;
                }
            };
            let svc = match dir.connect("echo") {
                Ok(s) => s,
                Err(e) => {
                    term.writeline(&format!("req connect err: {e:?}"));
                    return true;
                }
            };
            let txt = args.first().map(|s| s.as_str()).unwrap_or("hello-service");
            let mut payload = [0u8; PAYLOAD_LEN];
            let bytes = txt.as_bytes();
            let n = bytes.len().min(PAYLOAD_LEN - 1);
            payload[..n].copy_from_slice(&bytes[..n]);
            match svc.call(&payload) {
                Ok(got) => {
                    term.writeline(&format!(
                        "req echo -> {:?}",
                        core::str::from_utf8(&got).unwrap_or("?")
                    ));
                }
                Err(e) => {
                    term.writeline(&format!("req echo err: {e:?}"));
                }
            }
            let _ = svc.disconnect();
        }
        "dir" => {
            // 目录协议：Discover（纯探测）+ Enumerate（按名排序分页）。
            let dir = match Directory::open() {
                Ok(d) => d,
                Err(e) => {
                    term.writeline(&format!("dir open err: {e:?}"));
                    return true;
                }
            };
            match dir.discover("echo") {
                Ok(true) => term.writeline("discover echo -> found"),
                Ok(false) => term.writeline("discover echo -> not found"),
                Err(e) => term.writeline(&format!("dir discover err: {e:?}")),
            }
            let mut after: Option<String> = None;
            loop {
                match dir.list(after.as_deref()) {
                    Ok(Some(name)) => {
                        term.writeline(&format!("  {}", name.as_str()));
                        after = Some(String::from(name.as_str()));
                    }
                    Ok(None) => break,
                    Err(e) => {
                        term.writeline(&format!("dir list err: {e:?}"));
                        break;
                    }
                }
            }
        }
        "exit" => {
            term.writeline("bye");
            return false;
        }
        _ => {
            term.writeline(&format!("unknown: {cmd} (try help)"));
        }
    }
    true
}

#[unsafe(no_mangle)]
extern "C" fn main() {
    let term = Terminal::default();
    term.clear();
    term.fg(Color::Green);
    term.writeline("SQware shell");
    term.reset();
    term.writeline("type 'help' for commands.");

    loop {
        term.fg(Color::Cyan);
        // readline 收纳 prompt：Terminal 内部打 prompt + 行编辑 + 清行重绘含 prompt。
        let line = match term.readline("sq > ") {
            Readline::Line(s) => s,
            Readline::Eof | Readline::Interrupt => {
                term.reset();
                continue;
            }
        };
        term.reset();
        let args = split(&line);
        if args.is_empty() {
            continue;
        }
        let cmd = args[0].clone();
        let rest = &args[1..];
        if !exec(&cmd, rest, &term) {
            break;
        }
    }

    room::exit()
}
