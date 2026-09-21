#![no_std]
#![no_main]

//! system — **编排域**：这台机器上有哪些服务、怎么起、谁死了怎么办。
//!
//! 它是 boot 之后**唯一**起服务的地方。引导域（`root`）只把两样东西交给它：**这块字节**
//! （清单 + 全部镜像，一枚只读门闩）与**持树者的提示之路**；此外一概不给——连"要哪几枚
//! 设备门闩"都是本域按需求单去问的（配给由引导域直接授进**客人**的表里，本域只转投那段
//! 记录，一枚原件都不经过它）。
//!
//! ```text
//! 1  会话：交给"生我者"（= 引导域）本域那一枚孔，认下它那一枚 ⇒ 一条问答路
//! 2  领账：`initrd`（只读门闩）→ 借映 → 清单；`operator-tip`（孔）→ 认下持树者是谁
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

// 需求单归**收方**：两台驱动自己开（lib 里同一份源码），本域只照它开单。
use programs::driver::router::needs as router_needs;
use programs::driver::uart::needs as uart_needs;
use programs::supervisor::service;

// 板：本域是**装配侧**（把客人接上板、收尾点名）。
use programs::supervisor::system::board::bridge as board;
// 树：本域是**装配侧**（把客人接上树）。
use programs::supervisor::operator::bridge as operator;

use env::{HoleDir, Name, PieToken, TaskId};
use programs::supervisor::system::server;
use protocol::session::{Pier, Quay};
use protocol::system::desk::{Announce, Table};
use runtime::core::dock::Dock;
use runtime::core::port::{Access, Policy};
use runtime::core::tole::Tole;
use runtime::env::mail::{self, HolePie, PolePie};
use runtime::env::unit as utask;

use protocol::firmware;
use protocol::firmware::call::{Kind, Want};
use service::{Catalog, Died, Program};

/// 载荷区那枚门闩在配对块里的名字（boot 定的，见 `kernel/platform/devices.rs`）。
const INITRD: &str = "initrd";

/// 结算两条上限（毫秒）：与引导域开会话、以及装配期的等。
const BOOT_MS: usize = 1000;

/// 装配失败编号（按服务分：看日志就知道死在哪儿）。
const E_BOOT: Died = 1;
const E_ROUTER: Died = 5;
const E_ECHO: Died = 6;
const E_GUEST: Died = 7;
const E_PASSER: Died = 8;
const E_UART: Died = 9;

/// 线路由者（中断面域）：常驻，要三枚门闩，起来时交回通道，并挂上板。
const fn router() -> Program {
    Program {
        name: "router",
        announce: Announce::Channel,
        tokens: &[],
        channels: &["records"],
        needs: Some(router_needs::WANTS),
        board: true,
        operator: false,
        died: E_ROUTER,
    }
}

/// 串口驱动：常驻，要一枚门闩（`serial@10000000`），起来时交回通道，并挂上板。
///
/// **它排在线路由者之后**：控制器先就位，线再开闸（闸门归持有设备的那一台，见该域头注）。
const fn uart() -> Program {
    Program {
        name: "uart",
        announce: Announce::Channel,
        tokens: &[],
        channels: &["records"],
        needs: Some(uart_needs::WANTS),
        board: true,
        operator: false,
        died: E_UART,
    }
}

/// 客人：按名字找到 `router`、说一句、把答话带回来。
///
/// 它什么都不交回（`Announce::None`：本域不等它），故**上板那一格由 `board` 那一支负责**
/// ——本域等的是它那条板路接上（[`board::attach`] 的第 2 步），不是它说了什么。
const fn guest() -> Program {
    Program {
        name: "guest",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: false,
        died: E_GUEST,
    }
}

/// 过客：起来、挂一个名字、**直接死**（不说再见）。
///
/// 板上那两本账的"死"判据（"**那一枚入口还答得出吗**"，`Probe` = `Reserve` 那一格）就是为它
/// 存在的读数：它不说 `DISMISS`，故只有"看出来的"那一档收得掉它。
const fn passer() -> Program {
    Program {
        name: "passer",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: false,
        died: E_PASSER,
    }
}

