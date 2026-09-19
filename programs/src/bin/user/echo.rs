#![no_std]
#![no_main]

//! echo — **调试回显**：把调试面读到的一行原样写回去。
//!
//! 它不要服务、不要驱动、不要孔、不要配给：走 `env` 的调试面（`DebugCall`，class 8），
//! 内核把固件的调试控制台直接借给域。**"域能说一句话"不依赖任何别的域**——这就是新
//! 基线的第一件东西，也是它全部的用途：让任何域在服务还不存在时能说话、能收话。
//!
//! # 为什么按**行**回显，而不是读到几个字节就写几个
//!
//! 内核那一格是行语义（`putln!`）：**一次 `put` 就是一行**。按字节回显会得到
//! `h\ne\nl\nl\no\n`——不是这台机器的怪癖，是"一次写必须是一条完整的字"这条红线在
//! 地板上的形态。读入侧则相反：`get` 一次只给一个字节也很正常，故本程序**攒够一行**
//! 再写。
//!
//! # 收场
//!
//! 读到一行 `exit` 就退（域退场 ⇒ root 收场 ⇒ 停机）。门的"自退"判据靠它。
//!
//! # 为什么它也上板（`board: true`）
//!
//! 与 `passer` 同理、用途不同：**本域要让人看得出它死了**。本域退场时开的那几枚孔随退出
//! 钩子封印 ⇒ 板当场看出"客人没了" ⇒ 往死亡通知那条路推一格 ⇒ 装配者（`root`）据此记账
//! 并放下本域那个死域。本域是**最后一条**，故这一格同时就是"会话结束"的信号。
//!
//! 挂不上板也照旧回显（本域的主职是回显）——只是那条信号缺席。
//!
//! # 两条边界（设计红线）
//!
//! 1. **只有一个域读调试面**：读口只有一个，谁 `get` 谁把它拿走。
//! 2. **一次写必须是一条完整的字**；写同一个设备的域不止一个时，混着写就会互相插字
//!    （旧树里 root 的设备直连写与服务的写就是这么插花的：`root: conssoll: coole nsole…`）。
//!
//! 非 UTF-8 的行会在内核那一格折成 `<non-utf8>`（`env::fid::DebugCall` 的既有语义：
//! 那一段要过 `str`）。调试回显面对的是一台终端，够用——不为它动冻结面。

extern crate alloc;
extern crate programs;

// 共享物住在 supervisor 目录里，由各 bin 各自声明一次（见 `needs.rs` 头注）。
#[path = "../supervisor/board.rs"]
// 本域只用**客侧**那几手（板侧那一半归 root）⇒ 另一半在这里是死码。
#[allow(dead_code)]
mod board;

use alloc::format;
use core::time::Duration;

use env::DBCN_MAX;
use env::Name;
use protocol::board::call as bcall;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::room::{self, exit_with};
use runtime::env::unit as utask;

/// 本域挂在板上的名字（板按它分人；`root` 表里那一条也叫这个）。
const ME: &str = "echo";

/// 等板的总上限（毫秒）。**必须有界**：板死在头几步时本域不能陪着挂死（挂不上照旧回显）。
const MS: usize = 1000;

/// 域自己的正常退场码（与 `programs::entry::EXIT_OK` 同号）。
const EXIT_OK: usize = 0;

/// 一行的上界。更长的行**截断**回显（超过它的行不可能是 `exit`，故收场判据不受影响）。
const LINE_MAX: usize = 128;

/// 没字节可读时的重问间隔（毫秒）。见主循环那条"空转的红线"。
const IDLE_MS: u64 = 1;

const READY: &str = "echo: ready";
const NON_UTF8: &str = "<non-utf8>";

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let _ = debug::put(READY);
    // 上板：**注册在回显之前**——板要能看见本域（见头注）。挂不上照旧回显。
    let reg = register();
    let _ = debug::put(&format!("echo: reg={reg}"));

    let mut buf = [0u8; DBCN_MAX];
    let mut line = [0u8; LINE_MAX];
    let mut n_line = 0usize;

    'echo: loop {
        // 一次 `get` 给多少字节不定，**也可能一个字节都没有**——固件那一格**不阻塞**
        // （内核 `envcall/debug.rs::get` 的照实记：旧注写"至少一个"，实测是当场返 0）。
        let Ok(n) = debug::get(&mut buf) else { break };
        // **空转的红线**：固件一个字节都没给（`n == 0`）时不能立刻再问——那是把一整颗核
        // 烧在等键上（实测宿主 99%）。睡一毫秒再来，输入早到晚到一毫秒无所谓。
        if n == 0 {
            let _ = room::sleep(Duration::from_millis(IDLE_MS));
            continue;
        }
        let Some(chunk) = buf.get(..n) else { break };

        for &b in chunk {
            match b {
                b'\n' | b'\r' => {
                    let text = &line[..n_line];
                    if text == &b"exit"[..] {
                        break 'echo;
                    }
                    let _ = debug::put(core::str::from_utf8(text).unwrap_or(NON_UTF8));
                    n_line = 0;
                }
                // 行太长：多的字节丢掉（截断回显），但行尾判据照旧。
                _ => {
                    if let Some(slot) = line.get_mut(n_line) {
                        *slot = b;
                        n_line += 1;
                    }
                }
            }
        }
    }

    exit_with(EXIT_OK)
}

/// 上板报到（与 `passer` 同一段前奏）：返板的答码（`bcall::OK` = 挂上了）。
///
/// 这一挂的唯一用途是让**板看得见本域的死**；挂不上就返 `BAD`，主职照旧。
fn register() -> u8 {
    let Ok(sire) = utask::sire() else {
        return bcall::BAD;
    };
    let Ok((link, board)) = board::open(sire, MS) else {
        return bcall::BAD;
    };
    let Ok(talk) = board::ask_hole(board) else {
        return bcall::BAD;
    };
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        return bcall::BAD;
    };
    let Ok(me) = Name::new(ME) else {
        return bcall::BAD;
    };
    board::ask(talk, &link, board, bcall::REGISTER, me, entry, MS).unwrap_or(bcall::BAD)
}
