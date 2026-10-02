//! 领配给 → 开两图 → 读树 → 建账 → 铸入口 → 上板 ＋ 上树 → 挂组。
//! 起手的产物是**同一条命**（控制器的事实 / 账 / 门铃 / 组 / 门外那一页缓冲），故合成一个
//! 类型 Up：常驻那一圈每醒一次用到的就是它。
//! **三段里的前两段已住 programs::driver**（领配给、开图、上板、上树那几步三台同构）。
//! **上板 / 上树仍然尽力**：这一台起来就得收（铃一响就要 claim），故两件任一件没成都只报一行

use super::event::desk::Replies;
use crate::core::lines::Lines;
use crate::core::sources::Sources;
use crate::dev::plic::Plic;
use alloc::vec::Vec;
use env::{Access, HoleDir, PieKind, Policy, Wait};
use programs::driver::shared::context::{Context, Step};
use programs::driver::shared::device::{Ask, Device, Hub};
use programs::driver::shared::fail::Fail;
use programs::unit::router::{E_ROUTER, PLIC_CLASS};
use protocol::debug;
use protocol::service::hub as hcall;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Mine;
use runtime::PAGE_SIZE;
use runtime::core::res::bell::Bell;
use runtime::core::res::pile::Pile;
use runtime::env::mail::{self, HolePie, NolePie};
use runtime::env::unit as utask;

const SERVICE: &str = "router";

/// 它们不是树里的设备）
const PLIC_ASK: Ask = Ask {
    class: PLIC_CLASS,
    name: None,
    kind: PieKind::Pole,
    access: Access::FETCH_STORE,
    policy: Policy::ONLY,
};
const DTB_ASK: Ask = Ask {
    class: hcall::BOOT,
    name: Some(hcall::DTB),
    kind: PieKind::Pole,
    access: Access::FETCH,
    policy: Policy::NONE,
};
const IRQ_ASK: Ask = Ask {
    class: hcall::BOOT,
    name: Some(hcall::IRQ),
    kind: PieKind::Nole,
    access: Access::FETCH,
    policy: Policy::NONE,
};

/// 装泊位 / 等配给 / 办一趟登记 / 上树的期限（毫秒）
const QUAY_MS: usize = 1000;

pub struct Up {
    /// 控制器寄存器面（设备侧）
    pub plic: Plic,
    /// 账：线号 = 下标（容量按 `device_count` 校验 ⇒ 越界不可表达）
    pub lines: Lines,
    /// 门铃（内核给的那一枚；它只 `hush`，不铸）
    pub bell: Bell,
    /// 等三源的组
    pub pile: Pile,
    /// 门外那一页缓冲（取消息用；**按本族最长那一枚备足**，见 `resident`）
    pub buf: Vec<u8>,
    pub entry: HolePie,
    pub replies: Replies,
}

/// 起手
pub fn up() -> Result<Up, Fail> {
    // **起手第一件：入系统**（服务入口 → 上板 ＋ 开会话 → 上树落门牌）。
    let entry =
        mail::unseal_hole(protocol::driver::ENTRY_MARK).map_err(|_| Fail::at(E_ROUTER, "desk"))?;
    let sire = utask::sire();
    let ctx = Context::open(sire, Wait::AtMost(QUAY_MS)).map_err(|s| {
        Fail::at(
            E_ROUTER,
            match s {
                Step::Tree => "tree",
            },
        )
    })?;
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

    let plic_dev = Device::open(plic_deed.token).map_err(|_| Fail::at(E_ROUTER, "docks"))?;
    let dtb_dev = Device::open(dtb_deed.token).map_err(|_| Fail::at(E_ROUTER, "docks"))?;
    let dtb = dtb_dev.view();
    let bytes = unsafe { core::slice::from_raw_parts(dtb.base() as *const u8, dtb.size()) };
    let sources = Sources::of(bytes).ok_or(Fail::at(E_ROUTER, "tree"))?;
    let plic = Plic::new(plic_dev.view(), &sources);
    debug!("router: docks open");
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

    ctx.plate(entry, SERVICE, Mine::No, Wait::AtMost(QUAY_MS));

    // **报"答得动了"**（Setup::Ready）：牌子落了才算——装配者等它才往下起别人，于是"排在第几号"
    let _ = protocol::communication::session::establish::endpoint(
        runtime::env::unit::sire(),
        env::Mark::of(programs::unit::READY),
        env::Wait::POLL,
    );

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

    // 一问的形状是 lcall::Occupy::LEN；缓冲给**一页**（余量；孔不预设长度，装不下会答 `Denied` 且手原样）。
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
