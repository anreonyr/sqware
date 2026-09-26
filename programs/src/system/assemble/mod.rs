//! assemble — **scenario**：这一景有哪些 Program（**节点**）＋ 它们在系统里的关系（**边**）。
//!
//! 两件事分开摆：
//!
//!   - **节点**（[`Node::program`]）：静态声明——我是谁、实例化我要什么（`Program`）。
//!   - **边**（[`Node::edges`]）：谁上板 / 谁上树 / 谁是持树者 / 装配期给不给身份 /
//!     它是哪一双眼睛 / 死在装配哪一步报哪个号。**图的关系不塞进节点**。
//!
//! `plan::assembly::ALL` 仍是那张**装机单**（哪几台进哪张镜像、什么特权级、装载次序），
//! 本文件按它投影出这一景的节点与边；`setup` 那几格（要哪些门闩、开哪条通道）由
//! [`setup_of`] 一处给出——**需求单本身仍只有一处**（`plan::assembly` 的 `*_WANTS`）。
//!
//! 次序即契约：**内件三枚在前**（持树者排第一、名册紧随其后、盟册第三），**镜像那几台在后**
//! （按 `Plan::order` 排）。装配者按这条次序起，先起的先就绪，后面的就能向它要东西。

use alloc::vec::Vec;

use plan::assembly::{Died, Eyes, LODGER_WANTS, ROUTER_WANTS, RTC_WANTS, UART_WANTS};

use crate::system::control::Catalog;
use crate::system::program::{Program, Setup, Source};

pub mod inner;

/// **一条服务在系统里的关系（边）**。
///
/// 这五格原先住在 `Program` 上——那是"节点与边不分"。它们不是节点的属性：改一条边
/// （比如换一双眼睛）不该动节点本身。
#[derive(Clone, Copy)]
pub struct Edges {
    /// 要不要板那条路（`board::attach`）。**道也跟着这一格走**：道是板写的，不上板就不铸。
    pub board: bool,
    /// 要不要树那条路（`operator::attach`）。按需发——拿到这条路的服务，就能动整棵树。
    pub operator: bool,
    /// **装配期给不给它一条身份**（`derive(ROOT)` + `bind`）。`false` 是给负证客人的。
    pub bind: bool,
    /// **它就是持树者本身**：起来之后本域把它那条提示之路认到手，此后每位上树的客人都往
    /// 那条路上递号。**它必须排在第一位**：排在它前面的客人没树可上。
    pub holds_tree: bool,
    /// **它是持树者的哪一双眼睛**（`None` = 不是：绝大多数行都不是）。
    pub eyes: Option<Eyes>,
    /// 装配死在这一条时报哪个号（**按服务分的号住这里**）。
    pub died: Died,
}

/// **一个节点**：静态声明 ＋ 它在图里的边。
#[derive(Clone, Copy)]
pub struct Node {
    pub program: Program,
    pub edges: Edges,
}

/// 这一景要起的全部节点（次序即装配次序）。
pub fn nodes(catalog: &Catalog) -> Vec<Node> {
    let mut rows: Vec<&plan::assembly::Row> = plan::assembly::ALL
        .iter()
        .filter(|row| row.plan.is_some() && catalog.find(row.name).is_some())
        .collect();
    rows.sort_by_key(|row| row.plan.as_ref().map(|p| p.order));
    let mut all: Vec<Node> = inner::INNER.iter().copied().collect();
    all.extend(
        rows.iter()
            .filter_map(|row| row.plan.as_ref().map(|p| node_of(row.name, p))),
    );
    all
}

/// 这一景的 **Program 集合**（§9 的读面：只取节点，不含边）。
pub fn programs(catalog: &Catalog) -> Vec<Program> {
    nodes(catalog).into_iter().map(|n| n.program).collect()
}

/// 装配单的一行 → 一个节点（边逐格搬，名字取自那一行）。
fn node_of(name: &'static str, p: &plan::assembly::Plan) -> Node {
    Node {
        program: Program {
            name,
            source: Source::Catalog(name),
            setup: setup_of(name),
        },
        edges: Edges {
            board: p.board,
            operator: p.operator,
            bind: p.bind,
            holds_tree: p.holds_tree,
            eyes: p.eyes,
            died: p.died,
        },
    }
}

/// **这一台要什么**：按名字给那一份 `setup`（今天只有四台要东西，其余 `&[]`）。
///
/// **需求单不在这抄第二遍**：要的几样仍取自 `plan::assembly` 的 `ROUTER_WANTS` /
/// `UART_WANTS` / `RTC_WANTS` / `LODGER_WANTS`；这里只说"要哪几条、开哪条通道"。
/// 通道名与客人那一侧是同一个字面量（`driver/assemble.rs` 的 `RECORDS`）。
fn setup_of(name: &str) -> &'static [Setup] {
    match name {
        "router" => ROUTER_SETUP,
        "uart" => UART_SETUP,
        "rtc" => RTC_SETUP,
        "lodger" => LODGER_SETUP,
        _ => &[],
    }
}

/// 线路由者：中断控制器（按类）＋ 设备树本体 / 门铃（按已知坐标）＋ 一条通道。
static ROUTER_SETUP: &[Setup] = &[
    Setup::Need(ROUTER_WANTS[0]),
    Setup::Need(ROUTER_WANTS[1]),
    Setup::Need(ROUTER_WANTS[2]),
    Setup::Channel("records"),
];

/// 串口驱动：那一台 `ns16550a`（按类）＋ 一条通道。
static UART_SETUP: &[Setup] = &[Setup::Need(UART_WANTS[0]), Setup::Channel("records")];

/// 实时钟驱动：那一台 `google,goldfish-rtc`（按类）＋ 一条通道。
static RTC_SETUP: &[Setup] = &[Setup::Need(RTC_WANTS[0]), Setup::Channel("records")];

/// 房客：一条没人要的线（`virtio,mmio`，按类）＋ 一条通道。
static LODGER_SETUP: &[Setup] = &[Setup::Need(LODGER_WANTS[0]), Setup::Channel("records")];
