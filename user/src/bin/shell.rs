#![no_std]
#![no_main]

//! shell — 命令解释器，经 Terminal（term 模块）显示、读命令并分发给系统能力。
//!
//! 分层：本 bin 是 Shell；`user::term::{Terminal, Readline}` 是渲壳 + 行编辑宿主。
//! Shell 经 `Terminal::put` 输出、`Terminal::readline` 读输入，不再自建行编辑。
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
//!   exit  — 退出 shell（RoomCall::Reap）

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use core::time::Duration;

use user::core::task;
use user::env::chrono::{self, clock};
use user::env::mail::HolePie;
use user::env::room::{self, sleep};
use user::term::{Color, Readline, Terminal};

/// 打印（消费 EnvResult，免 must_use 警告）。
fn put(s: &str) {
    let _ = user::env::io::put(s);
}

/// 按空白切词（保留空输入 = 空 Vec）。
fn split(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

/// 各系统能力命令。
fn exec(cmd: &str, args: &[String]) -> bool {
    // 返回 false = 退出（exit 命令）。
    match cmd {
        "help" => {
            put("help / clock / ticks / alloc / echo / sleep / spawn / hole / exit\n");
        }
        "clock" => {
            let (s, n) = clock().unwrap_or((0, 0));
            put(&format!("clock {s}.{:09} sec\n", n));
        }
        "ticks" => {
            let t = chrono::ticks().unwrap_or(0);
            put(&format!("ticks {t}\n"));
        }
        "alloc" => {
            let addr = user::env::memory::allocate(4096).unwrap_or(0);
            put(&format!("alloc -> {addr:#x}\n"));
        }
        "echo" => {
            put(&args.join(" "));
            put("\n");
        }
        "sleep" => {
            let ms = args.first().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            put(&format!("sleep {ms}ms\n"));
            let _ = sleep(Duration::from_millis(ms));
            put("woke\n");
        }
        "spawn" => {
            let n = args.first().and_then(|s| s.parse::<u64>().ok()).unwrap_or(1000);
            let sum = task::closure(move || {
                let mut acc: u64 = 0;
                for i in 0..n {
                    acc = acc.wrapping_add(i);
                }
                acc
            })
            .join();
            put(&format!("spawnjoin -> {sum}\n"));
        }
        "hole" => {
            let msg = b"hi from shell";
            let pie = HolePie::unseal().unwrap();
            let mut buf = [0u8; 64];
            let mut m = [0u8; 64];
            m[..msg.len()].copy_from_slice(msg);
            pie.push(&m).ok();
            pie.pull(&mut buf).ok();
            put(&format!("hole got {:?}\n", core::str::from_utf8(&buf).unwrap_or("?")));
            pie.seal().ok();
        }
        "exit" => {
            put("bye\n");
            return false;
        }
        _ => {
            put(&format!("unknown: {cmd} (try help)\n"));
        }
    }
    true
}

#[unsafe(no_mangle)]
extern "C" fn main() {
    let term = Terminal::default();
    term.clear();
    term.fg(Color::Green);
    put("SQware shell\n");
    term.reset();
    put("type 'help' for commands.\n");

    loop {
        term.fg(Color::Cyan);
        put("sq > ");
        term.reset();
        let line = match term.readline() {
            Readline::Line(s) => s,
            Readline::Eof | Readline::Interrupt => continue,
        };
        let args = split(&line);
        if args.is_empty() {
            continue;
        }
        let cmd = args[0].clone();
        let rest = &args[1..];
        if !exec(&cmd, rest) {
            break;
        }
    }

    room::exit()
}
