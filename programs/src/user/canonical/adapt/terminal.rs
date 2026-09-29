//! canonical::adapt::terminal — **那一圈（壳）**：读口与写口两边轮转，中间过一遍行规程。
//!
//! ```text
//!   1  收：`rx.pull_timeout(POLL)` 把读口收干净（**这一手同时放 uart 走出"把一批推给我们"那一格**）
//!   2  喂：逐字节进 [`Discipline::feed`]，它把要回显的字节写进本批的 `echo` 缓冲里
//!   3  写：**只在写口就绪时推**（就绪＝槽空＝当场成功）；槽满就短等一拍、再回去收
//!   4  都没事：阻塞等读口（此刻写口是空的，uart 不在等我们）
//! ```
//!
//! **本域只有一条输出路**：行规程的回显。交付的行**不再写出去**——本域就是终端，没有下游；写一遍
//! 只会把每一行在屏幕上显示两次（旧 `echo` 那台是"行回显"，与这一台不是一件事）。
//!
//! **照实记（为什么不是"收一批、写一行"）**：控制台服务那一手（uart 排空之后把一批推给读口）是
//! **阻塞**的（`publish` → `push` 等槽空），而本域这一手（推给写口）在写口满时也阻塞 ⇒ "本域正
//! 写着、uart 正把下一批推给我们"这一格**互相等死**。实测：喂 12 字节，机器稳定停在第 8 字节
//! （uart 每 8 字节左右排一批）。故本域自己轮转，两边的 rendezvous 总能被本域拆开，谁也不硬等谁。

use super::console::Console;
use crate::core::discipline::{Discipline, Step};
use alloc::vec::Vec;
use env::{HoleDir, Wait};
use runtime::PAGE_SIZE;

/// 写口满时的短等一拍（毫秒）。**不是空转**：写口那一格是 uart 还没取走的，睡一拍再复探。
const TICK_MS: usize = 1;

/// 起手先说的一句用法。**本机没有终端回显**（敲的字不会被设备送回来），不说一句就不知道能敲什么。
const USAGE: &str = "canonical: 行规程（ECHO / DEL 退格 / ^U 抹行）；^D 或 exit 收场";

/// 跑到收场词 / EOF。
pub fn run(console: &Console) {
    let Some(mut buf) = page() else { return };
    let mut d = Discipline::new();
    // 待写的消息队列：**一条消息 = 一条完整的字**（回显那几格都在里面）。
    let mut out: Vec<Vec<u8>> = Vec::new();
    push_line(&mut out, USAGE);
    let mut quit = false;

    loop {
        // 1：收——非阻塞地把读口收干净。**这一手同时放 uart 走出"把一批推给我们"那一格**。
        let mut got = false;
        if !quit {
            while let Ok(n) = console.rx.pull_timeout(&mut buf, Wait::POLL) {
                got = true;
                if !eat(&mut d, &buf[..n], &mut out) {
                    quit = true; // 收场词到了：**先把待写的放完**，再走
                    break;
                }
            }
        }
        // 2：写——**只在写口就绪时推**。往写口推的人只有本域 ⇒ 就绪就是槽空 ⇒ 推当场成功。
        if !out.is_empty() {
            match console.tx.wait(HoleDir::Push, Wait::POLL) {
                Ok(true) => {
                    // 借到这一条就结束（不把 `out` 的借带进下面的 `remove`）。
                    let failed = console.tx.push(&out[0]).is_err();
                    if failed {
                        return; // 写口封了 = 持设备的域没了
                    }
                    out.remove(0);
                }
                // 槽里压着上一条（uart 还没取走）：短等一拍再来，不空转也不硬等。
                _ => {
                    let _ = console.tx.wait(HoleDir::Push, Wait::AtMost(TICK_MS));
                }
            }
            continue;
        }
        // 3：收场词到了、队列也空了 ⇒ 走。
        if quit {
            return;
        }
        // 4：没得写也没收到：阻塞等读口（**此刻写口是空的，uart 不在等我们**）。
        if got {
            continue;
        }
        let Ok(n) = console.rx.pull(&mut buf) else {
            return;
        };
        if !eat(&mut d, &buf[..n], &mut out) {
            quit = true;
        }
    }
}

/// 吃一批：逐字节过行规程；行尾那一格把整行交出去（**本域没有下游**，只有收场词与 EOF 有下文）。
/// 返 `false` = 收场。
fn eat(d: &mut Discipline, bytes: &[u8], out: &mut Vec<Vec<u8>>) -> bool {
    // 这一批要回显的字节攒在这里：一并在"行尾那一格"或"本批末尾"推出去。
    let mut echo: Vec<u8> = Vec::new();
    for &b in bytes {
        match d.feed(b, &mut echo) {
            Step::More => {}
            // 行尾那一格：**先把这一段的回显冲出去**（含那个回车），行本身交出去（本域不看它）。
            Step::Line => {
                if !echo.is_empty() {
                    out.push(core::mem::take(&mut echo));
                }
            }
            // 收场词：先把回显冲完，再走。
            Step::Exit => {
                if !echo.is_empty() {
                    out.push(core::mem::take(&mut echo));
                }
                return false;
            }
            // `^D` 落在空行上：输入到此为止（本批剩下的字节不再看）。**补一个换行**，免得最后
            // 一行与下一个输出黏在一起（有意的可读性偏差，不是 termios 的行为）。
            Step::Eof => {
                echo.extend_from_slice(b"\r\n");
                out.push(echo);
                return false;
            }
        }
    }
    if !echo.is_empty() {
        out.push(echo);
    }
    true
}

/// 一条"完整的字"进待写队列（**一次写 = 一条完整的字**；换行本域补，空行因此也发得出去）。
fn push_line(out: &mut Vec<Vec<u8>>, line: &str) {
    let mut one = Vec::with_capacity(line.len() + 1);
    one.extend_from_slice(line.as_bytes());
    one.push(b'\n');
    out.push(one);
}

/// 一页缓冲（**载体的界**：`envcall` 把一条消息卡在 `1..=一页`）。
fn page() -> Option<Vec<u8>> {
    let mut v: Vec<u8> = Vec::new();
    v.try_reserve_exact(PAGE_SIZE).ok()?;
    v.resize(PAGE_SIZE, 0);
    Some(v)
}
