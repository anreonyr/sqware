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
use super::desk::Replies;
use crate::core::sources::Sources;
use crate::plic::Plic;
use alloc::vec::Vec;
use env::{Access, HoleDir, Kind, Policy, Wait};
use programs::driver::context::{Context, Step};
use programs::driver::device::{Ask, Device, Hub};
use programs::driver::fail::Fail;
use programs::program::router::{E_ROUTER, PLIC_CLASS};
use programs::system::board::client as board;
use protocol::debug;
use protocol::driver::hub as hcall;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Mine;
use runtime::PAGE_SIZE;
use runtime::core::bell::Bell;
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie, NolePie};
use runtime::env::unit as utask;

/// 本域挂在树上的名字（`/svc/drv/router`，[`protocol::driver::ROAD`] 之下的那一段）。
const SERVICE: &str = "router";

/// 本域要认的三样：**中断控制器**（按类）＋ **设备树本体 / 门铃**（点名——那两件的名字是常量，
/// 它们不是树里的设备）。
///
/// **照实记（这三条从前住装配表）**：它们是本域那张需求单（`ROUTER_WANTS`）里那三格——装配者按
/// 同一张单替本域领三样。那一整条路退了 ⇒ 单子回了它自己的域（类那一格仍是同一枚
/// [`PLIC_CLASS`]，读树那一侧也用同一枚）。
const PLIC_ASK: Ask = Ask {
    class: PLIC_CLASS,
    name: None,
    kind: Kind::Pole,
    access: Access::FETCH_STORE,
    policy: Policy::ONLY,
};
const DTB_ASK: Ask = Ask {
    class: hcall::BOOT,
    name: Some(hcall::DTB),
    kind: Kind::Pole,
    access: Access::FETCH,
    policy: Policy::NONE,
};
const IRQ_ASK: Ask = Ask {
    class: hcall::BOOT,
    name: Some(hcall::IRQ),
    kind: Kind::Nole,
    access: Access::FETCH,
    policy: Policy::NONE,
};

/// 装泊位 / 等配给 / 办一趟登记 / 上树的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// 起手那几步的产物：本域要活下去的全部凭据。
pub struct Up {
    /// 控制器寄存器面（设备侧）。
    pub plic: Plic,
    /// 账：线号 = 下标（容量按 `device_count` 校验 ⇒ 越界不可表达）。
    pub lines: Lines,
    /// 门铃（内核给的那一枚；它只 `hush`，不铸）。
    pub bell: Bell,
    /// 等三源的组。
    pub pile: Pile,
    /// 门外那一页缓冲（取消息用；**按本族最长那一枚备足**，见 `resident`）。
    pub buf: Vec<u8>,
    /// 本域的服务入口（门牌那枚孔，本线程铸、本线程读）。
    pub entry: HolePie,
    /// 一格一格的**答话存根**（见 [`desk::Reply`]）：那一等不许落在本域这条循环里。
    pub replies: Replies,
}

