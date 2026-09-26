#![no_std]
#![no_main]

//! system — **编排域**：这台机器上有哪些服务、怎么起、谁死了怎么办。
//!
//! 它是 boot 之后**唯一**起服务的地方。引导域（`root`）只把一样东西交给它：**这块字节**
//! （清单 + 全部镜像，一枚只读门闩）；此外一概不给。
//!
//! ```text
//! 1  会话：交给"生我者"（= 引导域）本域那一枚孔，认下它那一枚 ⇒ 一条问答路
//! 2  领账：`initrd`（只读门闩）→ 借映 → 清单
//! 3  按装配单登记整张名册（`scenario::roster`：内件三枚 ＋ 镜像那几台）
//! 4  逐条起：建域 → 产线程 → 定会话 → 装通道 → 放行 → 等就绪 → 领配给 → 上板 → 接树
//! 5  监督：板手里挂着每位客人的孔（封印即投信），它看出谁没了就往死亡道推一格；
//!    本域从那条路醒来 ⇒ 等它收尾（`service::until`：**问 → 等 → 问**）⇒ 记账 ⇒ 放下死域
//! 6  最后一条没了 ⇒ 对仍在跑的显式 `stop`（`doom` = 域粒度 `Doom`）⇒ 全部记完 ⇒ 收场
//! 7  本域退出 ⇒ 引导域那枚孔随之封印 ⇒ 它退出 ⇒ 级联扑杀 ⇒ 自然停机（srst）
//! ```
//!
//! **本文件只剩流程**：起谁、按什么顺序、开哪几条通道、要哪些门闩、上不上板、上不上树，
//! 全在 `system/assemble/`。要加第三个服务 —— 表里加一行，本文件一个字不改。

extern crate alloc;
extern crate programs;

use env::Mark;
use env::Wait;
use programs::service;

use env::{HoleDir, Name, PieToken};
use programs::system::machine::Machine;
use programs::system::{server, supervise};
use protocol::session::{Pier, Quay};
use protocol::system::board::LANE_PREFIX;
use protocol::system::desk::Table;
use runtime::core::dock::Dock;
use runtime::core::pile::Pile;
use runtime::core::port::{Access, Policy};
use runtime::env::mail::{self, HolePie, PolePie};
use runtime::env::unit as utask;

use contract::driver::supply::frame::{Kind, Want};
use plan::assembly::E_BOOT;
use protocol::driver::supply;
use service::{Catalog, Lane, Role};

// 装配单/名册住 lib 的 `system/assemble/`（同一份数据的两半：投影 + 内件表）。
use programs::system::assemble as scenario;
use programs::system::{coalition, operator, principal};

/// 结算两条上限（毫秒）：与引导域开会话、以及装配期的等。
const BOOT_MS: usize = 1000;

/// 本域的死法：**一格 = 死在起手的哪一步**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 与引导域那条会话没搭上。
    Firmware,
    /// 那台机器的自述（`Key::dtb`）没领到 / 读不懂。
    Machine,
    /// 那块载荷区（清单在里面）没领到 / 读不懂。
    Payload,
    /// 清单那一条读不懂。
    Manifest,
    /// 死亡道那只组。
    Group,
    /// 整表装配那一趟带来的号（`service` 那一族原样往外带）。
    Assemble(env::Reason),
    /// 监督那一趟。
    Supervise,
    /// 收尾那一趟。
    Doom,
}

impl Fail {
    fn code(self) -> env::Reason {
        match self {
            Fail::Firmware => E_BOOT,
            Fail::Assemble(code) => code,
            Fail::Machine => 5,
            Fail::Payload => 6,
            Fail::Manifest => 7,
            Fail::Group => service::E_TABLE,
            Fail::Supervise => 8,
            Fail::Doom => 9,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Fail::Firmware => "system: no firmware",
            Fail::Assemble(_) => "system: assemble",
            Fail::Machine => "system: no machine",
            Fail::Payload => "system: no payload",
            Fail::Manifest => "system: manifest bad",
            Fail::Group => "system: no group",
            Fail::Supervise => "system: supervise",
            Fail::Doom => "system: doom",
        }
    }
}

