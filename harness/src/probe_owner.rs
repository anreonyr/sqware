#![no_std]
#![no_main]

//! probe-owner — **负证客人（第二种）**：一位**有身份**的任务去顶别人声明归自己的那一格。
//!
//! [`probe_denied`](super::probe_denied) 证的是"**没身份** ⇒ 拒绝"；本程序证的是另一半：
//! **有身份、但那一格不是你的** ⇒ 也拒绝。两条合起来，`land` 的两条支路才算在真机上钉住。
//!
//! ```text
//!   1  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   2  SEEK  /device/uart            ⇒ 记下 uart 那一格**原来的号**
//!   3  LAND  /device/uart（自己的孔）⇒ 期望 DENIED（那一格是 uart 的：它声明了归属）
//!   4  SEEK  /device/uart            ⇒ 期望**还是原来那个号**（拒绝没有动那一格）
//!   5  **等** `/sys/lease` 那一格的主人退场（`probe-lease` 落完就走）⇒ 再落一次
//!      ⇒ 期望**接得上**（主人不在场 ⇒ 那一格重新可落）
//!   6  报读数就退场
//! ```
//!
//! 第 5 步是"规矩属于**活着的**主人"那一格的正证：`probe-lease` 声明归属之后直接死，
//! 内核退场钩子把它开的资源封印 ⇒ 持树者一问就知道主人不在场 ⇒ 那一格不该变成墓碑。
//!
//! # 两格读数为什么与 `probe-denied` 不同
//!
//! `probe-denied` 撞的是**一个从没铸过的名字**，故它的第二格是 `UNKNOWN`（"没被占"）。
//! 本域撞的是**已经在的名字**，故第二格必须是**同一个号**——"拒绝"不能把原来的格子弄坏，
//! 也不能把它变成"剪掉"。两台的第二格形状**故意不一样**，各自钉一支。
//!
//! # 为什么它必须有身份（`bind: true`）
//!
//! 这一台要证的正是"身份**对不上**"，故它自己得是个**已绑身份**——否则它撞到的是第一道
//! 门（没身份），量到的就不是归属那一条了。装配单上它与别的客人一样（`bind` 缺省即 `true`）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use protocol::system::operator::{EntryId, Where};

use alloc::format;

use env::{Name, PieToken, TaskId};
use protocol::driver;
use protocol::session::Quay;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 本域要顶的那一格：`/device/uart`——`uart` 把"读行"那枚孔挂在它下面，并声明**归自己**。
const DIR: &str = driver::DIR;
const ME: &str = "uart";

