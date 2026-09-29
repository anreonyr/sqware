//! system::bootstrap — **启动资源获取**：与引导域搭会话、领机器自述、领那块载荷区（清单）。
//!
//! 这里只做"把起手要的几样拿到手"，**与 Control 分开**：拿到之后交给 `System`，
//! 装配与监督一概不在这里。
//!
//! ```text
//!   talk_to_root()   与引导域搭一条双向的问答路（配给从这条路上领）
//!   take_machine()   领机器自述（设备树）——单子上写的是"类"，翻成"哪一段区"要有它
//!   take_catalog()   领那块载荷区（清单 + 全部镜像，零拷贝借映）
//! ```
//!
//! **配给从哪来**：装配者自己不持设备门闩——它在引导域手里。故发货走一次往返：
//! [`protocol::system::supply::client::draw`] 把"要哪几样"递过去，固件把门闩直接授进**客人**的表里
//! 并回一段记录，装配者再把这**一段字节原样**投到客人那条通道上（那一手在
//! [`Control::wire`](crate::system::control::Control::wire)）。

use env::Mark;
use env::Wait;
use protocol::communication::establish::{self, Endpoint};

use protocol::system::supply;
use protocol::system::supply::frame::{Kind, Want};
use runtime::core::dock::Dock;
use runtime::core::port::{Access, Policy};
use runtime::env::mail::PolePie;
use runtime::env::unit as utask;

use crate::system::control::Catalog;
use crate::system::machine::Machine;

/// 结算两条上限（毫秒）：与引导域开会话、以及领那两样。
const BOOT_MS: usize = 1000;

/// **引导那一族的死法**：一格 = 死在起手的哪一步。
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
}

impl Fail {
    /// 本族共用的号（沿用旧 `system::Fail` 那几格）。
    pub fn code(self) -> env::Reason {
        match self {
            Fail::Firmware => E_BOOT,
            Fail::Machine => 5,
            Fail::Payload => 6,
            Fail::Manifest => 7,
        }
    }

    pub const fn text(self) -> &'static str {
        match self {
            Fail::Firmware => "system: no firmware",
            Fail::Machine => "system: no machine",
            Fail::Payload => "system: no payload",
            Fail::Manifest => "system: manifest bad",
        }
    }
}

/// 引导那一族共用的号（"启动参数读不出来"那一格）。
use crate::program::system::E_BOOT;

/// **起手要的三样东西**：与引导域的会话、那台机器的自述、那块载荷区（清单）。
pub struct Boot {
    /// 与引导域那条双向问答路（配给从这条路上领）。
    pub pier: Endpoint,
    /// 本域手里那台机器的自述——单子上那一格写的是**类**，翻成"哪一段区"要有它。
    pub machine: Machine,
    /// 这块字节里**清单与全部镜像都在里头**（同一批物理页，借映进本域的 VA）。
    pub catalog: Catalog<'static>,
}

/// **领全套**：失败一格 = 死在起手的哪一步（与旧 `system()` 那几步逐格对应）。
pub fn take() -> Result<Boot, Fail> {
    // 1. 与引导域开会话：本域那一枚交给"生我者"，并认下它那一枚（一问一答两个方向）。
    let pier = talk_to_root().ok_or(Fail::Firmware)?;

    // 2. 领树：坐标是 `Key::dtb()`。
    let machine = take_machine(&pier).map_err(|_| Fail::Machine)?;

    // 2′. 领账：载荷区的**坐标从树里读**（`/chosen` 的 `linux,initrd-start`）。
    let Some(payload) = machine.payload() else {
        return Err(Fail::Payload);
    };
    let catalog = take_catalog(&pier, payload).map_err(|_| Fail::Machine)?;

    Ok(Boot {
        pier,
        machine,
        catalog,
    })
}

/// 与引导域搭一条**双向**的问答路。
///
/// **一手就是"两头都装"**（[`establish::endpoint`]）：本域铸一枚（刻 `boot` 的记号）交给生我者
/// ——本域**读**自己那一枚（回单从这来）——并认下它那一枚（**写**：单子往那去）。
fn talk_to_root() -> Option<Endpoint> {
    let sire = utask::sire();
    let pier = establish::endpoint(sire, Mark::of(supply::BOOT), Wait::AtMost(BOOT_MS)).ok()?;
    // **认不到它那一枚 = 这条问答路没搭上**：单子发不出去（原 `claim` 那一格）。
    pier.tx()?;
    Some(pier)
}

/// 领树：与载荷区同一条路（一张只有一条的单子 + 借映）。
fn take_machine(pier: &Endpoint) -> Result<Machine, ()> {
    let want = Want::new(env::Key::dtb(), Kind::Pole, Access::FETCH, Policy::NONE);
    let token = draw_one(pier, want).ok_or(())?;
    let dock = Dock::open(PolePie::from_token(token)).map_err(|_| ())?;
    Machine::of(dock.view()).map_err(|_| ())
}

/// 领那块载荷区并把清单读出来。坐标是**机器自己在树里写的那一段**（`/chosen`）。
///
/// **零拷贝**：那几十 MB 不是搬过来的，是同一批物理页借映进本域。
fn take_catalog(pier: &Endpoint, key: env::Key) -> Result<Catalog<'static>, ()> {
    let want = Want::new(key, Kind::Pole, Access::FETCH, Policy::NONE);
    let token = draw_one(pier, want).ok_or(())?;
    let dock = Dock::open(PolePie::from_token(token)).map_err(|_| ())?;
    let view = dock.view();
    // SAFETY: 这段借映在**本域存活期间**一直有效（门闩在本域表里，本域到收场才退出）；
    // 视图只读（`FETCH`），本域只解析、不写。
    let blob: &'static [u8] =
        unsafe { core::slice::from_raw_parts(view.base() as *const u8, view.size()) };
    Catalog::new(blob).ok_or(())
}

/// 问引导域要一枚：递一张只有一条的单子，取回那一条的号（按**坐标**认，不按位次）。
///
/// 缓冲是本调用的局部（**一问一答**，一问一次）；引导期只发生两次。
fn draw_one(pier: &Endpoint, want: Want) -> Option<env::PieToken> {
    let me = utask::self_id();
    let key = want.key()?;
    let mut reply = [0u8; supply::REPLY_CAP];
    let records =
        supply::client::draw(pier, me, &[want], &mut reply, Wait::AtMost(BOOT_MS)).ok()?;
    supply::client::pick(records, key)
}
