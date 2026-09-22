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

/// 把持树者接上一位客人（装配者调用）：**三步**（见文件头那张图）。
///
/// `host` = 持树者的号（`service::spawn` 交回来的那个，装配者本来就知道它）。
/// `tip` = 提示之路在**本线程表里**的那一枚（第一次用时认下来，此后逐条传下去）。
///
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 提示孔认不到……对调用方是
/// 同一件事——**这条服务没接上树**——但"死在哪一步"正是装配诊断要的那一格。
pub fn attach(
    quay: &mut Quay,
    client: TaskId,
    host: TaskId,
    millis: usize,
    tip: &mut Option<PieToken>,
) -> Result<(), &'static str> {
    let link = Name::new(LINK).map_err(|_| "operator:name")?;
    // 1. 本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：答话从客人自己那枚走）。
    quay.seat(link).map_err(|_| "operator:seat")?;
    // 2. 认领**这位客人**交出来的那一枚（记号 = 这条路的名字，客侧 `seat` 刻的就是它）。
    quay.claim(client, Mark::of(link.as_str()), millis)
        .map_err(|_| "operator:claim")?;
    // 3. 提示孔（只认一次）→ 把客人那一枚转授给持树者 → 告两边。
    let _ = host_of(host, millis, tip)?;
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
