#![no_std]
#![no_main]

//! rtc — **第二台设备驱动**：`rtc@101000` 的持有者（设备树里那条 11 号线），兼**报时服务**。
//! 它存在的理由原来是一条**判据**（线那四格、配给、门牌、设备面这一整套，只有 `uart` 一台
//! 第二个走进服务面协议的真实实例。
//! `uart` 那一面只有一个方向（把排空读到的字节交出去），本面**两个方向都有**（客人问、设备叫）。
//! **主流程只有三段**（本文件就是全部）：
//! ```text
//! 入系统 解门牌 → 上板 + 开会话 → 上树（/svc/drv/rtc）
//! 设备   从设备账认领那一台 → 开图 → 自证（读两次纳秒计数器：两次不同 ⇒ 它真的在走）
//!        → 登记那条线（契里给的那个号归本域）
//! 核心   一只组等两个源（**这两个源是 rtc 自己的形状**，见 `adapt/resident.rs`）
//!          门上有请求（客人借来一枚回信孔）  问时间 → 就地答；定闹钟 → 占住那一格 + 武装设备
//!          线上有投递（设备自己拉的线）      清掉那一格 ⇒ 那一格到点 ⇒ 从那枚孔推"那一声"
//! ```
//! **适配那几段不在这里**：`Device` / `Hub` / `Context` 住 [`programs::driver`]；门面与常驻那两
//! 手的壳在 `adapt/{desk,resident}.rs`；会话核（纯）在 `programs::driver::rtc::core::host`。
//! 树上的名字只多一处：本域的门牌（`/svc/drv/rtc`）。**板只管生死**（不挂牌子）。

extern crate alloc;
extern crate programs;

/// 住持面（适配）：门面 / 常驻——**只属于这一台**，故由 bin 自己 `mod`。
mod adapt;

/// 设备面（本域私有：谁的设备谁自己带）。
mod dev;

use env::{Access, PieKind, Policy, Wait};
use programs::driver::shared::context::{Context, Step};
use programs::driver::shared::device::{Ask, Device, Hub};
use programs::driver::shared::fail::Fail;
use programs::driver::rtc::core::Host;
use programs::unit::rtc::E_RTC;
use protocol::debug;
use protocol::driver::ENTRY_MARK;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Mine;
use dev::rtc as device;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 本域要认的那一台：**那一台 `google,goldfish-rtc`**（类 ＋ 独占的读写真）。
const ASK: Ask = Ask {
    class: "google,goldfish-rtc",
    name: None,
    kind: PieKind::Pole,
    access: Access::FETCH_STORE,
    policy: Policy::ONLY,
};

/// 本域挂在树上的名字：`/svc/drv/rtc`（[`protocol::driver::ROAD`] 之下的那一段，**服务名**）。
const ME: &str = "rtc";

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**。
const MS: usize = 1000;

/// 本域那一台：**返回类型就是它的死法**——一格一格都是 `Fail::at(E_RTC, "…")`
/// （**一族口径**在 [`programs::driver::shared::fail`]：号取自装配表——一个数都不写）。
#[programs::entry]
fn main() -> Result<(), Fail> {
    // 解门牌 → 上板 ＋ 开会话 → 上树落门牌。门牌**公开可查**（`Mine::No`）：谁都能查、谁都能用。
    let entry = mail::unseal_hole(ENTRY_MARK).map_err(|_| Fail::at(E_RTC, "tree"))?;
    let ctx = Context::join(entry, utask::sire(), Wait::AtMost(MS)).map_err(|s| {
        Fail::at(
            E_RTC,
            match s {
                Step::Tree => "tree",
            },
        )
    })?;
    // 找设备账那两枚面 → 认领一台（类 `google,goldfish-rtc`）→ 开图 → 自证。
    let tree = operator::Face::from(&ctx.session);
    let hub = Hub::find(&tree, E_RTC, Wait::AtMost(MS))?;
    let deed = hub.claim(&tree, &ASK, E_RTC, Wait::AtMost(MS))?;
    debug!("rtc: claimed {}", deed.name.as_str());
    let dev = Device::open(deed.token).map_err(|_| Fail::at(E_RTC, "device open failed"))?;
    // 自证：那对纳秒格子读两次（两次不同 ⇒ 它是活的）。
    let (t0, t1) = (device::now(dev.view()), device::now(dev.view()));
    debug!("rtc: time {t0} -> {t1}");

    // 占线（契里那个号；"这台是哪条线"那条权威在设备账那一台）。
    let line = ctx
        .line(deed.line, Wait::AtMost(MS))
        .map_err(|_| Fail::at(E_RTC, "line"))?;
    debug!("{ME}: line occupied");

    ctx.plate(ME, Mine::No, Wait::AtMost(MS));

    // **报"答得动了"**（`Setup::Ready`）：牌子落了才算——装配者等它才往下起别人，于是"排在第几号"
    let _ = protocol::communication::establish::endpoint(
        runtime::env::unit::sire(),
        env::Mark::of(programs::unit::READY),
        env::Wait::POLL,
    );

    adapt::resident::run(&ctx, &dev, line, &mut Host::new())
}