/// 调试回显：只走调试面，不要门闩、不交通道。**但它上板也上树**：
///
/// - 上板（`board: true`）只为让板**看得见它的死**：它退场时开的那几枚孔随退出钩子封印
///   ⇒ 板当场看出"客人没了" ⇒ 推一格死亡通知给装配者。本域的监督事件源就是这一条。
/// - 上树（`operator: true`）是**第一位真客人**：它把本域的入口挂到树上、再查回来取一枚
///   （`echo.rs::trip` 那几行）——树的载体因此有一条**跑在机器上的读数**。
const fn echo() -> Program {
    Program {
        name: "echo",
        announce: Announce::None,
        tokens: &[],
        channels: &[],
        needs: None,
        board: true,
        operator: true,
        died: E_ECHO,
    }
}

/// **装配单**：本域按这个顺序起服务。
///
/// 持树者（`operator`）**不在这张单里**：它由引导域先起，本域只从引导域手里领它那条
/// 提示之路——于是本域也**不必认识它的号**（`Reserve` 那枚孔就答出来了）。
///
/// `echo` **必须在最后**：[`service::assemble`] 返 [`PLAN`] 的最后一条，本域等它退场
/// ——那正是"读到一行 `exit` 才收场"的那一格。
///
/// 两台驱动**排在最前**且线路由者在前：控制器先就位，线再开闸（`uart` 持有那台串口）。
const PLAN: &[Program] = &[router(), uart(), guest(), passer(), echo()];

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 与引导域开会话：本域那一枚交给"生我者"，并认下它那一枚（一问一答两个方向）。
    let Some(boot_pier) = talk_to_root() else {
        service::die(E_BOOT, "system: no firmware");
    };

    // 2. 领账：这块字节里**清单与全部镜像都在里头**（同一批物理页，借映进本域的 VA）。
    let catalog = match take_catalog(&boot_pier) {
        Ok(catalog) => catalog,
        Err(why) => service::die(E_BOOT, why),
    };
    // 与树说话要两样：**它是谁**（那条提示之路的主人，`Reserve` 答出来）与**一条答话路**
    // （就在我们手里这一枚）。故本域不必问引导域"持树者的号是多少"。
    let (host, tip) = match take_tree(&boot_pier) {
        Ok(tree) => tree,
        Err(why) => service::die(E_BOOT, why),
    };

    // 3/4. 死亡道：一位服务一条（本域铸、记号 `gone-<名字>`；装配时各交一份给板线程）。
    //      一服务一道 ⇒ **身份就是"哪条道响了"**：两位同时死也不会挤丢；本线程用一只
    //      **组**等任一道（`Tole`），零轮询。组是**独占**的（`shared = false`）。
    let mut lanes: [Option<PieToken>; Table::CAP] = [None; Table::CAP];
    let tole = match Tole::unseal(false) {
        Ok(tole) => tole,
        Err(_) => service::die(service::E_TABLE, "system: no group"),
    };
    for (i, p) in PLAN.iter().enumerate() {
        let Ok(lane) = mail::unseal_hole(&alloc::format!("gone-{}", p.name)) else {
            continue;
        };
        let _ = tole.hang(&HolePie::from_token(lane), HoleDir::Pull);
        lanes[i] = Some(lane);
    }

    // 登记整张表，再按顺序起（配给从 `boot_pier` 那条路领）。
    let mut table = Table::new();
    let last = match service::assemble(&mut table, &catalog, PLAN, &boot_pier, host, tip, &lanes) {
        Ok(last) => last,
        Err(service::E_MANIFEST) => service::die(service::E_MANIFEST, "system: manifest bad"),
        Err(service::E_TABLE) => service::die(service::E_TABLE, "system: table full"),
        Err(service::E_PROGRAM) => service::die(service::E_PROGRAM, "system: program missing"),
        Err(died) => service::die(died, "system: service failed"),
    };

    // 5/6. 监督：哪条道响 ⇒ 那一位没了 ⇒ 记账 + 放下；最后一条没了 ⇒ 显式收掉仍在跑的。
    server::supervise(&mut table, last, &lanes, &tole, PLAN);
    // 会话的收尾由会话的主人负责：常驻线程是它起的，也是它收的。本域里那枚板线程没有
    // `Join` 可等（`attach` 里弃权了），故按号点名收掉——同域线程之间没有寿命耦合。
    // **等待线程也住本域**，这一刀连它们一起收（域亡 = 成员清零）。
    board::shut();
    // 7. 本域退出 ⇒ 引导域那枚孔封印 ⇒ 它退出 ⇒ 级联 ⇒ 停机。
    service::die(service::E_OK, "system: done")
}