impl programs::Exit for Fail {
    fn report(&self) -> programs::Report<'_> {
        programs::Report::note(self.code(), self.text())
    }
}

/// **四种角色共用的出口**：`Ok` 报 [`EXIT_OK`](env::EXIT_OK)，`Err` 那一格原样往上递。
fn exit(role: Result<(), programs::Report<'static>>) -> programs::Report<'static> {
    match role {
        Ok(()) => programs::Report::new(env::EXIT_OK),
        Err(r) => r,
    }
}

/// **本域的死法 → 出口那一格**（两句话都是常量：`code()` + `text()`）。
///
/// 三枚内件走 [`server::said`](server::said)——它们共用 [`server::Start`] 那一枚死法类型。
fn said(f: Fail) -> programs::Report<'static> {
    programs::Report::note(f.code(), f.text())
}

#[programs::entry]
fn main() -> programs::Report<'static> {
    // **一枚 ELF 四种角色**：角色由 `Spawn` 那一格 `args` 递进来（[`Role`]）。空 args
    // ⇒ **编排域自己那一枚**。
    match Role::of_args(runtime::core::unit::args()) {
        Role::System => exit(system().map_err(said)),
        Role::Tree => exit(operator::server::serve().map_err(server::said)),
        Role::Roster => exit(principal::server::serve().map_err(server::said)),
        Role::League => exit(coalition::server::serve().map_err(server::said)),
    }
}

/// **编排域那一枚的身子**：这台机器上有哪些服务、怎么起、谁死了怎么办。
///
/// 返 `Result<(), Fail>`：本域的死法有类型（[`Fail`]），折成出口那一格是 [`main`] 那一步的事。
fn system() -> Result<(), Fail> {
    // 1. 与引导域开会话：本域那一枚交给"生我者"，并认下它那一枚（一问一答两个方向）。
    let Some(boot_pier) = talk_to_root() else {
        return Err(Fail::Firmware);
    };

    // 2. 领树：本域手里那台机器的自述——单子上那一格写的是**类**，翻成"哪一段区"要有它。
    //    坐标是 `Key::dtb()`。
    let machine = match take_machine(&boot_pier) {
        Ok(machine) => machine,
        Err(_) => return Err(Fail::Machine),
    };

    // 2′. 领账：这块字节里**清单与全部镜像都在里头**（同一批物理页，借映进本域的 VA）。
    //     载荷区的**坐标从树里读**（`/chosen` 的 `linux,initrd-start`）。
    let Some(payload) = machine.payload() else {
        return Err(Fail::Payload);
    };
    let catalog = match take_catalog(&boot_pier, payload) {
        Ok(catalog) => catalog,
        Err(_) => return Err(Fail::Machine),
    };

    // 3/4. 死亡道：**上板的那几位**一位一条（本域铸、记号 `gone-<名字>`；装配时各交一份给
    //      板线程）。一服务一道 ⇒ **身份就是"哪条道响了"**：两位同时死也不会挤丢；本线程用
    //      一只**组**等任一道（`Pile`），零轮询。组是**独占**的（`shared = false`）。
    let pile = match Pile::unseal(false) {
        Ok(pile) => pile,
        Err(_) => return Err(Fail::Group),
    };
    // **名册**：内件三枚在前、镜像里那几台在后（`roster` 那一处给次序）。
    let roster = scenario::roster(&catalog);
    let mut lanes: alloc::vec::Vec<Lane> = alloc::vec::Vec::new();
    if lanes.try_reserve(roster.len()).is_err() {
        return Err(Fail::Group);
    }
    for (_, p) in roster.iter() {
        let road = if p.board {
            // 记号 = `LANE_PREFIX` ＋ 名字：**前缀只有一处定义**（板那一侧按同一个常量
            // 拼出来找它——见 `lane_for`）。
            mail::unseal_hole(Mark::of(&alloc::format!("{LANE_PREFIX}{}", p.name))).ok()
        } else {
            None
        };
        if let Some(road) = road {
            let _ = pile.attach(&HolePie::from_token(road), HoleDir::Pull);
        }
        lanes.push(Lane { name: p.name, road });
    }

    // 登记整条名册，再按顺序起（配给从 `boot_pier` 那条路领）。
    let mut table = Table::new();
    let last = match service::assemble(&mut table, &catalog, &roster, &boot_pier, &lanes, &machine)
    {
        Ok(last) => last,
        Err(code) => return Err(Fail::Assemble(code)),
    };

    // 5/6. 监督：哪条道响 ⇒ 那一位没了 ⇒ 记账 + 放下；最后一条没了 ⇒ 显式收掉仍在跑的。
    supervise::run(&mut table, last, &lanes, &pile);
    // 7. 本域退出 ⇒ 引导域那枚孔封印 ⇒ 它退出 ⇒ 级联 ⇒ 停机。
    Ok(())
}

