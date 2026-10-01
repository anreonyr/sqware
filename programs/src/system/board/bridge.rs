//! board::bridge — **装配侧**：把板接上一位客人（三步，次序即契约）与收尾点名
//!
//! 三侧分家之后本文件只放**装配侧**：把板接上一位客人（三步，次序即契约）与收尾点名；两侧共用的图与次序说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::system::board`]。

use alloc::string::String;
use alloc::string::ToString;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::Mark;
use env::Wait;
use env::wire::Field;

use env::{HoleDir, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::core::unit::{self, Join};
use runtime::env::mail;
use runtime::env::unit as utask;

use protocol::communication::establish;
use protocol::system::board as bcall;
pub use protocol::system::board::{LINK, TIP_MARK};

use crate::program::Program;
use crate::system::Assembly;
use crate::system::control::{READY_MS, Service};

use super::server::host_loop;

/// 板线程的号（0 = 还没起）。`TaskId` 是**全局身份**，可跨线程，故这一枚放得进 static。
static HOST: AtomicUsize = AtomicUsize::new(0);
// 提示之路在**装配者表里**的那一枚句柄**不放 static**：`PieToken` 是表的身份、标着 `!Sync`
// （`env::wire::handle`），static 装不下它——它由装配者这一枚线程自己拿着，逐次传下去。

/// **板在装配者这一侧的状态**：那条提示之路（一枚装配线程用一条）。
///
/// 原先它是 `System` 上的一个裸字段（`btip`）。它问的是**板的语义**——提示怎么认、往哪递
/// ——故收进板这一间；装配者那一侧只留这一个手柄。
#[derive(Default)]
pub struct Bridge {
    tip: Option<PieToken>,
}

impl Bridge {
    /// **把这位客人接上板**（三步见 [`attach`]，次序即契约）。返 `Err(哪一步)`。
    pub fn attach(
        &mut self,
        me: TaskId,
        client: TaskId,
        name: String,
        millis: Wait,
        lane: Option<PieToken>,
    ) -> Result<(), &'static str> {
        attach(me, client, name, millis, &mut self.tip, lane)
    }
}

/// **板这一轴在装配那一趟里的那一手**：读这一台声明上 `presence` 那一格（"要不要存在信号"）。
///
/// **在通道之后**：那条路由客人在起来之后自己装（它是问的那一侧），而它要先收到配给才轮得到
/// 那一问。**这一手的全部内容**：把这位客人交出来的那一枚转授给板线程——板据此看得见它的死
/// （三步见 [`attach`]，次序即契约）。
pub fn attach_client(
    assembly: &mut Assembly,
    program: &Program,
    service: &mut Service,
) -> Result<(), &'static str> {
    if !program.relation.presence {
        return Ok(());
    }
    let name = program.name().to_string();
    // **道那一格退了场**（照实记：死改由监督那一趟的表侧扫认，见 `control::supervise` 的
    // `Watch::new`）——故这一手不再取道，`lane` 那一格恒为 `None`。
    let lane = None;
    assembly.board.attach(
        utask::self_id(),
        service.0,
        name,
        Wait::AtMost(READY_MS),
        lane,
    )
}

