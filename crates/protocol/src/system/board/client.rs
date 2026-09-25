//! board::client — **客侧三手**：装上板路、铸问话孔、一问一答（说「我走了」也在这一侧）
//!
//! 三侧分家之后本文件只放**客侧三手**：装上板路、铸问话孔、一问一答（说「我走了」也在这一侧）；两侧共用的图与次序说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`crate::system::board::call`]。

use contract::message::Message;
use env::wire::Field;
use env::Wait;
use env::Mark;
use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, AnyPie};

use crate::session::slip::Slip;
use crate::session::Quay;
use crate::system::board::Fail;
use crate::system::board::call as bcall;
pub use crate::system::board::{ASK_MARK, ENTRY_MARK, LINK};

/// 客侧第一步：装上板那条路（**记号就是这条路的名字**），认下对端那一枚，并收下
/// "**答话的是谁**"（[`hear`] 那一格）。
///
/// 返本端这座码头（**答话**从它走，问话走 [`ask_hole`]）与**板线程的号**。
///
/// `holder` = 客人认的对端 = **它的生我者**（孔交给它，它再转授给板线程）——注意它不是板：
/// 客人交出来的孔都落在生我者表里，故"板是谁"得由装配者告诉（见文件头），此后客人交孔、
/// 交入口才叫得出板。认领那一枚按 `(对端, 记号)` 两格认：对端是 `holder`（它 `seat` 出来
/// 的那一枚），记号就是板路的名字。
pub fn open(holder: TaskId, millis: Wait) -> Result<(Quay, TaskId), Fail> {
    let link = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let mut quay = Quay::open(holder, crate::session::call::hands());
    quay.seat(link).map_err(bcall::map_seat)?;
    quay.claim(holder, Mark::of(link.as_str()), millis)
        .map_err(bcall::map_claim)?;
    let board = hear(&quay, millis).ok_or(Fail::Unknown)?;
    Ok((quay, board))
}

/// 客侧第一步半：铸**问话孔**并交到板手里（本端随即自窄到只写）。
///
/// `board` = [`open`] 收下的那个号。交出去的是可读可写，随后本端 `narrow` 到 `STORE`：
/// 一条路上只有一个读者（`session` 事实 2），故**板读、本端写**。
///
/// 记号 = [`ASK_MARK`]：板那侧就是按它把这枚孔与**入口**分开的（两枚都由本端铸、本端交）。
pub fn ask_hole(board: TaskId) -> Result<PieToken, Fail> {
    // **一个域只铸一枚问话孔**：与我这一面同一句（见 `operator::client::ask_hole` 的照实记）
    // ——先找我表里那一枚，有就不铸第二枚。板那一侧按 `(开者, 记号)` 两格认孔，故第二枚的
    // 症状是"多出来的那枚永远没人读它的推"。
    if let Some(have) = crate::session::call::find(me()?, ASK_MARK) {
        return Ok(have);
    }
    let ask = mail::unseal_hole(ASK_MARK).map_err(|_| Fail::Denied)?;
    let hole = mail::HolePie::from_token(ask);
    port::ship(&hole, board, Access::FETCH | Access::STORE, Policy::NONE)
        .map_err(|_| Fail::Denied)?;
    hole.narrow(env::Permission::STORE)
        .map_err(|_| Fail::Denied)?;
    Ok(ask)
}

/// **本端是哪一枚线程**（"这一枚孔是谁开的"那一问要它；同 `operator` 那一面）。
fn me() -> Result<TaskId, Fail> {
    runtime::env::unit::self_id().map_err(|_| Fail::Unknown)
}

