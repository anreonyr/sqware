#![no_std]
#![no_main]

//! guest — **第一位真客人**：按名字找到一个服务、跟它说一句话、把答话带回来。
//!
//! 本域手里只有一样东西：**名字**。`plic` 在哪个域、哪一枚孔、谁建的——那三样都由板回答
//! （`LOOKUP` 把入口**经会话**授进本域表里，不从报文里来）。
//!
//! ```text
//!   1  板那条路：seat(板) + claim(生我者, 板) —— 本端那一枚孔交给生我者（装答话路）；
//!      另铸一枚**问话孔**给板
//!   2  REGISTER "guest"：本域的服务入口经会话交给板（于是本域也能被按名字找到）
//!   3  LOOKUP   "plic" ：板上问一句，入口从会话里进本域表（找不到就再问，有界）
//!   4  借一枚**回信孔**给它、把 32 字节（**本域的名字** = "谁在敲门"）推进它的入口，
//!      再从回信孔读回一句答话（它报的是它自己的名字）
//!   5  说一句 DISMISS（**一字节帧**）——"我走了"：板据此撤格 + 摘掉本域挂在板上的牌子
//!   6  报一行读数就退场 —— 一次往返，不留常驻
//! ```
//!
//! # 一句话就是一个名字
//!
//! 两个方向各 32 字节、**定长**（尾随 NUL 是填充）——与牌子同一个解码面（[`Name`]）。
//! **这不是一份协议**：没有动作码、没有状态码、没有长度前缀也没来源字段。真要做"调用"，
//! 帧形得另开一轮裁决；这一支只证四步：名字 → 入口 → 说话 → 答话。
//!
//! # 一次调用为什么是两枚孔
//!
//! 孔是**单槽**，而一个槽只有一个读者——本端推上去的那一句，**本端自己也会读回来**
//! （推完立刻读，读到的就是自己那一句），请求因此根本到不了对端。故两个方向**各一枚孔**
//! （`session` 事实 2 说的就是这件事），各只有一个写者：
//!
//! ```text
//!   回信孔（本端铸、给对端写）  ◀── 答话（它报的名字）
//!   它的入口（对端铸、本端写）  ──▶ 那一句（本域的名字）
//! ```
//!
//! 对端凭什么知道答话该往哪儿推：**内核在推的那一刻盖的发送者戳**——它按"我表里
//! `owner` 是这位客人的那一枚"找（副本共享 `owner`）。故报文里不必带号、也不必带名字。
//! 而**同一来源的多枚孔**（本域的服务入口、问话孔、回信孔）靠**记号**分开：每铸一枚都
//! 刻上它那条路的用途名（`entry` / `ask` / `back`）。
//!
//! # 特权级由清单定
//!
//! 本域是 **U 态**（`kernel/build.rs::INITRD_BINS`）：铸孔、交出、一问一答**都不需要 S 态**，
//! 故一个最小特权的域也能按名字找到服务——这一刀最想验的就是这一句。（唯一收在 S 态的是
//! 铸**门铃**，本域用不着。）
//!
//! **退场**：本域**自己说**一句 `DISMISS`（一字节帧）——板据此撤掉本域那一格、摘掉本域
//! 挂在板上的全部牌子、并把本域的问话孔从组里摘掉（名字的位置留着）。**没说就走**的那种
//! 仍由板**看见**（答话路那枚孔径死）后惰性剔除。

extern crate alloc;
extern crate programs;

// 共享物住在 supervisor 目录里，由各 bin 各自声明一次（见 `needs.rs` 头注）。
#[path = "../supervisor/board.rs"]
// 本域只用**客侧**那三手（板侧那一半归 root）⇒ 另一半在这里是死码。
#[allow(dead_code)]
mod board;

use alloc::format;
use core::time::Duration;

use env::wire::NAME_LEN;
use env::{Name, PieToken};
use protocol::board::call as bcall;
use runtime::core::port::{self, Access, Policy};
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::room::{self, exit_with_note};
use runtime::env::unit as utask;

/// 本域挂在板上的名字，与要找的那个服务——**本域知道的全部**。
const ME: &str = "guest";
const WANT: &str = "plic";

/// 等板 / 等答的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 找不到就再问一次的间隔（毫秒）：板是**运行期**的账，本域可能比 `plic` 先起。
const RETRY_MS: usize = 1;

/// 本地失败写进读数的那一格（与 `board::call::BAD` 同值：没走到 / 读不懂）。
const BAD: u8 = bcall::BAD;

