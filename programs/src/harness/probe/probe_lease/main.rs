#![no_std]
#![no_main]

//! probe-lease — 会死的持有者：落一块声明归自己的门牌，然后直接死。
//! 它与 `probe-owner` 是**一对**：
//! - `probe-owner` 顶的是 `uart` 的牌子（主人**活着**）⇒ 应被拒；
//! # 这一台要读出来的那一格
//! 于是"那一格的主人还在不在场"这件事**问得出来**（`Reserve` 那一问，与 `VestedBy` 同一句话）。
//! 持树者据此让**没主的名字**重新可落：命名空间里不该留一块没人能改的墓碑。
//! **这不是"抢占"**：接手的门槛仍是"主人**不在场**"——一位活着的持有者照样顶不掉
//! （那一条由 `probe-owner` 证）。

extern crate alloc;
extern crate programs;

use alloc::string::ToString;

use env::Wait;
use programs::Report;

use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::Permit;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face as TreeFace, Mine};

use runtime::env::mail;
use runtime::env::unit as utask;

/// **容器那一段那一条路**（`/svc`）——那一段名字**只在协议那一侧说**；本台只用它一个末段
/// （`file_name()`，std 同形），故取名字那一手在运行期做（`file_name` 不是 `const`）。
const DIR: &protocol::service::operator::Path = protocol::common::svc::SVC;
const ME: &str = "lease";

const MS: usize = 1000;

const E_OK: usize = 0;
/// 走不下去（`bail`）那一档：**与"判据没过"是两回事**——判据没过走 panic 通道。
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
const OK_NOTE: &str = "probe-lease: landed, leaving";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-lease: no tree link");
    };
    let tree = TreeFace::of(session);
    let Some(dir) = DIR.file_name() else {
        return bail("probe-lease: bad name");
    };
    let me = ME.to_string();
    // 拿到的就是那块 Pane（"分"与"落"现在都挂在那块 Pane 上）。
    let root = tree.root();
    let Ok(sys) = root.open(dir.to_string(), Wait::AtMost(MS)) else {
        return bail("probe-lease: no /svc");
    };
    let Ok(entry) = mail::unseal_hole(env::Mark::of("lease-entry")) else {
        return bail("probe-lease: no entry");
    };

    let landed = sys.bind(me, entry, Permit::Unset, Mine::Yes, Wait::AtMost(MS));

    // 读数那一行照旧（两种形状：落上了报号、没落上报失败域那一格的名字）——**判据**在下面那一例里。
    match &landed {
        Ok(id) => debug!(
            "probe-lease: tree land=0 dir={} plate={}",
            sys.id().get(),
            id.id().get()
        ),
        Err(fail) => debug!("probe-lease: tree land={fail:?}"),
    }

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
