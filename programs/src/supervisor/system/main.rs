#![no_std]
#![no_main]

//! system — **编排域**：这台机器上有哪些服务、怎么起、谁死了怎么办。
//!
//! 它是 boot 之后**唯一**起服务的地方。引导域（`root`）只把一样东西交给它：**这块字节**
//! （清单 + 全部镜像，一枚只读门闩）；此外一概不给——连"要哪几枚设备门闩"都是本域按需求单
//! 去问的（配给由引导域直接授进**客人**的表里，本域只转投那段记录，一枚原件都不经过它）。
//!
//! **持树者也是本域起的服务**（`PLAN` 第一条）：它不另走引导域那条路——引导域不当它的装配者，
//! 也不替它把提示之路转来转去（那是"它是引导设施"时代的形状，那一笔已经清掉）。
//!
//! ```text
//! 1  会话：交给"生我者"（= 引导域）本域那一枚孔，认下它那一枚 ⇒ 一条问答路
//! 2  领账：`initrd`（只读门闩）→ 借映 → 清单
//! 3  按装配单登记整张表（`PLAN`）
//! 4  逐条起：建域 → 产线程 → 定会话 → 装通道 → 放行 → 等就绪 → 领配给 → 上板 → 接树
//! 5  监督：板手里挂着每位客人的孔（封印即投信），它看出谁没了就往死亡道推一格；
//!    本域从那条路醒来 ⇒ 等它收尾（`service::until`：**问 → 等 → 问**）⇒ 记账 ⇒ 放下死域
//! 6  最后一条没了 ⇒ 对仍在跑的显式 `stop`（`Ruin` = 域粒度 `Doom`）⇒ 全部记完 ⇒ 收场
//! 7  本域退出 ⇒ 引导域那枚孔随之封印 ⇒ 它退出 ⇒ 级联扑杀 ⇒ 自然停机（srst）
//! ```
//!
//! **本文件只剩流程**：起谁、按什么顺序、开哪几条通道、要哪些门闩、上不上板、上不上树，
//! 全在 [`PLAN`]（一张表）。要加第三个服务 —— 表里加一行，本文件一个字不改。

extern crate alloc;
extern crate programs;

use env::Mark;
use programs::supervisor::service;

// 照实记：这里原来还 `use ...::board::bridge as board`——只为收尾那一句 `board::shut()`。
// 那一手已删（它收掉的是本域自己，见第 7 步的照实记），故这一行也走了。板那一侧的装配
// （把客人接上板）住 `service::start`，本文件本来就不碰它。
use env::{HoleDir, Name, PieToken};
use programs::supervisor::system::machine::Machine;
use programs::supervisor::system::server;
use protocol::session::{Pier, Quay};
use protocol::system::desk::Table;
use runtime::core::dock::Dock;
use runtime::core::port::{Access, Policy};
use runtime::core::tole::Tole;
use runtime::env::mail::{self, HolePie, PolePie};
use runtime::env::unit as utask;

use protocol::driver::supply;
use protocol::driver::supply::call::{Kind, Want};
use env::assembly::E_BOOT;
use service::{Catalog, Program};

mod scenario;


// **照实记（这里原先有四个名字常量：`TREE` / `PRINCIPAL` / `COALITION` / `MEMBER`）**：
// 它们是"装配期认谁"的四个名字，而**认它们的是装配的机器**（`service::assemble` 里那三处
// `p.name == …`），不是本文件。装配单搬去 `env::assembly` 那一刀之后，本文件只剩流程，
// 这四个名字在这里**一个读者都没有**（编译期一直报 `never used`）——留着就是同一条事实
// 写两处，故删。名字仍各自只有一处：住 `service.rs`（持树者那条在 `Plan::holds_tree` 上）。

/// 结算两条上限（毫秒）：与引导域开会话、以及装配期的等。
const BOOT_MS: usize = 1000;


