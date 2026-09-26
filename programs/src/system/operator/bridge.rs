//! operator::bridge — **装配侧**：把持树者接上一位客人（三步），并认下它那条提示之路
//!
//! 三侧分家之后本文件只放**装配侧**；两侧共用的图与说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::system::operator`]。

use env::Mark;
use env::Wait;
use env::wire::Field;
use env::{Name, PieToken, TaskId};
use plan::assembly::Eyes;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use protocol::communication::establish;
use protocol::system::operator::frame::CoordFrame;
pub use protocol::system::operator::{LINK, TIP_MARK};

// ── 装配侧（装配者调用）──────────────────────────────────────

/// 把协调那一帧推给持树者（**每一位递门牌的域各调一次**）。
fn coord_frame(into: PieToken, who: TaskId, eyes: Eyes) -> Result<(), ()> {
    let mut rec = [0u8; CoordFrame::LEN];
    CoordFrame { who, eyes }.store(&mut rec);
    mail::HolePie::from_token(into).push(&rec).map_err(|_| ())
}

/// **协调那一帧要带的两格**：名册那一位域的号 / 盟册那一位域的号（`None` = 还没到）。
///
/// 一格一个位、**可以分两帧到**（次序不定），故装配者收着、持树者补着，都不当"一次性解出来"。
/// 两侧共读这一个类型（`server.rs` 原先自己写了一份同形的私有 `Coord`）。
#[derive(Clone, Copy, Default)]
pub struct Coord {
    pub roster: Option<TaskId>,
    pub league: Option<TaskId>,
}

impl Coord {
    /// 递帧要的两对：**槽位由眼睛的名字定**，不由位置定。
    fn pairs(self) -> [(Option<TaskId>, Eyes); 2] {
        [(self.roster, Eyes::Roster), (self.league, Eyes::League)]
    }
}

/// **持树者在装配者这一侧的状态**：持树者的号 ＋ 它那条提示之路 ＋ 协调帧那两格。
///
/// 原先这三样是 `System` 上的三个裸字段（`tree` / `otip` / `coord`）。它们问的是**树的语义**
/// ——客人怎么接、提示怎么认、哪一双眼睛往哪记——故收进树这一间。
#[derive(Default)]
pub struct Tree {
    host: Option<TaskId>,
    tip: Option<PieToken>,
    coord: Coord,
}

impl Tree {
    /// 持树者那一枚的号（`None` = 还没起）。
    pub fn host(&self) -> Option<TaskId> {
        self.host
    }

    /// **把这位客人接上树**（三步见 [`attach`]）。持树者还没起就没得接。
    pub fn attach(
        &mut self,
        client: TaskId,
        millis: Wait,
    ) -> Result<(), &'static str> {
        let host = self.host.ok_or("no tree yet")?;
        attach(client, host, millis, &mut self.tip, self.coord)
    }

    /// **它就是持树者本身**：认下它那条提示之路，此后客人上树才有路可走。
    pub fn adopt(&mut self, host: TaskId, millis: Wait) -> Result<(), &'static str> {
        self.host = Some(host);
        self.tip = None;
        host_of(host, millis, &mut self.tip)?;
        Ok(())
    }

    /// **它是哪一双眼睛**：把那一格记进给持树者的协调帧（重复推是幂等的）。
    pub fn eye(&mut self, eyes: Eyes, who: TaskId) {
        match eyes {
            Eyes::Roster => self.coord.roster = Some(who),
            Eyes::League => self.coord.league = Some(who),
        }
    }
}

