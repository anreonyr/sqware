//! router::adapt::boot — **起手**：领配给 → 开两图 → 读树 → 建账 → 铸入口 → 上板 ＋ 上树 → 挂组。
//!
//! 起手的产物是**同一条命**（控制器的事实 / 账 / 门铃 / 组 / 门外那一页缓冲），故合成一个
//! 类型 [`Up`]：常驻那一圈每醒一次用到的就是它。

use super::fail;
use crate::core::sources::Sources;
use crate::plic::Plic;
use alloc::vec::Vec;
use env::{HoleDir, PieToken, TaskId, Wait};
use plan::assembly::ROUTER_WANTS as WANTS;
use programs::driver::assemble;
use programs::driver::tree::{self, Mine};
use protocol::debug;
use protocol::driver::line::core::Lines;
use protocol::system::board::client as board;
use protocol::system::operator::client as operator;
use runtime::PAGE_SIZE;
use runtime::core::bell::Bell;
use runtime::core::dock::Dock;
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie, NolePie, PolePie};
use runtime::env::unit as utask;

/// 本域挂在树上的名字（`/device/router`，[`protocol::driver::DIR`] 之下的那一段）。
const SERVICE: &str = "router";

/// 装泊位 / 等配给 / 办一趟登记 / 上树的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// 起手那几步的产物：本域要活下去的全部凭据。
pub struct Up {
    /// 控制器寄存器面（设备侧）。
    pub plic: Plic,
    /// 树那侧解出来的事实与线集合（纯核心）。
    pub sources: Sources,
    /// 账：线号 = 下标（容量按 `device_count` 校验 ⇒ 越界不可表达）。
    pub lines: Lines,
    /// 门铃（内核给的那一枚；它只 `hush`，不铸）。
    pub bell: Bell,
    /// 等三源的组。
    pub pile: Pile,
    /// 门外那一页缓冲（取消息用；**按载体备**，见 `resident`）。
    pub buf: Vec<u8>,
    /// 本域的服务入口（门牌那枚孔，本线程铸、本线程读）。
    pub entry: HolePie,
}

