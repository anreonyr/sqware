//! board::bridge — **装配侧**：把板接上一位客人（三步，次序即契约）与收尾点名
//!
//! 三侧分家之后本文件只放**装配侧**：把板接上一位客人（三步，次序即契约）与收尾点名；两侧共用的图与次序说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::system::board::call`]。

use env::wire::Field;
use env::Wait;
use core::sync::atomic::{AtomicUsize, Ordering};
use env::Mark;

use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::core::unit::{self, Join};
use runtime::env::mail;

use protocol::session::Quay;
use protocol::system::board::call as bcall;
pub use protocol::system::board::{LINK, TIP_MARK, TIP_NAME};

use super::server::host_loop;

/// 板线程的号（0 = 还没起）。`TaskId` 是**全局身份**，可跨线程，故这一枚放得进 static。
static HOST: AtomicUsize = AtomicUsize::new(0);
// 提示之路在**装配者表里**的那一枚句柄**不放 static**：`PieToken` 是表的身份、标着 `!Sync`
// （`env::wire::handle`），static 装不下它——它由装配者这一枚线程自己拿着，逐次传下去。

/// 把板接上一位客人（装配者调用）：**三步**（见文件头"一问一答的次序"）。
///
/// `client` = 客人（= 装配者刚起的那一枚线程；iii 之后它可能住**本域**）：**客人交出来的那一枚就落在
/// 本域表里**（客人把孔交给生我者），而它的 `owner` 正是这位客人——故第 2 步认的是**它**。
///
/// `name` = **装配单上那一位的名字**：它随提示那一格交给板（[`bcall::Tip::LEN`]）——板据此在
/// `admit` 那一刻就把"谁 → 死亡道"记下（**这一位不必自己报名**，见那一格的照实记）。
///
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 死亡道转授或递格失败……
/// 对调用方是同一件事——**这条服务没接上板**——但"死在哪一步"正是装配诊断要的那一格
/// （与 `service::step` 同款；逐字的因就是函数体里那几处 `map_err` 的字符串）。
pub fn attach(
    quay: &mut Quay,
    me: TaskId,
    client: TaskId,
    name: Name,
    millis: Wait,
    tip: &mut Option<PieToken>,
    lane: Option<PieToken>,
) -> Result<(), &'static str> {
    let link = Name::new(LINK).map_err(|_| "board:name")?;
    // 1. 本端那一枚交出去（落在本域表里——客人拿不到它，也不需要：答话从客人自己那枚走）。
    quay.seat(link).map_err(|_| "board:seat")?;
    // 2. 认领**这位客人**交出来的那一枚（记号 = 板路自己的名字，客侧 `seat` 刻的就是它）。
    //    本域给每个孩子各开一座码头，故认的是"它给我的"——不然会把别的客人的孔配到它头上
    //    （`Quay::claim` 的正文）。
    //    **次序**：板那条比 `records` 后到，而 `records` 的写端已经用掉了 ⇒ 这一枚认在
    //    板泊位上（一枚孔只配一条泊位）。
    quay.claim(client, Mark::of(link.as_str()), millis)
        .map_err(|_| "board:claim")?;
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
    let reply = reply_path(quay).ok_or("board:hand")?;
    // **转授那一手把"我给你的那一枚在你表里是几号"交出来**（`to.seed()`）：它随提示一起过去，
    // 板那边一次 `Reserve` 就认得答话路——不必扫自己的表（见 [`bcall::Tip::LEN`] 的照实记）。
    let seed = hand(reply, host).map_err(|()| "board:hand")?;
    // 客人那一侧的一格：**答话的是谁**（板线程的号，8 字节）——与提示孔那一格对偶。
    tell(host, reply).map_err(|_| "board:who")?;
    // 提示在**转授之后**：板据此可以按"提示一到，答话路必已在本表里"办事。
    // **提示那一格多带两格**（名字 ＋ 答话路那一格）：名字让板在 `admit` 那一刻把这一位的死亡道
    // 记下（不必等它自己报名），末格让板认答话路不必扫表——两笔都见 [`bcall::Tip::LEN`] 的照实记。
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

    // 认领板线程交回来的那一枚提示孔：本域另开一座码头等它（判据 = `owner == 板线程`
    // **且** 记号 = `tip`——板线程那一枚是它自己铸的，记号就是它的用途名）。
    // 这条路上只走"一位新客人"（号 ＋ 名字，见 [`tell_guest`]），故本端那一枚交出去也无妨
    // （板线程不用它，也不碍事）。
    let slot = Name::new(TIP_NAME).map_err(|_| "board:name")?;
    let mut quay = Quay::open(id, protocol::session::call::hands());
    quay.seat(slot).map_err(|_| "board:seat")?;
    quay.claim(id, TIP_MARK, millis).map_err(|_| "board:tip")?;
    let pier = quay.find(slot).ok_or("board:tip")?;
    // 交给调用方拿着：同一条路上以后每次都往里推一位新客人（**同一枚线程**用它）。
    *tip = pier.at_peer();
    Ok(id)
}

