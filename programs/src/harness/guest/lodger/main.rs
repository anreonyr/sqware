#![no_std]
#![no_main]

//! 占住一条线、直接死（不说再见）。
//! 它是路由者那一手探活（`sweep`）的**读数程序**：`passer` 喂的是板那本账（"那一枚入口还答得
//! # 为什么它要真领那枚门闩
//! 线路由者**不验属主**（那是下来的代价，见 protocol::driver::line），故"只报名、
//! 那台设备，只是从不碰它（需求单上因此只要最小的一格权）。
//! # 名字与线号
//! （`virtio,mmio`），**哪一台由设备账回答**（列册的头一条）；线号由**契**给（区→线那条权威
//! 在设备账那一台）。
//! # 特权级由清单定
//! 都不需要 S 态。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use programs::driver::shared::device::{Ask, Hub};
use programs::unit::lodger::E_LODGER;

use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::operator::{Fail, Face};
use protocol::system::operator::client as operator;

use env::{Access, PieKind, PieToken, Policy};
use protocol::driver::line;
use protocol::driver::line::frame as lcall;
use env::unit;
use runtime::core::res::pie::{table_size};

/// 领上就死
const ASK: Ask = Ask {
    class: "virtio,mmio",
    name: None,
    kind: PieKind::Pole,
    access: Access::FETCH,
    policy: Policy::ONLY,
};

const SERVICE: &str = "router";

const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        return Report::note(E_TRIP, "lodger: no tree link");
    };
    let tree = Face::of(session);

    let hub = match Hub::find(&tree, E_LODGER, Wait::AtMost(MS)) {
        Ok(hub) => hub,
        Err(_) => return Report::note(E_TRIP, "lodger: no hub"),
    };
    let deed = match hub.claim(&tree, &ASK, E_LODGER, Wait::AtMost(MS)) {
        Ok(deed) => deed,
        Err(_) => return Report::note(E_TRIP, "lodger: no device"),
    };
    debug!("lodger: claimed {}", deed.name.as_str());

    // 3. 找到线路由者，取回那扇门——三趟登记都往它推。
    let entry = match find_router(&tree) {
        Some(entry) => entry,
        None => return Report::note(E_TRIP, "lodger: no router"),
    };

    // 4. 三趟登记：占上 / 同一条线再来一次 / **报一台没有线的设备**（`line = 0`）。
    let line = deed.line;
    let (ok, held) = attempt(entry, line);
    // `Denied`，故它自己那一格码说不清（详见 protocol::driver::line::client 那两格的注）。
    //   cause: 1 门牌读不出开者 · 2 铸/交不出本端那一半 · 3 铸不出回信孔 · 4 回信孔交不出去
    //          5 登记那句推不出去 · 6 路由者答的不是 OK（`phrase` 是它答的原码）· 7 认不下对端
    if ok != lcall::OK {
        protocol::debug::put(&alloc::format!(
            "lodger: occupy deny cause={} phrase={} line={}",
            protocol::driver::line::client::OCCUPY_DENY.load(core::sync::atomic::Ordering::Relaxed),
            protocol::driver::line::client::OCCUPY_CODE.load(core::sync::atomic::Ordering::Relaxed),
            line,
        ));
    }
    debug!("lodger: occupy={ok}");
    let occupied_pies = table_size();
    let (taken, _) = attempt(entry, line);
    debug!("lodger: taken={taken}");
    // 本账里没有零号格 ⇒ 答 `UNKNOWN`（1）。
    let unknown = attempt(entry, 0).0;
    debug!("lodger: unknown={unknown}");

    //    随退出钩子封印，路由者那一格因此醒来（`router: vacate line=1`）。
    let _held = held;
    // 各把本端 `seat` 出去的那一枚（Endpoint::shut）与本趟借出去的那枚回信孔放下（见
    let pies = table_size();
    debug!("lodger: pies={pies}");

    // 三趟登记的答码（与读数同一批常量）；第四例是**探针良过的那一格**（头注：把失败那两趟的
    // 释放临时关掉，同一处读数会涨）——故它是一个**判据**，不是常数（少放一枚孔，这一例就红）。
    assert_eq!(ok, lcall::OK);
    {
        assert_eq!(taken, lcall::TAKEN)
    }
    {
        assert_eq!(unknown, lcall::UNKNOWN)
    }
    {
        assert_eq!(pies, occupied_pies)
    }

    let all = ok == lcall::OK && taken == lcall::TAKEN && unknown == lcall::UNKNOWN;
    return Report::note(
        if all { E_OK } else { E_TRIP },
        if all {
            "lodger: gone"
        } else {
            "lodger: failed"
        },
    );
}

/// 上树一趟：`FIND /svc/drv/router` ⇒ 那扇门（登记从它走）
/// **（为什么不用 Face::tile）**：`entry` 自己已经译号一次 + `find` 一次，随后
/// （`lodger: pies=`，表里还剩几枚）把这一点量成判据 ⇒ 必须按
fn find_router(tree: &Face) -> Option<PieToken> {
    // 先拼路（`/svc/drv/router`：驱动那一族的常量接上服务名），再沿那条路取入口
    let road = protocol::driver::ROAD.try_join(SERVICE)?;
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(&road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
        {
            Ok(entry) => return Some(entry),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::core::task::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}

/// 占一趟：报**那一段区**、收一格答码。返的第二件是那条线本身（占上了才有）
/// 答码用 lcall::fail_to_code——**与线上同一张表**（客户端不从失败域另编一套号）
fn attempt(entry: PieToken, line: u32) -> (u8, Option<line::client::Line>) {
    match line::client::Line::occupy(entry, line, Wait::AtMost(MS)) {
        Ok(held) => (lcall::OK, Some(held)),
        Err(fail) => (lcall::fail_to_code(Some(fail)), None),
    }
}
