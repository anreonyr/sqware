#![no_std]
#![no_main]

//! passer — **过客**：起来、挂一个名字、**直接死**（不说再见）。
//!
//! 它是板上那两本账的"死"判据（**"那一枚入口还答得出吗"**，`Probe` = `Reserve` 那一格）的
//! **读数程序**：与 `guest` 的区别只有一处——**它不说 `DISMISS`**。故板上那枚牌子与板侧那一格
//! 只能由"看出来的"那一档（`sweep`）收掉。
//!
//! ```text
//!   1  板那条路：seat(板) + claim(生我者, 板) —— 本端那一枚孔交给生我者（装答话路）
//!   2  问话孔：本端铸、给板读（问话从它走，答话走上面那条板路）
//!   3  服务入口：本端铸（记号 entry）、经会话交给板 ⇒ 板上挂着"passer 在哪"
//!   4  REGISTER "passer" —— 报一行读数
//!   5  **直接死**（`exit_with_note`）：不退场、不交回、无门闩、通道为空
//! ```
//!
//! # 为什么不退场
//!
//! 退场（`DISMISS`）是**客人自己说的**那一档：板当场撤格。那样就验不到"看出来的"那一档了
//! ——本程序的全部价值就在**少说那一句**：它一死，它开的那些门随之封印（退出钩子），板侧那
//! 一格与板上那枚牌子都成了死实例，而 `Reserve` 对已封印的门答 `Err` ⇒ **板问得出来**。
//!
//! # 特权级由清单定
//!
//! 本域是 **U 态**（`kernel/build.rs::INITRD_BINS`，与 `guest` 同档）：铸孔、交出、一问一答
//! 都不需要 S 态。

extern crate alloc;
extern crate programs;

// 共享物住在 supervisor 目录里，由各 bin 各自声明一次（见 `needs.rs` 头注）。
// 板：本域是**客侧**（挂一个名字）。
use protocol::board::client as board;

use alloc::format;

use env::Name;
use protocol::board::call as bcall;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::room::exit_with_note;
use runtime::env::unit as utask;

/// 本域挂在板上的名字 —— 本域知道的全部。
const ME: &str = "passer";

/// 等板 / 等答的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 本地失败写进读数的那一格（与 `board::call::BAD` 同值：没走到 / 读不懂）。
const BAD: u8 = bcall::BAD;

/// 两种退场：挂上了 / 没挂上（都**不是 panic**；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Ok(sire) = utask::sire() else {
        bail("passer: no sire")
    };
    // 板那条路：本端装一条、认下生我者那一枚（孔交给生我者，它再转授给板线程）。
    //
    // **必须先于铸入口**：入口与问话孔都是本端铸的、都交到板手里，而板按**记号**分人
    // ——牌子那一格只认得 `entry` 那一枚；两枚同来源的孔若不刻记号，板就分不出哪个是入口。
    let Ok((link, board)) = board::open(sire, MS) else {
        bail("passer: no board link")
    };
    // 问话孔：本端铸、给板读（本端自窄到只写）——问话从它走，答话走上面那条板路。
    let Ok(talk) = board::ask_hole(board) else {
        bail("passer: no ask hole")
    };
    // 本域的服务入口：别人按名字找到本域之后往它说话。它也是要交给板的那一枚——记号
    // `entry`：板那侧按它把入口与问话孔分开（两枚都是本端铸、本端交）。
    let Ok(entry) = mail::unseal_hole(board::ENTRY_MARK) else {
        bail("passer: no entry")
    };
    let Ok(me) = Name::new(ME) else {
        bail("passer: bad name")
    };

    // 一、挂上自己：服务入口经会话交给板（板因此答得出"passer 在哪"）。
    let reg = board::ask(talk, &link, board, bcall::REGISTER, me, entry, MS).unwrap_or(BAD);
    say(&format!("passer: reg={reg} entry={} say={ME}", entry.get()));

    // 二、**直接死**：不说退场那一句、不交回、不留门闩。板上那枚牌子与板侧那一格从此是
    //     死实例——只有"那一枚还答得出吗"（`Probe`）问得出来。
    let registered = reg == bcall::OK;
    exit_with_note(
        if registered { E_OK } else { E_TRIP },
        if registered {
            "passer: gone"
        } else {
            "passer: failed"
        },
    )
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail(note: &str) -> ! {
    exit_with_note(E_TRIP, note)
}

/// 打一行。调试面是本域唯一的嘴（与 `guest` / `echo` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
