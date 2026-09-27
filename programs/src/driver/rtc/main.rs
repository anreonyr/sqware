#![no_std]
#![no_main]

//! rtc — **第二台设备驱动**：`rtc@101000` 的持有者（设备树里那条 11 号线），兼**报时服务**。
//!
//! 它存在的理由原来是一条**判据**（线那四格、配给、门牌、设备面这一整套，只有 `uart` 一台
//! 真设备走过——**抽象等第二个实例**）。那一刀落完之后，它是**第二个实例走进服务面**的那一台：
//! `uart` 那一面只有一个方向（把排空读到的字节交出去），本面**两个方向都有**（客人问、设备叫）。
//!
//! **主流程只有三段**（本文件就是全部）：
//!
//! ```text
//! 设备   领配给（ONLY）→ 开图 → 自证（读两次纳秒计数器：两次不同 ⇒ 它真的在走）
//! 入系统 解门牌 → 上板 + 开会话 → 上树（/device/rtc）→ 登记那条线（11 号线归本域）
//! 核心   一只组等两个源（**这两个源是 rtc 自己的形状**，见 `adapt/resident.rs`）
//!          门上有请求（客人借来一枚回信孔）  问时间 → 就地答；定闹钟 → 占住那一格 + 武装设备
//!          线上有投递（设备自己拉的线）      清掉那一格 ⇒ 那一格到点 ⇒ 从那枚孔推"那一声"
//! ```
//!
//! **适配那几段不在这里**：`Device` / `Context` 住 [`programs::driver`]；门面与常驻那两手的壳
//! 在 `adapt/{desk,resident}.rs`；会话核（纯）在 `programs::driver::rtc::core::host`。
//! 树上的名字只多一处：本域的门牌（`/device/rtc`）。**板只管生死**（不挂牌子）。

extern crate alloc;
extern crate programs;

/// 住持面（适配）：门面 / 常驻——**只属于这一台**，故由 bin 自己 `mod`。
mod adapt;

/// 设备面（本域私有：谁的设备谁自己带）。
mod rtc;

use env::Wait;
use programs::driver::context::{Context, Mine};
use programs::driver::device::Device;
use programs::driver::fail::Fail;
use programs::driver::rtc::core::Host;
use programs::program::rtc::{E_RTC, RTC_WANTS as WANTS};
use protocol::debug;
use rtc as device;

/// 本域挂在树上的名字：`/device/rtc`（[`protocol::driver::DIR`] 之下的那一段，**服务名**）。
const ME: &str = "rtc";

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**。
const MS: usize = 1000;

/// 本域那一台：**返回类型就是它的死法**——一格一格都是 `Fail::at(E_RTC, "…")`
/// （**一族口径**在 [`programs::driver::fail`]：号取自装配表——一个数都不写）。
#[programs::entry]
fn main() -> Result<(), Fail> {
    // ── 设备 ───────────────────────────────────────────────
    let [pie] = Device::claim::<{ WANTS.len() }>()?;
    debug!("rtc: got {}", WANTS.len());
    let dev = Device::open(pie).map_err(|_| Fail::at(E_RTC, "device open failed"))?;
    // 自证：那对纳秒格子读两次（两次不同 ⇒ 它是活的）。
    let (t0, t1) = (device::now(dev.view()), device::now(dev.view()));
    debug!("rtc: time {t0} -> {t1}");

    // ── 入系统 ─────────────────────────────────────────────
    // 解门牌 → 上板 ＋ 开会话 → 上树 → 占线（那一趟的壳在 [`Context::enter`]）。
    // 门牌**公开可查**（`Mine::No`）：谁都能查、谁都能用。
    let (ctx, line) = Context::enter(dev.key(), ME, Mine::No, E_RTC, Wait::AtMost(MS))?;

    // ── 核心 ───────────────────────────────────────────────
    adapt::resident::run(&ctx, &dev, line, &mut Host::new())
}
