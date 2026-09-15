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
//! # 两条边界（设计红线）
//!
//! 1. **只有一个域读调试面**：读口只有一个，谁 `get` 谁把它拿走。
//! 2. **一次写必须是一条完整的字**；写同一个设备的域不止一个时，混着写就会互相插字
//!    （旧树里 root 的设备直连写与服务的写就是这么插花的：`root: conssoll: coole nsole…`）。
//!
//! 非 UTF-8 的行会在内核那一格折成 `<non-utf8>`（`env::fid::DebugCall` 的既有语义：
//! 那一段要过 `str`）。调试回显面对的是一台终端，够用——不为它动冻结面。

extern crate programs;

use env::DBCN_MAX;
use runtime::env::debug;
use runtime::env::room::exit_with;

/// 域自己的正常退场码（与 `programs::entry::EXIT_OK` 同号）。
const EXIT_OK: usize = 0;

/// 一行的上界。更长的行**截断**回显（超过它的行不可能是 `exit`，故收场判据不受影响）。
const LINE_MAX: usize = 128;

const READY: &str = "echo: ready";
const NON_UTF8: &str = "<non-utf8>";

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let _ = debug::put(READY);

    let mut buf = [0u8; DBCN_MAX];
    let mut line = [0u8; LINE_MAX];
    let mut n_line = 0usize;

    'echo: loop {
        // 一次 `get` 给多少字节不定（固件的语义是"至少一个"）。
        let Ok(n) = debug::get(&mut buf) else { break };
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