// ── 装配单搬到 `scenario.rs`（用户裁定"测试和程序分开"）──────────────
//
// 那些行 [`Program`] 与装配单现在住 `scenario.rs`：本文件是**机器**，
// 不认识具体哪一台。`PLAN` 仍从那里 `use` 进来，故下面 [`service::assemble`] 那几处一字未改。

/// 本域的死法：**一格 = 死在起手的哪一步**——号与从前的 `service::die` **同值**
/// （`1` 引导那一族；`2/3/4` 归 [`service`] 那三格；`5/6/7` 是本域自己的）。
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
    Ruin,
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
            Fail::Ruin => 9,
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
            Fail::Ruin => "system: ruin",
        }
    }
}

impl programs::Exit for Fail {
    fn report(&self) -> programs::Report<'_> {
        programs::Report::note(self.code(), self.text())
    }
}

#[programs::entry]
fn main() -> Result<programs::Report<'static>, Fail> {
    // 1. 与引导域开会话：本域那一枚交给"生我者"，并认下它那一枚（一问一答两个方向）。
    let Some(boot_pier) = talk_to_root() else {
        return Err(Fail::Firmware);
    };

    // 2. 领树：本域手里那台机器的自述——单子上那一格写的是**类**，翻成"哪一段区"要有它。
    //    坐标是 `Key::dtb()`（"哪一件"那一形：树不知道自己写在哪，故只能这么取）。
    let machine = match take_machine(&boot_pier) {
        Ok(machine) => machine,
        Err(_) => return Err(Fail::Machine),
    };

    // 2′. 领账：这块字节里**清单与全部镜像都在里头**（同一批物理页，借映进本域的 VA）。
    //     载荷区的**坐标从树里读**（`/chosen` 的 `linux,initrd-start`）——机器自己写着它在哪，
    //     本域不另抄一个名字。树也在这一块里——它是**本域起的服务**（`PLAN` 第一条）。
    let Some(payload) = machine.payload() else {
        return Err(Fail::Payload);
    };
    let catalog = match take_catalog(&boot_pier, payload) {
        Ok(catalog) => catalog,
        Err(_) => return Err(Fail::Machine),
    };

    // 3/4. 死亡道：一位服务一条（本域铸、记号 `gone-<名字>`；装配时各交一份给板线程）。
    //      一服务一道 ⇒ **身份就是"哪条道响了"**：两位同时死也不会挤丢；本线程用一只
    //      **组**等任一道（`Tole`），零轮询。组是**独占**的（`shared = false`）。
    let mut lanes: [Option<PieToken>; Table::CAP] = [None; Table::CAP];
    let tole = match Tole::unseal(false) {
        Ok(tole) => tole,
        Err(_) => return Err(Fail::Group),
    };
    // **装配单从 `env::assembly` 派生**（`order` 那一格就是起手位次）。
    let plan = scenario::plan(&catalog);

    for (i, p) in plan.iter().enumerate() {
        let Ok(lane) = mail::unseal_hole(Mark::of(&alloc::format!("gone-{}", p.name))) else {
            continue;
        };
        let _ = tole.attach(&HolePie::from_token(lane), HoleDir::Pull);
        lanes[i] = Some(lane);
    }

    // 登记整张表，再按顺序起（配给从 `boot_pier` 那条路领）。
    let mut table = Table::new();
    let last = match service::assemble(&mut table, &catalog, &plan, &boot_pier, &lanes, &machine) {
        Ok(last) => last,
        Err(code) => return Err(Fail::Assemble(code)),
    };

    // 5/6. 监督：哪条道响 ⇒ 那一位没了 ⇒ 记账 + 放下；最后一条没了 ⇒ 显式收掉仍在跑的。
    server::supervise(&mut table, last, &lanes, &tole, &plan);
    // 会话的收尾由会话的主人负责：常驻线程是它起的，也是它收的。本域里那枚板线程没有
    // `Join` 可等（`attach` 里弃权了），故按号点名收掉——同域线程之间没有寿命耦合。
    // **等待线程也住本域**，这一刀连它们一起收（域亡 = 成员清零）。
    // 7. 本域退出 ⇒ 引导域那枚孔封印 ⇒ 它退出 ⇒ 级联 ⇒ 停机。
    //
    // **照实记（这一格量出来的）**：这一句上面原来还有一手 `board::shut()`——它点名
    // `doom` 本域那枚常驻板线程。而内核那一手的粒度是**域**（"杀它所属的域连同它的子树"），
    // 板线程**就住在编排域里** ⇒ 那一叫收掉的正是**本域自己**：编排域当场被扑杀，
    // 下面这一句判词**永远够不到**（实测：`TMP-c: board shut returned` 不出现，而
    // `system: done` 在 **1005 份 soak 日志里一次都没有**）。
    // 板线程本来就不必点名收：本域一退场，"域亡＝成员清零"把它一起带走——故那一手是
    // **重复的一刀**，代价是把本机最后一句读数一起收走了。
    // （本域收场是"被板那一刀扑杀"，故这一句判词**到不了**——留着只为类型闭合。）
    Ok(programs::Report::note(env::EXIT_OK, "system: done"))
}

