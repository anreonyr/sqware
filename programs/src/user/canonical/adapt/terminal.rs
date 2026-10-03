//! canonical::adapt::terminal — **那一圈（壳）**：读口与写口两边轮转，中间过一遍行规程。
//! ```text
//!   1  收：`console.rx.recv(POLL)` 把读口读干（**读空的那一趟把页上那一位应掉**）
//!   2  喂：逐字节进 [`Discipline::feed`]，它把要回显的字节写进本批的 `echo` 缓冲里
//!   3  写：**只在写口就绪时推**（就绪＝架上还有位）；推不进就记一枚丢，回去收读口
//!   4  都没事：阻塞等读口（此刻写口是空的，uart 不在等我们）
//! ```
//! **本域只有一条输出路**：行规程的回显。交付的行**不再写出去**——本域就是终端，没有下游；写一遍
//!
//! **两条路都不挂起**（这一刀的全部要点）：本域写口满了由架按策略丢（数落在架的 `lost` 上），
//! 而对面（uart）同样不等本域。故"两边各压一手、等对面先收"那一族环（见 `da97c34`）**从构造上
//! 不可达**——本域这一圈因此不再需要 `TAKEN_MS` / `TICK_MS` 那两格有期等待。
//!
//! **丢批 ⇒ 半行作废**：读端按 [`Reader::skipped`] 看见"中间那几格被顶掉了"，那一刻行规程里
//! 攒着的半行已经与回显对不上，故 `reset()` 掉——**壳认事实**（skipped 涨了），**核认规矩**
//! （半行怎么清，见 `core::discipline`）。

use crate::core::discipline::{Discipline, Step};
use alloc::vec::Vec;
use env::Wait;
use programs::driver::uart::client::Console;
use programs::driver::uart::core::frame::Bytes;

/// 起手先说的一句用法。**本机没有终端回显**（敲的字不会被设备送回来），不说一句就不知道能敲什么。
const USAGE: &str = "canonical: 行规程（ECHO / DEL 退格 / ^U 抹行）；^D 或 exit 收场";

/// 跑到收场词 / EOF。**收 `&mut`**：两端各自都要推游标（读端前进、写端落格）。
pub fn run(console: &mut Console) {
    let mut d = Discipline::new();
    // 待写的消息队列：**一条消息 = 一条完整的字**（回显那几格都在里面）。
    let mut out: Vec<Vec<u8>> = Vec::new();
    push_line(&mut out, USAGE);
    let mut quit = false;
    // **读端看见的跳号**与**本端推不进去的条数**：各自只在变的时候报一行（release 也看得见）。
    let mut seen = console.rx.skipped();
    let mut dropped = console.tx.lost();

    loop {
        // 1：收——非阻塞地把读口读干。**每一次 `recv` 读空时都会把页上那一位应掉**。
        let mut got = false;
        if !quit {
            while let Ok(batch) = console.rx.recv(Wait::POLL) {
                got = true;
                if console.rx.skipped() != seen {
                    seen = console.rx.skipped();
                    // 丢过批：半行作废（那一段字节与回显都对不上了）。
                    d.reset();
                    protocol::debug::put(&alloc::format!("canonical: rx gap skipped={seen}"));
                }
                if !eat(&mut d, batch.bytes(), &mut out) {
                    quit = true; // 收场词到了：**先把待写的放完**，再走
                    break;
                }
            }
        }
        // 2：写——一具架，**写端永不挂起**（满了按 `Mode::Oldest` 顶掉最旧未读格、记 `lost`）。
        for one in out.drain(..) {
            if let Some(batch) = Bytes::of(&one) {
                let _ = console.tx.send(batch);
            }
        }
        if console.tx.lost() != dropped {
            dropped = console.tx.lost();
            protocol::debug::put(&alloc::format!("canonical: tx lost={dropped}"));
        }
        // 3：收场词到了、队列也空了 ⇒ 走。
        if quit {
            return;
        }
        // 4：没得写也没收到：阻塞等读口（**此刻写口是空的，uart 不在等我们**）。
        if got {
            continue;
        }
        let Ok(batch) = console.rx.recv(Wait::Forever) else {
            return;
        };
        if !eat(&mut d, batch.bytes(), &mut out) {
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