/// 把板接上一位客人（装配者调用）：**三步**（见文件头"一问一答的次序"）。
///
/// `client` = 客人（= 装配者刚起的那一枚线程；iii 之后它可能住**本域**）：**客人交出来的那一枚就落在
/// 本域表里**（客人把孔交给生我者），而它的 `owner` 正是这位客人——故第 2 步认的是**它**。
///
/// `name` = **装配表上那一位的名字**：它随提示那一格交给板（[`bcall::Tip::LEN`]）——板据此在
/// `admit` 那一刻就把"谁 → 死亡道"记下（**这一位不必自己报名**）。
///
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 死亡道转授或递格失败……
/// 对调用方是同一件事——**这条服务没接上板**——但"死在哪一步"正是装配诊断要的那一格
/// （与 `service::step` 同款；逐字的因就是函数体里那几处 `map_err` 的字符串）。
pub fn attach(
    me: TaskId,
    client: TaskId,
    name: String,
    millis: Wait,
    tip: &mut Option<PieToken>,
    lane: Option<PieToken>,
) -> Result<(), &'static str> {
    // 1+2. **一手就是"两头都装"**：本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：
    //      答话从客人自己那枚走）＋ 认领**这位客人**交出来的那一枚（记号 = 板路的名字，客侧
    //      铸的也是它）。判据两格（`owner == client` ＋ 记号）与原 `seat` ＋ `claim` 逐字同源
    //      ——本域给每个孩子各开一条路，故认的是"它给我的"，不然会把别的客人的孔配到它头上。
    //      **次序**：板那条比 `records` 后到，而 `records` 的写端已经用掉了。
    let link =
        establish::endpoint(client, Mark::of(LINK), millis).map_err(|_| "board:seat")?;
    // **认不到对端那一枚 = 这条板路没接上**（原 `claim` 那一格）：本端这一侧虽然只读答话，
    // 但"两侧各装一条、凑齐才算通"那条不变量仍在——没齐就是没接上，不必等到第一次收帧。
    if link.tx().is_none() {
        return Err("board:claim");
    }
    // 3. 板线程（只起一枚）→ 把客人那一枚转授过去 → 板路上递一格"答话的是谁" → 提示来客人了。
    let host = host(me, millis, tip)?;
    // 死亡道：**这一位的那一条**转授给板线程（板按记号 `gone-<名字>` 在自己表里认领它）。
    // 位置在客人那一枚转授之后：板线程这时已经起来（`host` 起过就复用）。
    if let Some(lane) = lane {
        let hole = mail::HolePie::from_token(lane);
        port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
            .map_err(|_| "board:lane")?;
    }
    let Some(tip) = *tip else {
        return Err("board:tip");
    };
    // 板路上本端手里那一枚 = **客人答话路的写端**（认下来时进 `link.tx()`）。
    let reply = link.tx().ok_or("board:hand")?;
    // **转授那一手把"我给你的那一枚在你表里是几号"交出来**（`to.seed()`）：它随提示一起过去，
    // 板那边一次 `Reserve` 就认得答话路——不必扫自己的表。
    let seed = hand(reply, host).map_err(|()| "board:hand")?;
    // 客人那一侧的一格：**答话的是谁**（板线程的号，8 字节）——与提示孔那一格对偶。
    tell(host, reply).map_err(|_| "board:who")?;
    // 提示在**转授之后**：板据此可以按"提示一到，答话路必已在本表里"办事。
    // **提示那一格多带两格**（名字 ＋ 答话路那一格）：名字让板在 `admit` 那一刻把这一位的死亡道
    // 记下（不必等它自己报名），末格让板认答话路不必扫表。
    // **这一对孔本函数不必拿着、也放不下**：本端那一枚（`link.rx()`）是垫的（本端从不读它），
    // 可它得**一直活着**——客人那一侧要有人认它（`protocol::communication::session::Session::open`
    // 的 `claim` 扫的就是本域
    // 铸出去那一枚的副本），而认下之后板那一路也一直指着它写。它归**本域那张表**（`Endpoint`
    // 上只有 `claim`，没有"放下"这个动作）⇒ 本域退场时一并回收。
    tell_guest(client, name, seed, tip).map_err(|_| "board:tell")
}

/// 起板线程（**就一枚**），返它的号；起过了就把那个号给回来。
///
/// "为什么就一枚"见文件头。这里补**提示之路**的来历：装配者得告诉板线程"来客人了、它是
/// 谁"，而孔是**铸的人那张表**里的东西（装配者铸的孔，板线程表里没有它）——故这条路由板
/// 线程自己铸：它起来第一件事就是把这枚孔的副本交给**装配者**。`me` 因此得从外面给：
/// 同域里产出来的线程，`sire` 是**域的**生我者（建这个域的那一枚），不是产它的那一枚
/// （`UnitCall::Sire` 的正文）——同一个域里的两枚线程，"谁生我"答不出"谁产的"。
fn host(me: TaskId, millis: Wait, tip: &mut Option<PieToken>) -> Result<TaskId, &'static str> {
    let had = HOST.load(Ordering::Acquire);
    if had != 0 {
        return Ok(TaskId::new(had));
    }
    let node: Join<()> = unit::closure(move || host_loop(me));
    let id = node.id();
    // **弃权**（`Join` 的 Drop）：不等它的结果，但按协议把完成盒子交回去（板线程长期不
    // 返回；`mem::forget` 会把它漏掉）。
    drop(node);
    HOST.store(id.get(), Ordering::Release);

    // 认领板线程交回来的那一枚提示孔（判据 = `owner == 板线程` **且** 记号 = `TIP_MARK`
    // ——板线程那一枚是它自己铸的，记号就是它的用途名）。**只认、不铸**：本端这一侧在这条路上
    // 不需要自己那一枚（`claim` 那一手）。
    //
    // **照实记（这一格原先还多装了一条）**：从前这里先 `seat` 一次——本端另铸一枚、刻的是
    // 另一个记号（`TIP_NAME = "board-tip"`）——而那一枚两头都不用（本端不读它，板线程也不认
    // 它）。两个记号并成一个之后，这条路上只剩这一手。
    *tip = establish::claim(id, TIP_MARK, millis);
    if tip.is_none() {
        return Err("board:tip");
    }
    // 交给调用方拿着：同一条路上以后每次都往里推一位新客人（**同一枚线程**用它）。
    Ok(id)
}