/// 等树 / 办一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
///
/// **照实记（搬进用例之后）**：`BAD_NOTE`、以及"没走通"那条退场路，一起退役了——判据现在是
/// **一例一条**（`cases::Suite`），失败走 panic 通道、域当场死，故失败再也走不到出口那一手。
const OK_NOTE: &str = "probe-owner: owner rule held";

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(sire) = utask::sire() else {
        return bail("probe-owner: no sire");
    };

    // 一、与树开会话（同 `echo` / `probe-denied`）。
    let Ok((tree, host)) = operator::open(sire, Wait::AtMost(MS)) else {
        return bail("probe-owner: no tree link");
    };
    let Ok(hedge) = operator::ask_hole(host) else {
        return bail("probe-owner: no tree ask");
    };

    let (Ok(dir), Ok(me)) = (Name::new(DIR), Name::new(ME)) else {
        return bail("probe-owner: bad name");
    };
    let road = [dir, me];

    // 二、那一格**原来**的号（`uart` 落的）。**有界重试**：本域可能比 `uart` 先起。
    let Some(before) = wait_id(hedge, &tree, &road) else {
        return bail("probe-owner: no /device/uart");
    };

    // 三、铸一枚自己的孔，去顶那一格——**这一手该被拒**。
    let Ok(entry) = mail::unseal_hole(env::Mark::of("probe-entry")) else {
        return bail("probe-owner: no entry");
    };
    let at = Where::At(wait_dir(hedge, &tree, dir).unwrap_or(EntryId::new(0)));
    let land = operator::land(
        hedge,
        &tree,
        host,
        at,
        me,
        entry,
        ocall::Rule::Public,
        false,
        Wait::AtMost(MS),
    );
    let land_code = match land {
        Ok(id) => {
            say(&format!("probe-owner: tree land=OK id={}", id.get()));
            ocall::OK
        }
        Err(code) => code,
    };

    // 四、那一格**还在不在**（应是原来那个号）。
    let after = operator::seek(hedge, &tree, &road, Wait::AtMost(MS));
    let seq = match after {
        Ok(id) => format!("id={}", id.get()),
        Err(code) => format!("err:{code}"),
    };
    say(&format!(
        "probe-owner: tree land={land_code} before={} after={seq}",
        before.get()
    ));

    // 五、判据两格：被拒（`DENIED`）**且**那一格没动（还是原来那个号）。
    let denied = land_code == ocall::DENIED;
    let untouched = matches!(after, Ok(id) if id == before);

    // 六、**接手那一格没主的名字**：`probe-lease` 落完 `/sys/lease`（`mine = true`）就死，
    //     故它的资源已被退场钩子封印 ⇒ 持树者该让那一格重新可落。**有界重试**：本域可能
    //     比它先跑完那几手（提示是单槽，装配者按计划顺序推）。
    let taken = take_over(hedge, &tree, host);

    say(&format!(
        "probe-owner: lease land={} (owner gone ⇒ take-over)",
        match taken {
            Ok(id) => format!("0 id={}", id.get()),
            Err(code) => format!("{code}"),
        }
    ));

    // 七、判据：**一例一条**（原先三格 `&&` 成一句）。
    let took = taken.is_ok();
    {
        {
            assert!(
                denied,
                "那一格的主人还活着，land 本该被拒（land={land_code}）"
            )
        }
    }
    {
        { assert!(untouched, "被拒之后那一格换号了（不再是 before 那个号）") }
    }
    {
        { assert!(took, "probe-lease 已经死了，那一格该重新可落") }
    }

    return Report::note(E_OK, OK_NOTE);
}

/// 落 `/sys/lease`——**那一格的主人（`probe-lease`）已经退场**，故这一次该接得上。
///
/// 有界重试：对面那台与本域并行起来，"它死了没有"要看读数而不是靠猜。
fn take_over(hedge: PieToken, link: &Quay, host: TaskId) -> Result<EntryId, u8> {
    let (Ok(dir), Ok(me)) = (Name::new("sys"), Name::new("lease")) else {
        return Err(ocall::BAD);
    };
    let road = [dir, me];
    let Ok(at) = operator::part(hedge, link, Where::Root, dir, Wait::AtMost(MS)) else {
        return Err(ocall::BAD);
    };
    let mut left = MS;
    loop {
        // 那一格先得**已经在树上**（`probe-lease` 落过）——否则本域量的是"落一个新名字"。
        if operator::seek(hedge, link, &road, Wait::AtMost(MS)).is_ok() {
            let Ok(entry) = mail::unseal_hole(env::Mark::of("takeover-entry")) else {
                return Err(ocall::BAD);
            };
            match operator::land(
                hedge,
                link,
                host,
                Where::At(at),
                me,
                entry,
                ocall::Rule::Public,
                false,
                Wait::AtMost(MS),
            ) {
                Ok(id) => return Ok(id),
                Err(ocall::DENIED) if left > 0 => {
                    // 还没死透（或我们比它先到）：等一下再来。
                    let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                    left = left.saturating_sub(1);
                }
                Err(code) => return Err(code),
            }
        } else if left > 0 {
            let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
            left = left.saturating_sub(1);
        } else {
            return Err(ocall::UNKNOWN);
        }
    }
}

/// `/device` 那一格的号（分目录幂等 + 译号）。
fn wait_dir(say_hole: PieToken, link: &Quay, dir: Name) -> Option<EntryId> {
    operator::part(say_hole, link, Where::Root, dir, Wait::AtMost(MS)).ok()?;
    operator::seek(say_hole, link, &[dir], Wait::AtMost(MS)).ok()
}

/// 等 `uart` 把门牌落上（有界）：本域可能与它并行起来。
fn wait_id(say_hole: PieToken, link: &Quay, road: &[Name]) -> Option<EntryId> {
    let mut left = MS;
    loop {
        match operator::seek(say_hole, link, road, Wait::AtMost(MS)) {
            Ok(id) => return Some(id),
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    say(note);
    return Report::note(E_TRIP, note);
}

/// 打一行。调试面是本域唯一的嘴。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
