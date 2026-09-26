//! rtc::adapt::boot — **起手 1–3**：领配给归位 → 开图自证 → 上板（并开树那条会话）。
//!
//! 三步的产物是**同一条命**（设备映射 ＋ 树那条会话 ＋ 服务入口），故合成一个类型 [`Up`]：
//! 后面的门面、上树、常驻都从它取件。装会话那一步也在这里，因为**同一个域只开一条**
//! （`operator::open` 装的是"一条叫 `operator` 的泊位"，开第二条会撞同名）——上树与登记共用它。

use super::fail;
use crate::rtc;
use crate::say;
use alloc::format;
use env::{PieToken, TaskId, Wait};
use plan::Pair;
use plan::assembly::RTC_WANTS as WANTS;
use programs::driver::assemble;
use protocol::session::Quay;
use protocol::system::board::client as board;
use protocol::system::operator::client as operator;
use runtime::core::dock::{Dock, View};
use runtime::env::mail::{self, PolePie};
use runtime::env::unit as utask;

/// 等板 / 等树的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起手那几步的产物：本域要活下去的全部凭据。
pub struct Up {
    /// 那一台设备那条记录（坐标 ＋ 号）：开图与取"那一段区"都从它来。
    pub pie: Pair,
    /// 那一页的映射（域活多久它活多久——与原先 `main` 手里那条命同）。
    pub dock: Dock,
    /// 服务入口（上树 ＋ 挂组共用）。
    pub entry: PieToken,
    /// 树那条会话（上树与登记**共用这一条**）。
    pub link: Quay,
    /// 会话上那枚问话孔。
    pub talk: PieToken,
    /// 持树者（`land` 要它）。
    pub host: TaskId,
}

impl Up {
    /// 设备那一页的视图（[`View`] 是 `Copy`：门面与常驻各取一份，同一张页表）。
    pub fn view(&self) -> View {
        self.dock.view()
    }
}

/// 起手 1–3。
///
/// 1. 领配给：那一页寄存器（`ONLY`：同一时刻只该有一个持有者）；
/// 2. 开图 ＋ **自证**：那对纳秒格子读两次（两次不同 ⇒ 它是活的）；
/// 3. 上板（**只为让板看得见本域的死**）＋ 开树那条会话。
pub fn up() -> Result<Up, fail::Fail> {
    // 1. 领配给。
    let mut slots = [None; WANTS.len()];
    let got = assemble::receive(&mut slots)?;
    // 单子上只有一条，缺了它就没得开工（父域按同一张单发货，缺格即装配错）。
    let [Some(pie)] = slots else {
        return Err(fail::Fail::Assemble(assemble::E_GRANT));
    };
    say(&format!("rtc: got {got}"));

    // 2. 开图 + 自证。
    let dock = Dock::open(PolePie::from_token(pie.token())).map_err(|_| fail::Fail::Open)?;
    let view = dock.view();
    let (t0, t1) = (rtc::now(view), rtc::now(view));
    say(&format!("rtc: time {t0} -> {t1}"));

    // 3. 上板 + 树那条会话。
    let sire = utask::sire();
    let (_link, board_link) = board::open(sire, Wait::AtMost(MS)).map_err(|_| fail::Fail::Board)?;
    if board::ask_hole(board_link).is_err() {
        return Err(fail::Fail::Board);
    }
    let entry = mail::unseal_hole(board::ENTRY_MARK).map_err(|_| fail::Fail::Tree)?;
    let (link, host) = operator::open(sire, Wait::AtMost(MS)).map_err(|_| fail::Fail::Tree)?;
    let talk = operator::ask_hole(host).map_err(|_| fail::Fail::Tree)?;
    Ok(Up {
        pie,
        dock,
        entry,
        link,
        talk,
        host,
    })
}
