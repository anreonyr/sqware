//! uart::adapt::boot — **起手 1–3**：领配给 → 开图开闸 → 上板（并开树那条会话）。
//!
//! 三步的产物是**同一条命**（设备映射 ＋ 树那条会话 ＋ 读行那枚孔），故合成一个类型 [`Up`]：
//! 上树、登记、常驻都从它取件。装会话那一步也在这里，因为**同一个域只开一条**——上树与登记共用它。

use super::fail::{Fail, Step};
use crate::uart as device;
use env::{PieToken, TaskId, Wait};
use plan::assembly::UART_WANTS as WANTS;
use programs::driver::assemble;
use protocol::debug;
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
    /// 那一段区（内核按 `reg` 段造的门闩给的坐标；登记要用它，本域不写死它）。
    pub key: plan::Key,
    /// 那一页的映射（域活多久它活多久）。
    pub dock: Dock,
    /// 读行那枚孔——**它就是本域的门牌**（服务入口）。
    pub entry: PieToken,
    /// 树那条会话（上树与登记**共用这一条**）。
    pub link: Quay,
    /// 会话上那枚问话孔。
    pub talk: PieToken,
    /// 持树者。
    pub host: TaskId,
}

impl Up {
    /// 设备那一页的视图（[`View`] 是 `Copy`：常驻那一圈每醒一次取一份）。
    pub fn view(&self) -> View {
        self.dock.view()
    }
}

/// 起手 1–3 ＋ 树那条会话。
///
/// 1. 客侧装配：父域按本域那张单子把 `ns16550a` 那一台授进来（坐标由它读树定下来）；
/// 2. 开图 ＋ 开闸：**设备到手之后第一件要打开的就是"收到字节就拉线"**（这条线归本域，
///    因为只有持有设备的人才有资格动它）；
/// 3. 上板：**只为让板看得见本域的死**（本域开的那扇门随收尾封印 ⇒ 板当场看出来）。
pub fn up() -> Result<Up, Fail> {
    // 1. 客侧装配。
    let mut slots = [None; WANTS.len()];
    let got = assemble::receive(&mut slots)?;
    // 单子上只有一条，缺了它就没得开工（父域按同一张单发货，缺格即装配错）。
    let [Some(serial)] = slots else {
        return Err(Fail::at(Step::Assemble(assemble::E_GRANT)));
    };
    debug!("uart: got {got}");

    // 2. 开图 + 开闸。
    let dock = Dock::open(PolePie::from_token(serial.token())).map_err(|_| Fail::at(Step::Open))?;
    device::arm_rx(dock.view());
    // 坐标**随记录发下来**（内核按 `reg` 段造的门闩；本域既不写死名字、也不写死地址）。
    let key = serial.key().ok_or(Fail::at(Step::Open))?;
    // 设备门的坐标只能是区（内核就是按 `reg` 段造的）；别的形就是配给错了。
    let base = key.base().ok_or(Fail::at(Step::Open))?;
    debug!("uart: ier=rx at={base:#x}");

    // 3. 上板：**只为让板看得见本域的死**；不挂牌子——名字在树上。**问话孔照交**：不交的那一位
    //    在板账上永远"没挂齐"，板线程会一直退化成 1 ms 节拍（`board::settle` 的 `unarmed`）。
    let sire = utask::sire();
    let (_link, board) = board::open(sire, Wait::AtMost(MS)).map_err(|_| Fail::at(Step::Board))?;
    if board::ask_hole(board).is_err() {
        return Err(Fail::at(Step::Board));
    }

    // 树那条会话：**只能开一条**（`operator::open` 装的是"一条叫 `operator` 的泊位"，同一个域开
    // 第二条会撞同名；而两次 `open` 拿到的是两条*不同*的会话，孔各归各的表，混用更糟）——
    // 实测栽过：第二趟 `open` 失败 ⇒ 本域当场退出，客人那一侧读一枚封了的孔，一个字节都读不到。
    let (link, host) = operator::open(sire, Wait::AtMost(MS)).map_err(|_| Fail::at(Step::Tree))?;
    let talk = operator::ask_hole(host).map_err(|_| Fail::at(Step::Tree))?;
    let entry = mail::unseal_hole(board::ENTRY_MARK).map_err(|_| Fail::at(Step::Tree))?;
    Ok(Up {
        key,
        dock,
        entry,
        link,
        talk,
        host,
    })
}
