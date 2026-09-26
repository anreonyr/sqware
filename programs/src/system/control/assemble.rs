//! control::assemble — **装配那一半**：先立账（`enlist`），后递单（`wire`）。
//!
//! 两条都是"机器对整张单做的事"，与具体是哪一台无关：
//!   - [`Control::enlist`]：登记一行——**怎么算起来了**由这一行的 `setup` 推出
//!     （有通道 ⇒ `Announce::Channel`；否则放行即起来）。**先立整张账，再逐条起**。
//!   - [`Control::wire`]：按 `setup` 里那几条 `Need` 定坐标、向引导域领，再把这**一段字节
//!     原样**推到客人那条通道上（客人按位次归位——位置即格）。
//!
//! **本域不碰原件**：门闩在引导域手里，它直接授进 `task` 那张表，回一段"坐标 + 号"的记录；
//! 本域只做一次转投（[`protocol::system::grant::each`]）。
//!
//! **翻坐标是本域唯一解释机器自述的地方**：类（`compatible`）是收方写的，翻成哪一段区是
//! 树说的——两半在这条线上合拢，故那条权威只在这里（`system: <程序> <类> -> <区>` 就是它的读数）。

use env::{Name, Wait};
use protocol::debug;
use protocol::system::desk::Announce;

use protocol::driver::supply::frame::{WANT_MAX, Want};
use protocol::driver::supply;

use crate::system::program::Setup;

use super::{Control, Error, READY_MS, Service};

impl Control {
    /// **登记一行**：只知道名字与它"怎么算起来"——此刻还没有身子（`spawn` 才挂）。
    ///
    /// **"怎么算起来"由 `setup` 推出**：有 `Channel` ⇒ [`Announce::Channel`]（它起来时会交回
    /// 一枚孔，那枚到了才算起来）；否则 [`Announce::None`]（放行即起来）。这与旧装配单上那
    /// 两格（`announce` ＋ `channels`）**逐行等价**：有通道的那四台正是旧表里唯一写
    /// `Announce::Channel` 的四台。
    pub fn enlist(&mut self, name: &'static str, setup: &'static [Setup]) -> Result<(), Error> {
        let name = Name::new(name).map_err(|_| Error::Manifest)?;
        self.table
            .register(name, announce_of(setup))
            .map_err(|_| Error::Table)
    }

    /// **递单**：只对"要资源"的那几台动手（`Need` 一条都没有 ⇒ 立即返回）。
    ///
    /// 前置：它**已经起来**（[`Control::start`] 之后）——配给记录要落到它交回的那条路上。
    pub fn wire(
        &self,
        name: Name,
        service: &Service,
        setup: &'static [Setup],
    ) -> Result<(), Error> {
        let (task, quay) = service;
        // 门闩单：按 `setup` 的次序（= 收方那张单的次序）。条数上限那一格与 `draw` 同一条
        // （单子装不下）：这里先拦，好按定长缓冲逐格填。
        let mut wants = [Want::NONE; WANT_MAX];
        let mut n = 0usize;
        for s in setup {
            if let Setup::Need(need) = s {
                if n >= WANT_MAX {
                    return Err(Error::Step("too many wants"));
                }
                let class = need.class_name();
                wants[n] = need
                    .settle(|class| self.machine.site_of(class))
                    .ok_or(Error::Step("class not in tree"))?;
                if let (Some(class), Some(base)) = (class, wants[n].key().and_then(|key| key.base())) {
                    // 新机制要有读数：**类 → 那一段区**（翻译那一手看得见、可复核）。
                    debug!(
                        "system: {} {} -> {:#x}",
                        name.as_str(),
                        class.as_str(),
                        base
                    );
                }
                n += 1;
            }
        }
        if n == 0 {
            return Ok(());
        }

        // 递单走**第一条**通道（旧 `wire` 读的就是 `channels.first()`）。
        let Some(ch) = setup.iter().find_map(|s| match s {
            Setup::Channel(c) => Some(*c),
            _ => None,
        }) else {
            return Err(Error::Step("no channel"));
        };
        let ch = Name::new(ch).map_err(|_| Error::Step("no channel"))?;
        let Some(pier) = quay.find(ch) else {
            return Err(Error::Step("no channel"));
        };
        if !pier.paired() {
            return Err(Error::Step("no channel"));
        }

        // 一枚一枚要：条数就在那张表里，本层不抄"要几样"。
        // **编单子那只缓冲在 `draw` 里头**（一族最长那一只，见 `Message::Buf`）——这一层只备收的那一只。
        let mut reply = [0u8; supply::REPLY_CAP];
        let records = supply::client::draw(
            &self.boot,
            *task,
            &wants[..n],
            &mut reply,
            Wait::AtMost(READY_MS),
        )
        .map_err(|_| Error::Step("draw failed"))?;
        let said = pier.post(records);
        debug!(
            "wire: {} bytes, paired={}, post={}",
            records.len(),
            pier.paired(),
            said.is_ok()
        );
        said.map_err(|_| Error::Step("no channel"))
    }
}

/// **怎么算"它起来了"**：由这一行的 `setup` 推出（见 [`Control::enlist`]）。
fn announce_of(setup: &[Setup]) -> Announce {
    if setup.iter().any(|s| matches!(s, Setup::Channel(_))) {
        Announce::Channel
    } else {
        Announce::None
    }
}