/// 把一个号推过去（8 字节，小端）。
///
/// **一处用**：板路那一格（告客人"答话的是谁"）。提示那一格走 [`tell_guest`]——它多带一格
/// 名字。两处都是"装配者知道、对方叫不出"的那个号，故 `tell` 只认"推给哪一枚孔"，不认语义。
///
/// **帧形只有一处**：宽度与字节序归 [`Field`](env::wire::Field) 给 [`TaskId`] 那一对
/// `store` / `fetch`（从前这里是手写的一遍 `(who.get() as u64).to_le_bytes()`，那一对里记着
/// 这一格原先散在五处）。
pub(crate) fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let mut rec = [0u8; TaskId::WIDTH];
    who.store(&mut rec);
    let into = mail::HolePie::from_token(into);
    into.push(&rec).map_err(|_| ())
}

/// 把**一位新客人**推给板：**号（8 字节）＋ 定长名字 ＋ 答话路那一格**（[`bcall::Tip::LEN`]），小端。
///
/// **名字为什么从这一格走**：见 [`bcall::Tip::LEN`] 的照实记——道是装配者铸的，名字也在装配
/// 单里；板据此在 `admit` 那一刻就把"谁 → 道"记下，客人自己不必报名。
///
/// **末格同一条理由**：那个号也只有装配者手里有（它刚转授过去），板叫不出——故随这一格一起过去。
/// **帧形只有一处**：三项怎么排、各占多宽，全在 `bcall::Tip` 那一对 `store` / `fetch` 里；
/// 这一侧只管把值递过去（从前这里是三行手切的偏移，与读者那一侧各写一遍）。
pub(crate) fn tell_guest(who: TaskId, name: Name, seed: PieToken, into: PieToken) -> Result<(), ()> {
    let mut rec = [0u8; bcall::Tip::LEN];
    bcall::Tip {
        who,
        name,
        reply: seed,
    }
    .store(&mut rec);
    let into = mail::HolePie::from_token(into);
    into.push(&rec).map_err(|_| ())
}

/// 板路上本端手里那一枚（客人答话路的**写端**）：答话往它推，"答话的是谁"也从它递。
pub(crate) fn reply_path(quay: &Quay) -> Option<PieToken> {
    let link = Name::new(LINK).ok()?;
    quay.find(link)?.at_peer()
}

/// 把**客人交出来的那一枚**转授给板线程，返**它在板表里的号**（`port::ship` 的 `to.seed()`）。
///
/// 转授的是"客人开的那扇门"（`owner` 是客人），板那侧认领时认的正是它。
///
/// **返那一格是这一刀的要害**：板拿到它就能一次 `Reserve` 把答话路认下来，不必扫自己的表
/// （见 [`bcall::Tip::LEN`] 的照实记）。原先这个号被 `.map(|_| ())` 扔了。
///
/// 子集只给 `R|W`，**不加 `VEST`**：板线程用这一枚写答话，不需要再授出——一分不多。
/// 本域自己那一份转授之后**不收**：客人给过来的这一枚不带 `ONLY`（`seat` 给的是
/// `R|W|VEST`）⇒ 这次授出是**复制**，源枚在我表里照旧可用；收它要多一条 `release`，
/// 而这一步之后没有任何东西再碰它——本域常驻，随域退场一起回收。
pub(crate) fn hand(reply: PieToken, host: TaskId) -> Result<PieToken, ()> {
    let hole = mail::HolePie::from_token(reply);
    port::ship(&hole, host, Access::FETCH | Access::STORE, Policy::NONE)
        .map(|to| to.seed())
        .map_err(|_| ())
}

// 照实记：这里原来有一个 `shut()`（"点名收掉本域那枚常驻板线程"）。**已删**——它的文档
// 自己就写着内核那一手的粒度是**域**，而板线程**就住在编排域里**，故那一叫收掉的正是本域
// 自己：编排域当场被扑杀，收尾那一句判词 `system: done` **永远够不到**（实测：1005 份 soak
// 日志里 0 次）。板线程本来就不必点名收——本域一退场，"域亡＝成员清零"把它一起带走，
// 留着这个函数只是一把上了膛的枪。`HOST` 那一格照旧（`attach` 用它防重复起）。
