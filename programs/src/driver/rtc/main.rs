#![no_std]
#![no_main]

//! rtc@101000 的持有者（设备树里那条 11 号线），兼报时服务。
//! 它存在的理由原来是一条**判据**（线那四格、配给、门牌、设备面这一整套，只有 `uart` 一台
//! 第二个走进服务面协议的真实实例。
//! 手的壳在 `adapt/{desk,resident}.rs`；会话核（纯）在 programs::driver::rtc::core::host。

extern crate alloc;
extern crate programs;

/// 住持面（适配）：门面 / 常驻——**只属于这一台**，故由 bin 自己 `mod`
mod adapt;

mod dev;

use dev::rtc as device;
use env::{Access, PieKind, Policy, Wait};
use programs::driver::rtc::core::Host;
use programs::driver::shared::context::{Context, Step};
use programs::driver::shared::device::{Ask, Device, Hub};
use programs::driver::shared::fail::Fail;
use programs::unit::rtc::E_RTC;
use protocol::debug;
use protocol::driver::ENTRY_MARK;
use protocol::system::operator::client as operator;
use env::unit;
use env::pie;

const ASK: Ask = Ask {
    class: "google,goldfish-rtc",
    name: None,
    kind: PieKind::Pole,
    access: Access::FETCH_STORE,
    policy: Policy::ONLY,
};

const ME: &str = "rtc";

/// 等板 / 等树 / 办一趟登记的总上限（毫秒）。**必须有界**
const MS: usize = 1000;

/// （**一族口径**在 programs::driver::shared::fail：号取自装配表——一个数都不写）
#[programs::entry]
fn main() -> Result<(), Fail> {
    // 解门牌 → 上板 ＋ 开会话 → 上树落门牌。门牌**公开可查**（Mine::No）：谁都能查、谁都能用。
    let entry = pie::unseal_hole(ENTRY_MARK).map_err(|_| Fail::at(E_RTC, "tree"))?;
    let ctx = Context::open(unit::sire(), Wait::AtMost(MS)).map_err(|s| {
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

    ctx.plate(entry, ME, Wait::AtMost(MS));

    // **报"答得动了"**（Setup::Ready）：牌子落了才算——装配者等它才往下起别人，于是"排在第几号"
    let _ = protocol::communication::session::establish::endpoint(
        env::unit::sire(),
        env::Mark::of(programs::unit::READY),
        env::Wait::POLL,
    );

    adapt::resident::run(entry, &dev, line, &mut Host::new())
}
