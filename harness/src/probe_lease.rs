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

use env::Wait;
use programs::Report;

use protocol::debug;
use protocol::system::operator as ocall;
use protocol::system::operator::Where;
use protocol::system::operator::client as operator;


use env::Name;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 本域要落的那一格：`/sys/lease`——**声明归自己**，随后本域就死。
const DIR: &str = "sys";
const ME: &str = "lease";

/// 等树 / 办一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

const E_OK: usize = 0;
/// 走不下去（`bail`）那一档：**与"判据没过"是两回事**——判据没过走 panic 通道。
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
///
/// **照实记（搬进用例之后）**：`BAD_NOTE` 退役了——"牌没落上"现在是**用例没过**（走 panic
/// 通道、域当场死），再也走不到出口那一手；而 `E_TRIP` 留给 `bail` 那几手（起手没走通）。
const OK_NOTE: &str = "probe-lease: landed, leaving";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let Ok((tree, host)) = operator::open(sire, Wait::AtMost(MS)) else {
        return bail("probe-lease: no tree link");
    };
    let Ok(hedge) = operator::ask_hole(host) else {
        return bail("probe-lease: no tree ask");
    };
    let (Ok(dir), Ok(me)) = (Name::new(DIR), Name::new(ME)) else {
        return bail("probe-lease: bad name");
    };
    // `/sys` 已经在（principal / coalition 起的头）；`part` 幂等，故这里照走一遍拿号。
    let Ok(at) = operator::part(hedge, &tree, Where::Root, dir, Wait::AtMost(MS)) else {
        return bail("probe-lease: no /sys");
    };
    let Ok(entry) = mail::unseal_hole(env::Mark::of("lease-entry")) else {
        return bail("probe-lease: no entry");
    };

    // 落牌：**声明归本域**（`mine = true`，账里记成 `Owner`）。落完就走——那一格留成「没主」。
    let landed = operator::land(
        hedge,
        &tree,
        host,
        Where::At(at),
        me,
        entry,
        ocall::Rule::Public,
        true,
        Wait::AtMost(MS),
    );

    // 读数那一行照旧（两种形状：落上了报号、没落上报码）——**判据**在下面那一例里。
    match &landed {
        Ok(id) => debug!(
            "probe-lease: tree land=0 dir={} plate={}",
            at.get(),
            id.get()
        ),
        Err(code) => debug!("probe-lease: tree land={code}"),
    }

    // 判据：**一例**（这一台只有一条：牌落上了；落完就退场，把那一格留成"没主"）。
    let ok = landed.is_ok();
    {
        assert!(ok, "牌没落上（land 答的是码，见上面那一行读数）")
    }

    return Report::note(E_OK, OK_NOTE);
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}

