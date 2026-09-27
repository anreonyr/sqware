//! router::adapt::boot — **起手**：领配给 → 开两图 → 读树 → 建账 → 铸入口 → 上板 ＋ 上树 → 挂组。
//!
//! 起手的产物是**同一条命**（控制器的事实 / 账 / 门铃 / 组 / 门外那一页缓冲），故合成一个
//! 类型 [`Up`]：常驻那一圈每醒一次用到的就是它。
//!
//! **三段里的前两段已住 [`programs::driver`]**（领配给、开图、上板、上树那几步三台同构）。
//! 本文件剩下的是**路由者自己的起手**：读设备树（只有它读）、建账、铸入口、挂三源那只组。
//!
//! **上板 / 上树仍然尽力**：这一台起来就得收（铃一响就要 claim），故两件任一件没成都只报一行
//! 读数、不拦主循环。**照实记（三行并成一行）**：从前板那趟与树那趟各报各的
//! （`router: board: no link` / `router: tree: no lane` / `router: tree: no ask`）；收进
//! [`Context::join`] 之后只剩**一行**。健康机器上那三行本来都不出现，故验收读数不受影响。

use crate::core::lines::Lines;
use crate::core::sources::Sources;
use crate::plic::Plic;
use alloc::vec::Vec;
use env::{HoleDir, Wait};
use programs::driver::context::Context;
use programs::driver::device::Device;
use programs::driver::fail::Fail;
use programs::program::router::{E_ROUTER, ROUTER_WANTS as WANTS};
use protocol::debug;
use programs::system::board::client as board;
use protocol::system::operator::client::Mine;
use runtime::PAGE_SIZE;
use runtime::core::bell::Bell;
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie, NolePie};
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
pub fn up() -> Result<Up, Fail> {
    // 客侧装配：收配给（**编号原样带出去**——`take` 报的是"死在装配的哪一步"，折成同一个号就
    // 等于把那几个编号变成没人读得到的死码）。三枚都要在（`N` 就是那张单子的长度）。
    let [plic_pie, dtb_pie, bell_pie] = Device::claim::<{ WANTS.len() }>()?;
    debug!("router: got {}", WANTS.len());

    // 开图 + 读树：控制器、本域的 context、要接的线（与"没进来的账"）。
    let plic_dev = Device::open(plic_pie).map_err(|_| Fail::at(E_ROUTER, "docks"))?;
    let dtb_dev = Device::open(dtb_pie).map_err(|_| Fail::at(E_ROUTER, "docks"))?;
    let dtb = dtb_dev.view();
    // SAFETY: 设备树是内核只读借映进本域的整棵（保留区，终身存活）；`Sources::of` 只读它。
    let bytes = unsafe { core::slice::from_raw_parts(dtb.base() as *const u8, dtb.size()) };
    let sources = Sources::of(bytes).ok_or(Fail::at(E_ROUTER, "tree"))?;
    let plic = Plic::new(plic_dev.view(), &sources);
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
    let lines = Lines::new(sources.device_count()).ok_or(Fail::at(E_ROUTER, "line account full"))?;

    // 服务入口：本线程铸、本线程读——**它就是树上那块门牌**。
    //
    // 线那一面（账 + 各家客户的泊位）与入口同住这一张表：`PieToken` 只在铸它的那张表里
    // 念得出来，而客户往门里推、路由者往客户手里推——两端都得在同一张表里，故这里不再有
    // 第二枚线程。
    let entry = mail::unseal_hole(board::ENTRY_MARK).map_err(|_| Fail::at(E_ROUTER, "desk"))?;

    // 板那趟（装上板路、交上问话孔——只为让板看得见本域的死）+ 上树那趟（门牌 /device/router）。
    // **尽力**：任一件没成都只报一行读数、不拦主循环——这一台起来就得收（见文件头那一条照实记）。
    //
    // **照实记（上树那一手改经 `Context` 走，task-2 那一刀）**：从前这里直接叫
    // `operator::plate(&ctx.session, …)`（协议层的自由函数）。那一手已按"一个组合动作只有一个
    // 实现消费者就不强升为协议"的裁定下移成 [`Context::plate`]——本域与 `rtc` 走的是**同一手**，
    // 只是本域不占线（故不能走 `Context::enter`，见文件头）。
    let sire = utask::sire();
    match Context::join(entry, sire, Wait::AtMost(QUAY_MS)) {
        Ok(ctx) => ctx.plate(SERVICE, Mine::No, Wait::AtMost(QUAY_MS)),
        Err(_) => debug!("router: board/tree: no link"),
    }

    // 等三个源：**铃**（外部中断）、**门上有人**（登记）、**客人的排空**（每登记一条线
    // 就把那位客户的泊位挂进来，见 `desk`）。一只组同时等这三样——三件都是事件，
    // 故等待**没有期限**（见 `resident` 里那一注）：会丢的那一次铃已在根上修掉。
    let pile = Pile::unseal(false).map_err(|_| Fail::at(E_ROUTER, "bell"))?;
    let entry_hole = HolePie::from_token(entry);
    if pile
        .attach(&NolePie::from_token(bell_pie.token()), HoleDir::Pull)
        .is_err()
        || pile.attach(&entry_hole, HoleDir::Pull).is_err()
    {
        return Err(Fail::at(E_ROUTER, "bell"));
    }

    // 一问的形状是 `lcall::Occupy::LEN`；缓冲给**一页**（载体的界，见 `Push` 的前置条件）。
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Fail::at(E_ROUTER, "desk"));
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
