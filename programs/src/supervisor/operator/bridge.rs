//! operator::bridge — **装配侧**：把持树者接上一位客人（三步），并认下它那条提示之路
//!
//! 三侧分家之后本文件只放**装配侧**：把持树者接上一位客人（三步），并认下它那条提示之路；两侧共用的图与说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::operator::call`]。

use env::Mark;
use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

pub use protocol::operator::{LINK, TIP_MARK, TIP_NAME};
use protocol::session::Quay;

// ── 装配侧（装配者调用）──────────────────────────────────────

/// **协调那一帧**（16 字节）——装配者告诉持树者"身份服务是谁、它自己把门牌交过来了"。
///
/// 布局：`[0..8]` = 身份服务的号（小端）｜`[8..16]` = 保留零。
///
/// # 为什么门牌不由装配者转授（照实记：这一格返工过）
///
/// 第一版让装配者把那一枚门牌**再转授**给树。真机栽了：`principal` 那一格报
/// `operator:coord-ship`，内核答 `-1 Denied`——而装配者手里那一枚权限位是对的（`0x3`：
/// `FETCH|STORE`）、也在表里。那一格的三道闸（覆盖子集 / 持 `VEST` / 形态一致）都不是原因，
/// 于是这一笔"第二手转授"在装配窗口里带进了说不清的锚与来历问题。
///
/// **改成由身份服务自己交**（它本来就是树的客人：`serve_tree` 那一趟已经握着树路）：
/// 它 `serve_tree` 之后把门牌那一枚直接 `ship` 给持树者，再把**自己的号**经这一帧递过去。
/// 于是：
///
/// - 持树者拿到的门牌**一手来源**，没有第二手转授；
/// - 装配者只剩"递一格号"这一件事，`attach` 里不多一次 `Ship`；
/// - 身份服务本来就与树有一条会话（挂门牌那一趟），这一笔是它的近邻。
///
/// 长度即语义：**8 字节 = 一位客人**（`settle` 认客人的那一格），**16 字节 = 这一帧**。
const COORD: usize = 16;

/// 把协调那一帧推给持树者（**只在身份服务那一格调一次**）。
fn coord_frame(into: PieToken, who: TaskId) -> Result<(), ()> {
    let mut frame = [0u8; COORD];
    frame[..8].copy_from_slice(&(who.get() as u64).to_le_bytes());
    mail::HolePie::from_token(into).push(&frame).map_err(|_| ())
}

/// 把持树者接上一位客人（装配者调用）：**三步**（见文件头那张图）。
///
/// `host` = 持树者的号（`service::spawn` 交回来的那个，装配者本来就知道它）。
/// `tip` = 提示之路在**本线程表里**的那一枚（第一次用时认下来，此后逐条传下去）。
/// `coord` = 协调那一帧要带的号（身份服务那一格才有）：**身份服务的号**。其余每一格传 `None`。
///
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 提示孔认不到……对调用方是
/// 同一件事——**这条服务没接上树**——但"死在哪一步"正是装配诊断要的那一格。
pub fn attach(
    quay: &mut Quay,
    client: TaskId,
    host: TaskId,
    millis: usize,
    tip: &mut Option<PieToken>,
    coord: Option<TaskId>,
) -> Result<(), &'static str> {
    let link = Name::new(LINK).map_err(|_| "operator:name")?;
    // 1. 本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：答话从客人自己那枚走）。
    quay.seat(link).map_err(|_| "operator:seat")?;
    // 2. 认领**这位客人**交出来的那一枚（记号 = 这条路的名字，客侧 `seat` 刻的就是它）。
    quay.claim(client, Mark::of(link.as_str()), millis)
        .map_err(|_| "operator:claim")?;
    // 3. 提示孔（只认一次）→ 把客人那一枚转授给持树者 → 告两边。
    //    `host_of` 之后 `tip` 必有值（认不到它自己就返 `Err` 了）——所以这里取的是**那一枚孔**，
    //    要推的正是它（**不是** `host`：那是持树者的号，推不动）。
    let _ = host_of(host, millis, tip)?;
    let tip_at = (*tip).ok_or("operator:tip")?;
    // 3.5 **协调那一帧**：把身份服务的号推过去。**门牌不由这里转授**（那是身份服务自己
    // 在 `serve_tree` 之后直接交给持树者的，理由见 [`COORD`] 那段照实记）。
    // 次序仍是契约：客人号来之前，持树者先认出身份服务那一枚门牌（它按 `owner` + 记号找）。
    if let Some(who) = coord {
        coord_frame(tip_at, who).map_err(|_| "operator:coord")?;
    }
    let reply = reply_path(quay).ok_or("operator:hand")?;
    hand(reply, host).map_err(|()| "operator:hand")?;
    // 客人那一侧的一格：**答话的是谁**（持树者的号，8 字节）。
    tell(host, reply).map_err(|_| "operator:who")?;
    // 提示在**转授之后**：持树者据此可以按"提示一到，答话路必已在本表里"办事。
    tell(client, (*tip).ok_or("operator:tip")?).map_err(|_| "operator:tell")
}

/// 认下持树者交回来的那一枚提示孔（**只认一次**）：本域另开一座码头等它。
///
/// 判据两格：`owner == 持树者`（那一枚是它铸的）**且** 记号 = `tip`。认下来之后本线程
/// 拿着的就是"往提示之路推客人号"那一枚。
///
/// **两种装配者都用它**：编排域用它把客人接上树（[`attach`] 的第一步），引导域用它
/// 把这条提示之路先认到手里、再转授给编排域（`root` 引导期的交接那一格）。
pub fn host_of(
    host: TaskId,
    millis: usize,
    tip: &mut Option<PieToken>,
) -> Result<TaskId, &'static str> {
    if tip.is_some() {
        return Ok(host);
    }
    let slot = Name::new(TIP_NAME).map_err(|_| "operator:name")?;
    let mut quay = Quay::open(host);
    quay.seat(slot).map_err(|_| "operator:seat")?;
    quay.claim(host, TIP_MARK, millis)
        .map_err(|_| "operator:tip")?;
    let pier = quay.find(slot).ok_or("operator:tip")?;
    // 交给调用方拿着：同一条路上以后每次都往里推客人号（**同一枚线程**用它）。
    *tip = pier.at_peer();
    if tip.is_none() {
        return Err("operator:tip");
    }
    Ok(host)
}

/// 把一个号推过去（8 字节，小端）。
///
/// **两处共用这一句**：提示孔那一格（告持树者"客人是谁"）与树路那一格（告客人"答话的是谁"）。
/// 两处都是"装配者知道、对方叫不出"的那个号——故 `tell` 只认"推给哪一枚孔"，不认语义。
pub(crate) fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let into = mail::HolePie::from_token(into);
    into.push(&(who.get() as u64).to_le_bytes()).map_err(|_| ())
}

/// 树路上本端手里那一枚（客人答话路的**写端**）：答话往它推，"答话的是谁"也从它递。
pub(crate) fn reply_path(quay: &Quay) -> Option<PieToken> {
    let link = Name::new(LINK).ok()?;
    quay.find(link)?.at_peer()
}

/// 把**客人交出来的那一枚**转授给持树者。
///
/// 转授的是"客人开的那扇门"（`owner` 是客人），持树者那侧认领时认的正是它。
///
/// 子集只给 `R|W`，**不加 `VEST`**：持树者用这一枚写答话，不需要再授出——一分不多。
pub(crate) fn hand(reply: PieToken, host: TaskId) -> Result<(), ()> {
    let hole = mail::HolePie::from_token(reply);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}
