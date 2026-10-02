//! canonical::adapt::terminal — **那一圈（壳）**：读口与写口两边轮转，中间过一遍行规程。
//! ```text
//!   1  收：`rx.pull(buf, POLL)` 把读口收干净（**这一手同时放 uart 走出"把一批推给我们"那一格**）
//!   2  喂：逐字节进 [`Discipline::feed`]，它把要回显的字节写进本批的 `echo` 缓冲里
//!   3  写：**只在写口就绪时推**（就绪＝槽空＝当场成功）；槽满就短等一拍、再回去收
//!   4  都没事：阻塞等读口（此刻写口是空的，uart 不在等我们）
//! ```
//! **本域只有一条输出路**：行规程的回显。交付的行**不再写出去**——本域就是终端，没有下游；写一遍

use super::console::Console;
use crate::core::discipline::{Discipline, Step};
use alloc::vec::Vec;
use env::{HoleDir, Wait};
use runtime::PAGE_SIZE;

/// 写口满时的短等一拍（毫秒）。**不是空转**：写口那一格是 uart 还没取走的，睡一拍再复探。
const TICK_MS: usize = 1;

/// **等"这一条被取走"的期限**（毫秒）：等不到就回去收读口（uart 正等本域收它那一批）。
/// 一拍足够——对方那一圈就在毫秒级；而**没有它就是一个死环**（见写口那一节的注）。
const TAKEN_MS: usize = 1;

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
    // **`out[0]` 那一手已经推出去了没有**（推出去之后只等"被取走"，不再推——字节住它自己那儿，
    // 中间回去收读口也不动它）。
    let mut handed = false;

    loop {
        // 1：收——非阻塞地把读口收干净。**这一手同时放 uart 走出"把一批推给我们"那一格**。
        let mut got = false;
        if !quit {
            while let Ok((n, _)) = console.rx.pull(&mut buf, Wait::POLL) {
                got = true;
                // **（临时读数）我从控制台读口取到了几字节**：`exit` 那五个字节有没有到人手里，
                // 这一行是唯一直接的回答（下面 `eat` 的收场那一格才是它的后果）。前 20 批。
                {
                    static N: ::core::sync::atomic::AtomicUsize =
                        ::core::sync::atomic::AtomicUsize::new(0);
                    if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                        protocol::debug::put(&alloc::format!("canonical: rx n={n}"));
                    }
                }
                if !eat(&mut d, &buf[..n], &mut out) {
                    quit = true; // 收场词到了：**先把待写的放完**，再走
                    break;
                }
            }
        }
        // 2：写——**两半各自有期**。往写口推的人只有本域 ⇒ 就绪就是槽空。
        //
        // **为什么第二半不能无期限**（这里就是"整机跑完不出场"那一族的因）：
        // 本域"等这一条被 uart 取走"与 uart"等本域把它那一批取走"（`Context::publish` 的第二半，
        // 也是 `Forever`）**互为前提**——两边各压着一只手、各等对方先收，而"收"那一手在各自圈里
        // 都排在**对方等的那一半之后**：
        //   uart 的一圈：等组 → 收写口 → 收线 → 排空 → **publish（在这里等本域收）**；
        //   本域的一圈：**收读口** → 行规程 → **写回显（在这里等 uart 收）**。
        // 于是只要 uart 回到"收写口"之前本域把回显推上去、而 uart 又正好进了下一圈的 `publish`，
        // 环就闭上，谁也动不了（量到的原文：`console: publish wait begin … seq=1` 之后**没有**
        // `done`，同时 `canonical: tx push begin` 之后没有"推完"那一行）。
        //
        // 解在**本域这一半有期**（`TAKEN_MS` 一拍）：等不到就回去**收读口**——正是 uart 在等本域
        // 做的那件事 ⇒ 环当场打开。本域这一条消息的字节住在**自己的 `out[0]`** 里（不是借来的），
        // 故"先回去干别的、下圈再来等"不会动到它。（uart 那一侧的 `Forever` 因此也总有回音。）
        if !out.is_empty() {
            // 第一半：推（**单次尝试**：槽满 ⇒ `Busy`，下圈再来；其余错 = 写口封了）。
            if !handed {
                match console.tx.push(&out[0], Wait::POLL) {
                    Ok(()) => handed = true,
                    Err(e) if e.source.is_busy() => {
                        // 槽里压着上一条（uart 还没取走）：短等一拍再来，不空转也不硬等。
                        let _ = console.tx.wait(HoleDir::Push, Wait::AtMost(TICK_MS));
                        continue;
                    }
                    Err(_) => return, // 写口封了 = 持设备的域没了
                }
            }
            // 第二半：**有期**等这只手被取走（等不到就回上面收读口，下圈再来）。
            match console.tx.wait(HoleDir::Push, Wait::AtMost(TAKEN_MS)) {
                Ok(true) => {
                    out.remove(0);
                    handed = false;
                }
                Ok(false) => {
                    // **（临时读数）这一拍没等到**：这一格正是从前闭成死环的那一格——它在响，
                    // 就说明"等不到就回去收读口"这条路真的在走（前 20 次）。
                    static N: ::core::sync::atomic::AtomicUsize =
                        ::core::sync::atomic::AtomicUsize::new(0);
                    if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                        protocol::debug::put("canonical: tx wait tick");
                    }
                }
                Err(_) => return, // 写口封了 = 持设备的域没了
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
        let Ok((n, _)) = console.rx.pull(&mut buf, Wait::Forever) else {
            return;
        };
        // **（临时读数）阻塞那一趟取到了几字节**（上面 `POLL` 那一支是热路，这一支才是一次"等人"）。
        {
            static N: ::core::sync::atomic::AtomicUsize =
                ::core::sync::atomic::AtomicUsize::new(0);
            if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                protocol::debug::put(&alloc::format!("canonical: waited rx n={n}"));
            }
        }
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

/// 一页缓冲（**余量**：本族一行远小于它；孔不预设长度，装不下才答 `Denied`）。
fn page() -> Option<Vec<u8>> {
    let mut v: Vec<u8> = Vec::new();
    v.try_reserve_exact(PAGE_SIZE).ok()?;
    v.resize(PAGE_SIZE, 0);
    Some(v)
}