/// 起手。
pub fn up() -> Result<Up, fail::Fail> {
    // 客侧装配：会话 + 收配给（**编号原样带出去**——`assemble` 报的是"死在装配的哪一步"，
    // 折成同一个号就等于把那几个编号变成没人读得到的死码）。
    let mut slots = [None; WANTS.len()];
    let got = assemble::receive(&mut slots)?;
    // 三枚都要在：少一枚就不必继续（父域按同一张单子发货，缺格即装配错）。
    let [Some(plic_pie), Some(dtb_pie), Some(bell_pie)] = slots else {
        return Err(fail::Fail::Assemble(assemble::E_GRANT));
    };
    debug!("router: got {got}");

    // 开图 + 读树：控制器、本域的 context、要接的线（与"没进来的账"）。
    let plic_dock =
        Dock::open(PolePie::from_token(plic_pie.token())).map_err(|_| fail::Fail::Open)?;
    let dtb_dock =
        Dock::open(PolePie::from_token(dtb_pie.token())).map_err(|_| fail::Fail::Open)?;
    let dtb = dtb_dock.view();
    // SAFETY: 设备树是内核只读借映进本域的整棵（保留区，终身存活）；`Sources::of` 只读它。
    let bytes = unsafe { core::slice::from_raw_parts(dtb.base() as *const u8, dtb.size()) };
    let sources = Sources::of(bytes).ok_or(fail::Fail::Tree)?;
    let plic = Plic::new(plic_dock.view(), &sources);
    debug!("router: docks open");
    // 线集合与五笔"没进来的账"——这台机器上有哪些中断源，唯一一次陈述。
    debug!(
        "router: device_count={} ctx={} lines={:?} unparented={} beyond={} mapped={} unparsed={} unregion={}",
        sources.device_count(),
        sources.context(),
        sources
            .lines
            .iter()
            .map(|s| s.line)
            .collect::<alloc::vec::Vec<u32>>(),
        sources.unparented,
        sources.beyond,
        sources.mapped,
        sources.unparsed,
        sources.unregion
    );
    let bell = Bell::new(NolePie::from_token(bell_pie.token()));

    // 账：格数按控制器自报的线数要，装不下 ⇒ 拒起（"领到的线一定记得下"是构造性事实）。
    // **起域时一条都不接**：接线是登记的直接后果（见 `driver/router/mod.rs`）。
    let lines = Lines::new(sources.device_count()).ok_or(fail::Fail::Account)?;

    // 服务入口：本线程铸、本线程读——**它就是树上那块门牌**。
    //
    // 线那一面（账 + 各家客户的泊位）与入口同住这一张表：`PieToken` 只在铸它的那张表里
    // 念得出来，而客户往门里推、路由者往客户手里推——两端都得在同一张表里，故这里不再有
    // 第二枚线程。
    let entry = mail::unseal_hole(board::ENTRY_MARK).map_err(|_| fail::Fail::Desk)?;

    // 板那趟（装上板路、交上问话孔——只为让板看得见本域的死）+ 上树那趟（门牌）。
    let sire = utask::sire();
    serve_board(sire, entry);

    // 等三个源：**铃**（外部中断）、**门上有人**（登记）、**客人的排空**（每登记一条线
    // 就把那位客户的泊位挂进来，见 `desk`）。一只组同时等这三样——三件都是事件，
    // 故等待**没有期限**（见 `resident` 里那一注）：会丢的那一次铃已在根上修掉。
    let pile = Pile::unseal(false).map_err(|_| fail::Fail::Bell)?;
    let entry_hole = HolePie::from_token(entry);
    if pile
        .attach(&NolePie::from_token(bell_pie.token()), HoleDir::Pull)
        .is_err()
        || pile.attach(&entry_hole, HoleDir::Pull).is_err()
    {
        return Err(fail::Fail::Bell);
    }

    // 一问的形状是 `lcall::Occupy::LEN`；缓冲给**一页**（载体的界，见 `Push` 的前置条件）。
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(fail::Fail::Desk);
    }
    buf.resize(PAGE_SIZE, 0);

    Ok(Up {
        plic,
        sources,
        lines,
        bell,
        pile,
        buf,
        entry: entry_hole,
    })
}

/// 板那趟 + 上树那趟：**不向板挂牌**（板只管生死），门牌挂树上。
///
/// **它不拦主循环**：两件都是"起来之后"的事——哪一件没成只报一句读数，收与结照旧。
fn serve_board(sire: TaskId, entry: PieToken) {
    // 板那条路：本端装一条、认下生我者那一枚（它再转授给板线程），再交一枚问话孔——
    // 不交的那一位在板账上永远"没挂齐"，板线程会一直退化成 1 ms 节拍。
    let link = board::open(sire, Wait::AtMost(QUAY_MS)).ok();
    let boarded = match &link {
        Some((_, board)) => board::ask_hole(*board).is_ok(),
        None => false,
    };
    if !boarded {
        debug!("router: board: no link");
    }
    // 上树：本域的门牌 = `/device/router`（名字用服务名，见 [`protocol::driver::DIR`]）。
    // **开会话那两步留在这里**（那一趟同构的部分在 [`tree::plate`]）：拿不到会话就只报一行、
    // 不拦主循环——收与结照旧。
    let Ok((link, host)) = operator::open(sire, Wait::AtMost(QUAY_MS)) else {
        debug!("router: tree: no lane");
        return;
    };
    let Ok(talk) = operator::ask_hole(host) else {
        debug!("router: tree: no ask");
        return;
    };
    tree::plate(
        SERVICE,
        Mine::No,
        &link,
        talk,
        host,
        entry,
        Wait::AtMost(QUAY_MS),
    );
}