/// 客侧第二步（**登记那一句**）：报上名字 ＋ 把入口交出去，取一句答。
/// 返答话那一格（[`bcall::OK`] = 板收下了）。
///
/// 问话推 `say`（[`ask_hole`] 铸的那一枚，板读），答话从本端这条板路读（板写）。
///
/// **入口要捎上**：它经会话交给板（`Accord` 一份），故写进帧里的是"种在板表里的那个号"
/// ——那个号才是板认得的坐标（两个编号空间不同源，互相拿错正是旧树 `[33..41]` 那一格的病）。
///
/// **照实记（`op: u8` 那一格退场）**：这一手从前叫 `ask(…, op: u8, …)`，而树内**四个调用点
/// 全递 `REGISTER`**（回显、两位探针、常客）；注销与查两条动作到今天没有客人（板那一侧的
/// 正文记着"板上那个 `LOOKUP` 此后没有真客人"）。故客侧这一手直接叫 [`bcall::Req::Register`]
/// 那一条动作的名字，"这一码才带 seed" 那条分辨随之退场——形状由 [`bcall::Req`] 说。
pub fn register(
    say: PieToken,
    link: &Quay,
    board: TaskId,
    name: Name,
    entry: PieToken,
    millis: Wait,
) -> Result<u8, Fail> {
    // 先把入口交出去、换回"它在板表里是几号"，再编帧——两个编号空间不同源。
    let seed = bcall::ship(entry, board).map_err(|_| Fail::Denied)?;
    // 装上、发出去——**一帧＝一条报**（偏移与长度不在这层：字段表与 `Message` 说）。
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`push` 满则挂），不是错误。
    Slip::<bcall::Req>::seal(say)
        .load(bcall::Req::Register { name, seed })
        .ship()
        .map_err(|_| Fail::Unknown)?;
    hear_rep(link, millis)
}

/// 客侧第四步：说一句"**我走了**"，收一格答话。
///
/// 与 [`register`] 同一对动作（推一句问话、从本端板路取一句答话），只少两样：**没有载荷**
/// （不说名字、不交入口）与**不带板的号**（没什么要 `ship` 给板的）。
///
/// 板那侧据此撤格 + 摘掉这一位挂在板上的**全部**牌子；它不在账上则答
/// [`UNKNOWN`](bcall::UNKNOWN)。
pub fn evict(say: PieToken, link: &Quay, millis: Wait) -> Result<u8, Fail> {
    // 孔是单槽：与 [`register`] 同一条路，只是这一条报短（长度由形状说）。
    Slip::<bcall::Req>::seal(say)
        .load(bcall::Req::Evict)
        .ship()
        .map_err(|_| Fail::Unknown)?;
    hear_rep(link, millis)
}

// 照实记（删掉的一处：板那一面的 `take`）：它从前在这儿，形状与 `operator::take` 逐字同构
// ——"板刚授进来的那一枚"按**来源位是板**（`vestor`）扫本表认回来。**它一天都没有调用者**：
// 客人拿板查到的那个入口走的是另一条路（`service.rs::face_of` 按"开者 = 板 ＋ 记号 = 入口"
// 问，`session::call::find` 那两格正判据），板这一面从来不靠"谁给的"认。
//
// 按"没有读者的格不留在协议面上"的口径删掉。它顺带也是 `vestor` 在扫描里的**第二个**读者
// （另一个是 `operator::take`，那一个活的）——乙′ 那一步要把 `vestor` 从 `Collect` 上拿掉，
// 这就是先少掉的一处。

/// 收下板路上那一格：**一句答**（登记与退场共用）——[`hear`] 的答话侧对偶。
///
/// 返答话那一格码（[`bcall::OK`] = 板收下了），或 `Unknown`：`land` 那个 `None` 盖着
/// "期限到了 / 读不懂 / 孔空了"，问的人拿这一个码决定要不要重问。
fn hear_rep(link: &Quay, millis: Wait) -> Result<u8, Fail> {
    let at = Name::new(LINK).map_err(|_| Fail::Unknown)?;
    let pier = link.find(at).ok_or(Fail::Unknown)?;
    // 收帧的缓冲由调用方给：这条答话路只有板会写 ⇒ 本族那只空缓冲（[`Message::EMPTY`]）就够。
    let mut buf = bcall::Union::EMPTY;
    Slip::<bcall::Union>::seal(pier.hole())
        .land(buf.as_mut(), millis)
        .map(bcall::Union::get)
        .ok_or(Fail::Unknown)
}

/// 收下板路上那一格：**答话的是谁**（装配侧 `programs/src/system/board/bridge.rs` 的 `tell`
/// 的对偶）。宽度与字节序归 [`Field`](env::wire::Field) 给 [`TaskId`] 那一对
/// `store` / `fetch`——这一格从前在五处各写一遍（那一对里记着）。
///
/// 返 `None` = 期限到了还没到 ⇒ 这条服务没接上板（客人报它自己的超时，不猜）。
pub(crate) fn hear(quay: &Quay, millis: Wait) -> Option<TaskId> {
    let link = Name::new(LINK).ok()?;
    let pier = quay.find(link)?;
    let mut buf = [0u8; TaskId::WIDTH];
    match mail::HolePie::from_token(pier.hole()).pull_timeout(&mut buf, millis) {
        Ok(n) if n == TaskId::WIDTH => TaskId::fetch(&buf),
        _ => None,
    }
}