/// 与引导域搭一条**双向**的问答路。
///
/// 两侧各装一枚（`seat`）、各认下对方那一枚（`claim`）：本域**读**自己那一枚（回单从这来），
/// **写**对端那一枚（单子往那去）。
fn talk_to_root() -> Option<Pier> {
    let sire = utask::sire();
    let slot = Name::new(supply::BOOT).ok()?;
    let mut quay = Quay::open(sire, protocol::session::call::hands());
    quay.seat(slot).ok()?;
    quay.claim(sire, Mark::of(supply::BOOT), Wait::AtMost(BOOT_MS))
        .ok()?;
    quay.find(slot).copied()
}

/// 领树：与载荷区同一条路（一张只有一条的单子 + 借映）。
fn take_machine(pier: &Pier) -> Result<Machine, Fail> {
    let want = Want::new(plan::Key::dtb(), Kind::Pole, Access::FETCH, Policy::NONE);
    let token = take(pier, want).ok_or(Fail::Machine)?;
    let dock = Dock::open(PolePie::from_token(token)).map_err(|_| Fail::Machine)?;
    Machine::of(dock.view()).map_err(|_| Fail::Machine)
}

/// 领那块载荷区并把清单读出来。坐标是**机器自己在树里写的那一段**（`/chosen`）。
///
/// **零拷贝**：那几十 MB 不是搬过来的，是同一批物理页借映进本域。
fn take_catalog(pier: &Pier, key: plan::Key) -> Result<Catalog<'static>, Fail> {
    let want = Want::new(key, Kind::Pole, Access::FETCH, Policy::NONE);
    let token = take(pier, want).ok_or(Fail::Payload)?;
    let dock = Dock::open(PolePie::from_token(token)).map_err(|_| Fail::Payload)?;
    let view = dock.view();
    // SAFETY: 这段借映在**本域存活期间**一直有效（门闩在本域表里，本域到收场才退出）；
    // 视图只读（`FETCH`），本域只解析、不写。
    let blob: &'static [u8] =
        unsafe { core::slice::from_raw_parts(view.base() as *const u8, view.size()) };
    Catalog::new(blob).ok_or(Fail::Manifest)
}

/// 问引导域要一枚：递一张只有一条的单子，取回那一条的号（按**坐标**认，不按位次）。
///
/// 缓冲是本调用的局部（**一问一答**，一问一次）；引导期只发生两次。
fn take(pier: &Pier, want: Want) -> Option<PieToken> {
    let me = utask::self_id();
    let key = want.key()?;
    let mut reply = [0u8; supply::REPLY_CAP];
    let records =
        supply::client::draw(pier, me, &[want], &mut reply, Wait::AtMost(BOOT_MS)).ok()?;
    supply::client::pick(records, key)
}