/// 把一个号推过去（8 字节，小端）。
///
/// **一处用**：板路那一格（告客人"答话的是谁"）。提示那一格走 [`tell_guest`]——它多带一格
/// 名字。两处都是"装配者知道、对方叫不出"的那个号，故 `tell` 只认"推给哪一枚孔"，不认语义。
///
/// **帧形只有一处**：宽度与字节序归 [`Field`](env::wire::Field) 给 [`TaskId`] 那一对
/// `store` / `fetch`。
pub(crate) fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let mut rec = [0u8; TaskId::WIDTH];
    who.store(&mut rec);
    let into = mail::HolePie::from_token(into);
    // **两半都写出来**（旧 `push` 是合一的）：等轮到自己 ＋ 等这只手被取走——`rec` 是栈。
    into.push(&rec, Wait::Forever).map_err(|_| ())?;
    into.wait(HoleDir::Push, Wait::Forever).map_err(|_| ())?;
    Ok(())
}

/// 把**一位新客人**推给板：**号（8 字节）＋ 定长名字 ＋ 答话路那一格**（[`bcall::Tip::LEN`]），小端。
///
/// **名字为什么从这一格走**：道是装配者铸的，名字也在装配表里；板据此在 `admit` 那一刻就把
/// "谁 → 道"记下，客人自己不必报名。
///
/// **末格同一条理由**：那个号也只有装配者手里有（它刚转授过去），板叫不出——故随这一格一起过去。
/// **帧形只有一处**：三项怎么排、各占多宽，全在 `bcall::Tip` 那一对 `store` / `fetch` 里。
pub(crate) fn tell_guest(
    who: TaskId,
    name: String,
    seed: PieToken,
    into: PieToken,
) -> Result<(), ()> {
    let mut rec = [0u8; bcall::Tip::LEN];
    let n = bcall::Tip {
        who,
        name,
        reply: seed,
    }
    .store_at(&mut rec, 0)
    .ok_or(())?;
    let into = mail::HolePie::from_token(into);
    into.push(&rec[..n], Wait::Forever).map_err(|_| ())?;
    into.wait(HoleDir::Push, Wait::Forever).map_err(|_| ())?;
    Ok(())
}

/// 把**客人交出来的那一枚**转授给板线程，返**它在板表里的号**（`port::ship` 的 `to.seed()`）。
///
/// 转授的是"客人开的那扇门"（`owner` 是客人），板那侧认领时认的正是它。
///
/// **返那一格是这一刀的要害**：板拿到它就能一次 `Reserve` 把答话路认下来，不必扫自己的表。
///
/// 子集只给 `R|W`，**不加 `VEST`**：板线程用这一枚写答话，不需要再授出——一分不多。
/// 本域自己那一份转授之后**不收**：客人给过来的这一枚带 `VEST`（`establish::endpoint` 铸的
/// 就是 `R|W|VEST`）⇒ 这次授出是**复制**，源枚在我表里照旧可用；而它**归本域持有**
/// （`Endpoint` 上放不下它，见 `establish` 文件头那条照实记）⇒ 本域退场时随表一起消失。
pub(crate) fn hand(reply: PieToken, host: TaskId) -> Result<PieToken, ()> {
    let hole = mail::HolePie::from_token(reply);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map(|to| to.seed())
        .map_err(|_| ())
}