/// 与引导域搭一条**双向**的问答路。
///
/// 两侧各装一枚（`seat`）、各认下对方那一枚（`claim`）：本域**读**自己那一枚（回单从这来），
/// **写**对端那一枚（单子往那去）。只 `seat` 不 `claim` 就只有读端——那是只收配给的客人
/// （如 `router`）的用法，编排者要问，故两半都要。
fn talk_to_root() -> Option<Pier> {
    let sire = utask::sire().ok()?;
    let slot = Name::new(firmware::BOOT).ok()?;
    let mut quay = Quay::open(sire);
    quay.seat(slot).ok()?;
    quay.claim(sire, slot, BOOT_MS).ok()?;
    quay.find(slot).copied()
}

/// 领那块载荷区并把清单读出来。
///
/// **零拷贝**：那 22 MB 不是搬过来的，是同一批物理页借映进本域——固化在清单里的镜像坐标
/// 是**相对这块区**的切片，故换一张表、换一个 VA 照样解析得出来。
fn take_catalog(pier: &Pier) -> Result<Catalog<'static>, &'static str> {
    let want =
        Want::new(INITRD, Kind::Pole, Access::FETCH, Policy::NONE).ok_or("system: payload")?;
    let token = take(pier, want).ok_or("system: payload ask")?;
    let dock = Dock::open(PolePie::from_token(token)).map_err(|_| "system: payload open")?;
    let view = dock.view();
    // SAFETY: 这段借映在**本域存活期间**一直有效（门闩在本域表里，本域到收场才退出）；
    // 视图只读（`FETCH`），本域只解析、不写。
    let blob: &'static [u8] =
        unsafe { core::slice::from_raw_parts(view.base() as *const u8, view.size()) };
    Catalog::new(blob).ok_or("system: payload manifest")
}

/// 领取树那条提示之路（由引导域从持树者手里转授过来），并**问出持树者是谁**。
///
/// `Reserve` 免费给出 `owner`（资源的来历，转发不丢）⇒ 本域不必让谁转告一个号。
fn take_tree(pier: &Pier) -> Result<(TaskId, PieToken), &'static str> {
    let want = Want::new(
        operator::TIP_NAME,
        Kind::Hole,
        Access::FETCH | Access::STORE,
        Policy::NONE,
    )
    .ok_or("system: tree want")?;
    let tip = take(pier, want).ok_or("system: tree ask")?;
    let (_, host, _) = mail::reserve(tip).map_err(|_| "system: tree reserve")?;
    Ok((host, tip))
}

/// 问引导域要一枚：递一张只有一条的单子，取回那一条的号。
///
/// 缓冲是本调用的局部（**一问一答**，一问一次）；引导期只发生两次。
fn take(pier: &Pier, want: Want) -> Option<PieToken> {
    let me = utask::self_id().ok()?;
    let name = want.name()?;
    let mut slip = [0u8; firmware::SLIP_CAP];
    let mut reply = [0u8; firmware::REPLY_CAP];
    let records = firmware::client::draw(pier, me, &[want], &mut slip, &mut reply, BOOT_MS).ok()?;
    firmware::client::pick(records, name.as_str())
}
