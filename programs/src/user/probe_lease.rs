#![no_std]
#![no_main]

//! probe-lease — **会死的持有者**：落一块**声明归自己**的门牌，然后**直接死**。
//!
//! 它与 `probe-owner` 是**一对**：
//!
//! - `probe-owner` 顶的是 `uart` 的牌子（主人**活着**）⇒ 应被拒；
//! - 本台留下 `/sys/lease` 然后退场（主人**死了**）⇒ `probe-owner` 随后应能**接手**。
//!
//! ```text
//!   1  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   2  PART  /sys（幂等）+ SEEK ⇒ 那一格的号
//!   3  LAND  /sys/lease，规矩 = Owner（**这一格归本域**）
//!   4  **直接死**（不说再见）：本域开的那几枚孔随之封印
//! ```
//!
//! # 这一台要读出来的那一格
//!
//! 落牌那一方退场时，内核的退场钩子把它开的资源**封印**（`gate::doom` 的 `seal_owned`）——
//! 于是"那一格的主人还在不在场"这件事**问得出来**（`Reserve` 那一问，与 `VestedBy` 同一句话）。
//! 持树者据此让**没主的名字**重新可落：命名空间里不该留一块没人能改的墓碑。
//!
//! **这不是"抢占"**：接手的门槛仍是"主人**不在场**"——一位活着的持有者照样顶不掉
//! （那一条由 `probe-owner` 证）。

extern crate alloc;
extern crate programs;

use protocol::operator::Where;
use protocol::operator::call as ocall;
use protocol::operator::client as operator;

use alloc::format;

use env::Name;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::room::exit_with_note;
use runtime::env::unit as utask;

/// 本域要落的那一格：`/sys/lease`——**声明归自己**，随后本域就死。
const DIR: &str = "sys";
const ME: &str = "lease";

/// 等树 / 办一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 两种退场（都不是 panic；kernel 会把这一句连同域号打出来）。
const OK_NOTE: &str = "probe-lease: landed, leaving";
const BAD_NOTE: &str = "probe-lease: failed";

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Ok(sire) = utask::sire() else {
        bail("probe-lease: no sire")
    };
    let Ok((tree, host)) = operator::open(sire, MS) else {
        bail("probe-lease: no tree link")
    };
    let Ok(hedge) = operator::ask_hole(host) else {
        bail("probe-lease: no tree ask")
    };
    let (Ok(dir), Ok(me)) = (Name::new(DIR), Name::new(ME)) else {
        bail("probe-lease: bad name")
    };
    // `/sys` 已经在（principal / coalition 起的头）；`part` 幂等，故这里照走一遍拿号。
    let Ok(at) = operator::part(hedge, &tree, Where::Root, dir, MS) else {
        bail("probe-lease: no /sys")
    };
    let Ok(entry) = mail::unseal_hole(env::Mark::of("lease-entry")) else {
        bail("probe-lease: no entry")
    };

    // 落牌：**声明归本域**（`Rule::Owner`）。落完就走——那一格留成"没主"。
    match operator::land(
        hedge,
        &tree,
        host,
        Where::At(at),
        me,
        entry,
        ocall::Rule::Public,
        true,
        MS,
    ) {
        Ok(id) => {
            say(&format!(
                "probe-lease: tree land=0 dir={} plate={}",
                at.get(),
                id.get()
            ));
            exit_with_note(E_OK, OK_NOTE)
        }
        Err(code) => {
            say(&format!("probe-lease: tree land={code}"));
            exit_with_note(E_TRIP, BAD_NOTE)
        }
    }
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail(note: &str) -> ! {
    say(note);
    exit_with_note(E_TRIP, note)
}

/// 打一行。调试面是本域唯一的嘴。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