/// 起手。
pub fn up() -> Result<Up, Fail> {
    // **起手第一件：入系统**（服务入口 → 上板 ＋ 开会话 → 上树落门牌）。
    //
    // **照实记（"上板 / 上树仍然尽力"那条口径在这一刀上翻了面）**：从前这两件任一件没成都只报
    // 一行读数、不拦主循环——因为本域起来就得收（铃一响就要 `claim`），而设备那三样是装配期直授
    // 的。这一刀之后**设备要从树上找**（`/dev/<类>/<名>`），而"这台归谁"由设备账（也住树那一
    // 层）回答 ⇒ 这条会话没了就没有设备，故它是**硬前置**（两格各报自己的步名）。
    let entry = mail::unseal_hole(board::ENTRY_MARK).map_err(|_| Fail::at(E_ROUTER, "desk"))?;
    let sire = utask::sire();
    let ctx = Context::join(entry, sire, Wait::AtMost(QUAY_MS)).map_err(|s| {
        Fail::at(
            E_ROUTER,
            match s {
                Step::Board => "board",
                Step::Tree => "tree",
            },
        )
    })?;
    // 设备那一趟：找设备账那两枚面 → 认三样（控制器 / 设备树 / 门铃）。
    let tree = operator::Face::from(&ctx.session);
    let hub = Hub::find(&tree, E_ROUTER, Wait::AtMost(QUAY_MS))?;
    let plic_deed = hub.claim(&tree, &PLIC_ASK, E_ROUTER, Wait::AtMost(QUAY_MS))?;
    let dtb_deed = hub.claim(&tree, &DTB_ASK, E_ROUTER, Wait::AtMost(QUAY_MS))?;
    let irq_deed = hub.claim(&tree, &IRQ_ASK, E_ROUTER, Wait::AtMost(QUAY_MS))?;
    debug!(
        "router: claimed {} {} {}",
        plic_deed.name.as_str(),
        dtb_deed.name.as_str(),
        irq_deed.name.as_str()
    );

    // 开图 + 读树：控制器、本域的 context（线那一半——"这台是哪条线"——已随设备账走，
    // 见 `core/sources.rs` 的照实记）。
    let plic_dev = Device::open(plic_deed.token).map_err(|_| Fail::at(E_ROUTER, "docks"))?;
    let dtb_dev = Device::open(dtb_deed.token).map_err(|_| Fail::at(E_ROUTER, "docks"))?;
    let dtb = dtb_dev.view();
    // SAFETY: 设备树是内核只读借映进本域的整棵（保留区，终身存活）；`Sources::of` 只读它。
    let bytes = unsafe { core::slice::from_raw_parts(dtb.base() as *const u8, dtb.size()) };
    let sources = Sources::of(bytes).ok_or(Fail::at(E_ROUTER, "tree"))?;
    let plic = Plic::new(plic_dev.view(), &sources);
    debug!("router: docks open");
    // 这台控制器那两个数——本域自己的事实，唯一一次陈述。
    debug!(
        "router: device_count={} ctx={}",
        sources.device_count(),
        sources.context()
    );
    let bell = Bell::new(NolePie::from_token(irq_deed.token));

    // 账：格数按控制器自报的线数要，装不下 ⇒ 拒起（"领到的线一定记得下"是构造性事实）。
    // **起域时一条都不接**：接线是登记的直接后果（见 `driver/router/mod.rs`）。
    let lines =
        Lines::new(sources.device_count()).ok_or(Fail::at(E_ROUTER, "line account full"))?;

    // **牌子最后落**（照实记，量出来的）：与 `rtc` / `uart` 两台同一条——牌子一落客人就找得到
    // 它，而本域此前还在认三样设备、开图、读树。**本台尤其要紧**：它的牌子是"线那本账"的入口，
    // 客人登记扑空一次就会放下它那条泊位（线那本账上因此会短暂地少一位客人）。
    ctx.plate(SERVICE, Mine::No, Wait::AtMost(QUAY_MS));

    // 等三个源：**铃**（外部中断）、**门上有人**（登记）、**客人的排空**（每登记一条线
    // 就把那位客户的泊位挂进来，见 `desk`）。一只组同时等这三样——三件都是事件，
    // 故等待**没有期限**（见 `resident` 里那一注）：会丢的那一次铃已在根上修掉。
    let pile = Pile::unseal(false).map_err(|_| Fail::at(E_ROUTER, "bell"))?;
    let entry_hole = HolePie::from_token(entry);
    if pile
        .attach(&NolePie::from_token(irq_deed.token), HoleDir::Pull)
        .is_err()
        || pile.attach(&entry_hole, HoleDir::Pull).is_err()
    {
        return Err(Fail::at(E_ROUTER, "bell"));
    }

    // 一问的形状是 `lcall::Occupy::LEN`；缓冲给**一页**（余量；孔不预设长度，装不下会答 `Denied` 且手原样）。
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Fail::at(E_ROUTER, "desk"));
    }
    buf.resize(PAGE_SIZE, 0);

    Ok(Up {
        plic,
        lines,
        bell,
        pile,
        buf,
        entry: entry_hole,
        replies: Replies::new(),
    })
}
