//! assemble — **scenario**：这一景起哪些台、每台要什么。
//!
//! **没有投影**：权威就是那张装机单 [`plan::assembly::ALL`]（名字 / 特权级 / 进哪张镜像 /
//! 关系 / 死在装配哪一步）。本文件只做那两件单子自己做不了的事：
//!
//!   - [`rows`]：把"这一景真有的"滤出来、按 `order` 排（次序即装配次序）；
//!   - [`setup_of`]：每台"实例化要什么"（今天只有四台要东西）。
//!
//! **图的关系不在这里**（board / operator / holds_tree / eyes / bind / died）：它们就在
//! `Plan` 那几格上，由 `System` 直接读 —— 不再抄进第二张表（那一层曾是"旧 `Program` 的
//! 后半截换个名字"，本笔删掉）。

use alloc::vec::Vec;

use plan::assembly::{ALL, LODGER_WANTS, ROUTER_WANTS, RTC_WANTS, Row, UART_WANTS};

use crate::system::control::Catalog;
use crate::system::program::Setup;

/// 这一景要起的台：**按 `order` 排**（小的先起）。先起的先就绪，后面的就能向它要东西。
///
/// 三枚服务（`operator` / `principal` / `coalition`）就在这张单里（order 0/1/2）——它们与其他
/// 每一台同一条路，不再是"与编排者共一份字节"的那三行。
pub fn rows(catalog: &Catalog) -> Vec<&'static Row> {
    let mut rows: Vec<&'static Row> = ALL
        .iter()
        .filter(|row| row.plan.is_some() && catalog.find(row.name).is_some())
        .collect();
    rows.sort_by_key(|row| row.plan.as_ref().map(|p| p.order));
    rows
}

/// **这一台要什么**：按名字给那一份 `setup`（今天只有四台要东西，其余 `&[]`）。
///
/// **需求单不在这抄第二遍**：要的几样仍取自 `plan::assembly` 的 `ROUTER_WANTS` /
/// `UART_WANTS` / `RTC_WANTS` / `LODGER_WANTS`；这里只说"要哪几条、开哪条通道"。
/// 通道名与客人那一侧是同一个字面量（`driver/assemble.rs` 的 `RECORDS`）。
pub fn setup_of(name: &str) -> &'static [Setup] {
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
