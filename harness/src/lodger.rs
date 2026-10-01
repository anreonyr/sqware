#![no_std]
#![no_main]

//! lodger — **房客**：占住一条线、**直接死**（不说再见）。
//!
//! 它是路由者那一手探活（`sweep`）的**读数程序**：`passer` 喂的是板那本账（"那一枚入口还答得
//! 出吗"），本域喂的是**线那本账**——它像一位驱动那样占住一条线，然后一句话不说就走。
//!
//! ```text
//!   1  认领一台：**按类 `virtio,mmio` 要**（本机八台同类，设备账给的头一台 =
//!      区首址最小的那台 `virtio_mmio@10001000`，1 号线）那枚 ONLY 门闩——真持有那台设备，
//!      但本域从不映视图、不碰寄存器（为什么要一条**没人要**的线，见 `ASK` 那一格）
//!   2  上树一条会话：FIND /svc/drv/router ⇒ 那扇门
//!   3  三趟登记 —— 成功那一格与**失败域**都卖读数（答码见 `line::frame` 那张表）：
//!        占那条 virtio 线    → `lodger: occupy=0`    （0 = OK：线归本域）
//!        同一条线再来一次     → `lodger: taken=2`     （2 = TAKEN：主人是本域自己）
//!        **报一台没有线的设备** → `lodger: unknown=1`   （1 = UNKNOWN：这台没有线——`line = 0`）
//!   4  **直接死**：不说退场、不交回 ⇒ 它铸的那枚孔随退出钩子封印 ⇒ 路由者被叫醒、探活
//!      答不出 ⇒ 拆线 + 空出格子（读数 `router: vacate line=1`）。死之前报一行 `lodger: pies=`
//!      ——**失败那两趟两边收干净了没有**的读数（见下面那一注）。
//! ```
//!
//! # `TAKEN` 那一趟为什么拿本域自己的线试
//!
//! 拿 `uart` 那条线试会**与它的登记抢时间**（装配表里 `uart` 排在本域之前，但它的登记在本域
//! 之后才办完）——谁先到谁得 `OK`，那是竞态，不是读数。拿**本域刚占下的那条线**再来一次，
//! 答 `TAKEN` 就是**确定**的，而判据一字不改（"这条线有人了"——主人是谁不影响这一格）。
//!
//! # 为什么它要真领那枚门闩
//!
//! 线路由者**不验属主**（那是下来的代价，见 [`protocol::driver::line`]），故"只报名、
//! 不领设备"一样占得住线。本域**真领**：这样"主人没了"这句话才是字面意义上真的——它确实持有
//! 那台设备，只是从不碰它（需求单上因此只要最小的一格权）。
//!
//! # 名字与线号
//!
//! 设备名只有一处（`ASK` 那一格），与 `uart` 同一条纪律：**本域不发明名字**——类是本域说的
//! （`virtio,mmio`），**哪一台由设备账回答**（列册的头一条）；线号由**契**给（区→线那条权威
//! 在设备账那一台）。
//!
//! # 特权级由清单定
//!
//! 本域是 **U 态**（`programs::unit::PROGRAMS` 里这一行的 `kind`）：铸孔、交出、上树找服务、领一枚门闩
//! 都不需要 S 态。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

// 设备那一族共用的客侧三手（`Ask` / `Hub` / `Device`）——本域领门闩走的是同一条路。
use programs::driver::device::{Ask, Hub};
use programs::unit::harness::E_LODGER;

// 树：本域是**客侧**（按名找服务）——只用那条会话（房客没有门牌，不上树）。
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::Fail;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face;

use env::{Access, PieKind, PieToken, Policy};
use protocol::driver::line;
use protocol::driver::line::frame as lcall;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 本域要认的那一台：**一条没人要的线**（`virtio,mmio` 那一类里区首址最小的一台）——
/// 领上就死。
const ASK: Ask = Ask {
    class: "virtio,mmio",
    name: None,
    kind: PieKind::Pole,
    access: Access::FETCH,
    policy: Policy::ONLY,
};

/// 本域要找的那位服务（线路由者）在树上的名字。
const SERVICE: &str = "router";

/// 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 两种退场：三趟都答对了 / 有一趟不是（都**不是 panic**；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[programs::entry]
fn main() -> Report<'static> {
    // 1. 上树一条会话（本域是**客侧**：按名找服务，会话一个域只开一条）。
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        return Report::note(E_TRIP, "lodger: no tree link");
    };
    let tree = Face::of(session);

    // 2. **认领一台**（设备账那一趟）：门闩到手就是"持有"的全部（本域不映视图、不碰寄存器）。
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
    // **读数（release 也报）：这一趟折在哪一条出口上**——`occupy` 把七条成因折成同一个
    // `Denied`，故它自己那一格码说不清（详见 `protocol::driver::line::client` 那两格的注）。
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
    let (taken, _) = attempt(entry, line);
    debug!("lodger: taken={taken}");
    // 第三趟报 `0`：那是**"这台没有线"**（设备账真会给的答案，见本文件头注），而路由者那一
    // 本账里没有零号格 ⇒ 答 `UNKNOWN`（1）。
    let unknown = attempt(entry, 0).0;
    debug!("lodger: unknown={unknown}");

    // 4. **直接死**：不说退场那一句、不交回。`held` 那条线活到本域退场为止——它铸的那枚孔
    //    随退出钩子封印，路由者那一格因此醒来（`router: vacate line=1`）。
    let _held = held;
    // 三趟之后本域表里还剩几枚：**失败那两趟两边收干净了没有**的读数——`TAKEN` 与 `UNKNOWN`
    // 各把本端 `seat` 出去的那一枚（`Endpoint::shut`）与本趟借出去的那枚回信孔放下（见
    // `protocol::driver::line::client::Line::occupy`）。少放一枚，这一格当场大 1。
    let pies = mail::table_size();
    debug!("lodger: pies={pies}");

    // 判据就地登记：**只搬本域已经在判的东西**。前三例的期望是
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
        assert_eq!(pies, 11)
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

/// 上树一趟：`FIND /svc/drv/router` ⇒ 那扇门（登记从它走）。
///
/// **（为什么不用 `Face::tile`）**：`entry` 自己已经译号一次 + `find` 一次，随后
/// `Tile::token` 又 `find` 一次 ⇒ 每趟多授一枚没人接的副本。本台**末尾那一格读数**
/// （`lodger: pies=`，表里还剩几枚）把这一点量成判据 ⇒ 必须按
/// [`Pane::tile`]（译号）＋ [`Tile::token`]（这一趟 `find`）的**一枚**写。
fn find_router(tree: &Face) -> Option<PieToken> {
    // 先拼路（`/svc/drv/router`：驱动那一族的常量接上服务名），再沿那条路取入口
    // （**译不出就重试**：门牌是驱动落的，它可能落得比本域晚）。
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
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}

/// 占一趟：报**那一段区**、收一格答码。返的第二件是那条线本身（占上了才有）。
///
/// 答码用 [`lcall::fail_to_code`]——**与线上同一张表**（客户端不从失败域另编一套号）。
fn attempt(entry: PieToken, line: u32) -> (u8, Option<line::client::Line>) {
    match line::client::Line::occupy(entry, line, Wait::AtMost(MS)) {
        Ok(held) => (lcall::OK, Some(held)),
        Err(fail) => (lcall::fail_to_code(Some(fail)), None),
    }
}