/// 把持树者接上一位客人（装配者调用）：**三步**（见文件头那张图）。
///
/// `host` = 持树者的号（`service::spawn` 交回来的那个，装配者本来就知道它）。
/// `tip` = 提示之路在**本线程表里**的那一枚（第一次用时认下来，此后逐条传下去）。
/// `coord` = 协调那一帧要带的两格（**哪一位域** + **它是哪一双眼睛**）：那位递门牌的域那一格
/// 才有值，其余留空——**定长两格**，因为这一族只有两双眼睛（[`Eyes`]）。
///
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 提示孔认不到……对调用方是
/// 同一件事——**这条服务没接上树**——但"死在哪一步"正是装配诊断要的那一格。
pub fn attach(
    client: TaskId,
    host: TaskId,
    millis: Wait,
    tip: &mut Option<PieToken>,
    coord: Coord,
) -> Result<(), &'static str> {
    let link = Name::new(LINK).map_err(|_| "operator:name")?;
    // 1+2. **一手就是"两头都装"**：本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：
    //      答话从客人自己那枚走）＋ 认领**这位客人**交出来的那一枚（记号 = 这条路的名字）。
    //      判据两格（`owner == client` ＋ 记号）与原 `seat` ＋ `claim` 逐字同源。
    let link =
        establish::endpoint(client, Mark::of(link.as_str()), millis).map_err(|_| "operator:seat")?;
    // **认不到对端那一枚 = 这条路没接上**（原 `claim` 那一格）。
    if link.tx().is_none() {
        return Err("operator:claim");
    }
    // 3. 提示孔（只认一次）→ 把客人那一枚转授给持树者 → 告两边。
    //    `host_of` 之后 `tip` 必有值（认不到它自己就返 `Err` 了）——所以这里取的是**那一枚孔**，
    //    要推的正是它（**不是** `host`：那是持树者的号，推不动）。
    let _ = host_of(host, millis, tip)?;
    let tip_at = (*tip).ok_or("operator:tip")?;
    // 3.5 **协调那一帧**：把递门牌那几位域的号推过去。**门牌不由这里转授**（那是各域自己
    // 在 `serve_tree` 之后直接交给持树者的）。
    // 次序仍是契约：客人号来之前，持树者先认出名册那一枚门牌（它按 `owner` + 记号找）；
    // 两帧按位递、次序不定，收到哪一枚就补上哪一枚（对齐见 `server.rs` 的 `settle`）。
    for (who, eyes) in coord.pairs() {
        if let Some(who) = who {
            coord_frame(tip_at, who, eyes).map_err(|_| "operator:coord")?;
        }
    }
    // 树路上本端手里那一枚 = **客人答话路的写端**（认下来时进 `link.tx()`）。
    let reply = link.tx().ok_or("operator:hand")?;
    hand(reply, host).map_err(|()| "operator:hand")?;
    // 客人那一侧的一格：**答话的是谁**（持树者的号，8 字节）。
    tell(host, reply).map_err(|_| "operator:who")?;
    // 提示在**转授之后**：持树者据此可以按"提示一到，答话路必已在本表里"办事。
    // **这一对孔本函数不必拿着、也放不下**：本端那一枚（`link.rx()`）是垫的（本端从不读它），
    // 可它得**一直活着**——客人那一侧要有人认它（`operator::client::open` 的 `claim` 扫的就是
    // 本域铸出去那一枚的副本），而认下之后持树者那一路也一直指着它写。它归**本域那张表**
    // （`Endpoint` 上只有 `claim`，没有"放下"这个动作）⇒ 本域退场时一并回收。
    tell(client, (*tip).ok_or("operator:tip")?).map_err(|_| "operator:tell")
}

/// 认下持树者交回来的那一枚提示孔（**只认一次**）：判据两格——`owner == 持树者`
/// （那一枚是它铸的）**且** 记号 = [`TIP_MARK`]。认下来之后本线程拿着的就是
/// "往提示之路推客人号 / 推协调帧"那一枚。
///
/// **只认、不铸**：本端这一侧在这条路上不需要自己那一枚。**照实记（这一格原先还多装了一条）**：
/// 从前这里先 `seat` 一次——本端另铸一枚、刻的是另一个记号（`TIP_NAME = "operator-tip"`）——
/// 而那一枚两头都不用。两个记号并成一个之后，这条路上只剩这一手。
///
/// **两种装配者都用它**：编排域用它把客人接上树（[`attach`] 的第一步），引导域用它
/// 把这条提示之路先认到手里、再转授给编排域（`root` 引导期的交接那一格）。
pub fn host_of(
    host: TaskId,
    millis: Wait,
    tip: &mut Option<PieToken>,
) -> Result<TaskId, &'static str> {
    if tip.is_some() {
        return Ok(host);
    }
    // 交给调用方拿着：同一条路上以后每次都往里推客人号 / 协调帧（**同一枚线程**用它）。
    *tip = establish::claim(host, TIP_MARK, millis);
    if tip.is_none() {
        return Err("operator:tip");
    }
    Ok(host)
}

/// 把一个号推过去（8 字节，小端）。
///
/// **两处共用这一句**：提示孔那一格（告持树者"客人是谁"）与树路那一格（告客人"答话的是谁"）。
/// 两处都是"装配者知道、对方叫不出"的那个号——故 `tell` 只认"推给哪一枚孔"，不认语义。
///
/// **帧形只有一处**：宽度与字节序归 [`Field`](env::wire::Field) 给 [`TaskId`] 那一对
/// `store` / `fetch`。
pub(crate) fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let mut rec = [0u8; TaskId::WIDTH];
    who.store(&mut rec);
    let into = mail::HolePie::from_token(into);
    into.push(&rec).map_err(|_| ())
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