/// 两种退场：走通了 / 没走通（都**不是 panic**；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Ok(sire) = utask::sire() else {
        bail("guest: no sire")
    };
    // 板那条路：本端装一条、认下生我者那一枚（孔交给生我者，它再转授给板线程）。
    //
    // **必须先于铸入口**：入口与问话孔都是本端铸的、都交到板手里，而板按**记号**分人
    // ——牌子这一格只认得"entry"那一枚；两枚同来源的孔若不刻记号，板就分不出哪个是入口。
    let Ok((link, board)) = board::open(sire, MS) else {
        bail("guest: no board link")
    };
    // 问话孔：本端铸、给板读（本端自窄到只写）——问话从它走，答话走上面那条板路。
    // `board` = 板路上先到的那一格（**答话的是谁**）：孔只在铸它的表里念得出来，故这个号
    // 是"板收得到问话"的前提。
    let Ok(talk) = board::ask_hole(board) else {
        bail("guest: no ask hole")
    };
    // 本域的服务入口：别人按名字找到本域之后往它说话，本域从它读。它也是要交给板的那一枚
    // ——记号 `entry`：板那侧按它把入口与问话孔分开（两枚都是本端铸、本端交）。
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        bail("guest: no entry")
    };
    let Ok(me) = Name::new(ME) else {
        bail("guest: bad name")
    };
    let Ok(want) = Name::new(WANT) else {
        bail("guest: bad name")
    };
    let none = PieToken::NONE;

    // 一、挂上自己：服务入口经会话交给板（板因此答得出"guest 在哪"）。
    let reg = board::ask(talk, &link, board, bcall::REGISTER, me, entry, MS).unwrap_or(BAD);

    // 二、问一句名字。**找不到就再问**，有界：本域可能比 `plic` 先起（板上没有"装配期"）。
    let mut left = MS;
    let lookup = loop {
        let code = board::ask(talk, &link, board, bcall::LOOKUP, want, none, MS).unwrap_or(BAD);
        if code != bcall::UNKNOWN || left == 0 {
            break code;
        }
        let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
        left = left.saturating_sub(RETRY_MS);
    };

    // 三、查到的那一枚（板经会话授进本域表里）：借一枚回信孔过去、说一句、把答话读回来。
    let (at, answer) = match board::take(&link, board) {
        Some(at) => (at, call(at, me)),
        // 查到了却没在表里认出那一枚：也算没走通（读数里的 `entry=0`）。
        None => (none, None),
    };
    // 四、走完这一趟：说一句"我走了"（一字节帧，不带名字也不带入口）。板据此撤掉本域那一格、
    //     摘掉本域挂在板上的牌子，答一格 `OK`；本域不在板上那本账上则答 `UNKNOWN`。
    let bye = board::dismiss(talk, &link, MS).unwrap_or(BAD);

    let said = answer.as_ref().map(Name::as_str).unwrap_or("?");
    say(&format!(
        "guest: reg={reg} lookup={lookup} entry={} say={ME} answer={said} bye={bye}",
        at.get()
    ));

    // 五、退场：一次往返，不留常驻（kernel 打的那一行就是这一格的读数）。
    let walked = reg == bcall::OK && lookup == bcall::OK && answer.is_some();
    exit_with_note(
        if walked { E_OK } else { E_TRIP },
        if walked {
            "guest: trip ok"
        } else {
            "guest: trip failed"
        },
    )
}

/// 一次调用：**借一枚回信孔**给对端，再把话推进它的入口，然后从回信孔读答话。
///
/// 为什么是两枚孔，见文件头"一次调用为什么是两枚孔"。对端是谁**从入口本身读**：
/// `owner` = 那扇门是谁开的（副本共享同一事实），这正是"这扇门的主人"。
///
/// 读回来的若不是合法名字（对面答了别的东西），返 [`None`]——**不猜**。
fn call(at: PieToken, me: Name) -> Option<Name> {
    // 回信孔：本端铸一枚（记号 `back`：本端表里此刻已经躺着入口与问话孔，记号把它们分开），
    // 副本交给"这扇门的主人"（它只往里写，故只给 `R|W`）。
    let back = mail::unseal_hole("back").ok()?;
    let peer = bcall::opened_by(at)?;
    port::ship(
        &mail::HolePie::from_token(back),
        peer,
        Access::READ | Access::WRITE,
        Policy::NONE,
    )
    .ok()?;
    // 说一句：往**它的入口**推本域的名字。
    let hole = mail::HolePie::from_token(at);
    hole.push(me.bytes()).ok()?;
    // 读答话：从**回信孔**读（不是从它的入口读——那一枚的读者是它）。
    let mut buf = [0u8; NAME_LEN];
    let n = mail::HolePie::from_token(back)
        .pull_timeout(&mut buf, MS)
        .ok()?;
    (n == NAME_LEN)
        .then(|| Name::from_bytes(buf).ok())
        .flatten()
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail(note: &str) -> ! {
    exit_with_note(E_TRIP, note)
}

/// 打一行。调试面是本域唯一的嘴（与 `echo` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