/// 与引导域搭一条**双向**的问答路。
///
/// 两侧各装一枚（`seat`）、各认下对方那一枚（`claim`）：本域**读**自己那一枚（回单从这来），
/// **写**对端那一枚（单子往那去）。只 `seat` 不 `claim` 就只有读端——那是只收配给的客人
/// （如 `router`）的用法，编排者要问，故两半都要。
fn talk_to_root() -> Option<Pier> {
    let sire = utask::sire().ok()?;
    let slot = Name::new(supply::BOOT).ok()?;
    let mut quay = Quay::open(sire);
    quay.seat(slot).ok()?;
    quay.claim(sire, Mark::of(supply::BOOT), BOOT_MS).ok()?;
    quay.find(slot).copied()
}

/// 领树：与载荷区同一条路（一张只有一条的单子 + 借映）。
///
/// **本域为什么读树**：单子上那一格写的是类（`compatible`），翻成"哪一段区"要有设备树；而单子
/// 是本域造的（子方只认得生我者，单子不经过它），故读树只能落在本域（理由见
/// `system::machine` 头注）。这一枚的坐标是 [`env::Key::dtb`]——**它不是树里的节点**。
fn take_machine(pier: &Pier) -> Result<Machine, Fail> {
    let want = Want::new(env::Key::dtb(), Kind::Pole, Access::FETCH, Policy::NONE);
    let token = take(pier, want).ok_or(Fail::Machine)?;
    let dock = Dock::open(PolePie::from_token(token)).map_err(|_| Fail::Machine)?;
    Machine::of(dock.view()).map_err(|_| Fail::Machine)
}

/// 领那块载荷区并把清单读出来。坐标是**机器自己在树里写的那一段**（`/chosen`，见 `main`）。
///
/// **零拷贝**：那几十 MB 不是搬过来的，是同一批物理页借映进本域——固化在清单里的镜像坐标
/// 是**相对这块区**的切片，故换一张表、换一个 VA 照样解析得出来。
fn take_catalog(pier: &Pier, key: env::Key) -> Result<Catalog<'static>, Fail> {
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

/// 问引导域要一枚：递一张只有一条的单子，取回那一条的号（按**坐标**认，不按位次——这一手
/// 是编排域给自己领，与"配给推进客人"那条路无关）。
///
/// 缓冲是本调用的局部（**一问一答**，一问一次）；引导期只发生两次。
fn take(pier: &Pier, want: Want) -> Option<PieToken> {
    let me = utask::self_id().ok()?;
    let key = want.key()?;
    let mut slip = [0u8; supply::ORDER_CAP];
    let mut reply = [0u8; supply::REPLY_CAP];
    let records = supply::client::draw(pier, me, &[want], &mut slip, &mut reply, BOOT_MS).ok()?;
    supply::client::pick(records, key)
}

