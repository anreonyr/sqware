//! board::client — **客侧三手**：装上板路、铸问话孔、一问一答（说「我走了」也在这一侧）
//!
//! 三侧分家之后本文件只放**客侧三手**：装上板路、铸问话孔、一问一答（说「我走了」也在这一侧）；
//! 两侧共用的图与次序说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`crate::system::board`]。

use crate::message::Message;
use env::Mark;
use env::Wait;
use env::wire::Field;
use env::{Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use crate::communication::establish::{self, Endpoint, EstablishFail};
use crate::communication::sender::Sender;
use crate::system::board as bcall;
use crate::system::board::Fail;
pub use crate::system::board::{ASK_MARK, ENTRY_MARK, LINK};

/// 客侧第一步：装上板那条路（**记号就是这条路的名字**），认下对端那一枚，并收下
/// "**答话的是谁**"（[`hear`] 那一格）。
///
/// 返本端这一对孔（**答话**从 `rx` 读，问话走 [`ask_hole`]）与**板线程的号**。
///
/// `holder` = 客人认的对端 = **它的生我者**（孔交给它，它再转授给板线程）——注意它不是板：
/// 客人交出来的孔都落在生我者表里，故"板是谁"得由装配者告诉（见文件头），此后客人交孔、
/// 交入口才叫得出板。
pub fn open(holder: TaskId, millis: Wait) -> Result<(Endpoint, TaskId), Fail> {
    let pair =
        establish::endpoint(holder, Mark::of(LINK), millis).map_err(map_establish)?;
    // **认不到对端那一枚 = 这条板路没接上**（原 `map_claim` 那一格）：本端这一侧虽然只读答话，
    // 但"两侧各装一条、凑齐才算通"那条不变量仍在——没齐就是没接上，不必等到第一次收帧。
    if pair.tx().is_none() {
        return Err(Fail::Unknown);
    }
    let board = hear(&pair, millis).ok_or(Fail::Unknown)?;
    Ok((pair, board))
}

/// 客侧第一步半：铸**问话孔**并交到板手里（本端随即自窄到只写）。
///
/// `board` = [`open`] 收下的那个号。交出去的是可读可写，随后本端 `narrow` 到 `STORE`：
/// 一条路上只有一个读者（`Receiver::recv` 的照实记），故**板读、本端写**。
///
/// 记号 = [`ASK_MARK`]：板那侧就是按它把这枚孔与**入口**分开的（两枚都由本端铸、本端交）。
pub fn ask_hole(board: TaskId) -> Result<PieToken, Fail> {
    // **一个域只铸一枚问话孔**：与我这一面同一句（见 `operator::client::ask_hole` 的照实记）
    // ——先找我表里那一枚，有就不铸第二枚。板那一侧按 `(开者, 记号)` 两格认孔，故第二枚的
    // 症状是"多出来的那枚永远没人读它的推"。
    if let Some(have) = establish::find(me(), ASK_MARK) {
        return Ok(have);
    }
    // 铸 + 交出读端 + 本端窄到只写：一手就是 `establish::give`。
    establish::give(board, ASK_MARK).map_err(|_| Fail::Denied)
}

/// **本端是哪一枚线程**（"这一枚孔是谁开的"那一问要它；同 `operator` 那一面）。
///
/// 不返 `Result`：`SelfId` 那一格恒写 id（生成的入口标了 `#[infallible]`）。
fn me() -> TaskId {
    runtime::env::unit::self_id()
}

/// 客侧第二步（**登记那一句**）：报上名字 ＋ 把入口交出去，取一句答。
/// 返答话那一格（[`bcall::OK`] = 板收下了）。
///
/// 问话推 `say`（[`ask_hole`] 铸的那一枚，板读），答话从本端这条板路读（板写）。
///
/// **入口要捎上**：它经会话交给板（`Accord` 一份），故写进帧里的是"种在板表里的那个号"
/// ——那个号才是板认得的坐标（两个编号空间不同源，互相拿错正是旧树 `[33..41]` 那一格的病）。
pub fn register(
    say: PieToken,
    link: &Endpoint,
    board: TaskId,
    name: Name,
    entry: PieToken,
    millis: Wait,
) -> Result<u8, Fail> {
    // 先把入口交出去、换回"它在板表里是几号"，再编帧——两个编号空间不同源。
    // **交出那一手就是 `port::ship` 本身**（`establish` 那条照实记：壳一个不留），
    // 权限 `R|W` ＋ 一格 `VEST`：板的本职是**再授出**（`Query` 的下场），
    // 少 `VEST` ⇒ 板转授那一步答 `Denied`、看上去像"板坏了"。
    let pie = mail::HolePie::from_token(entry);
    let seed = port::ship(&pie, board, Access::FETCH | Access::STORE, Policy::VEST)
        .map(|to| to.seed())
        .map_err(|_| Fail::Denied)?;
    // 装上、发出去——**一帧＝一条报**（偏移与长度不在这层：字段表与 `Message` 说）。
    // 孔是单槽：槽里还压着上一条时这一推会**等在门外**（`send` 满则挂），不是错误。
    Sender::<bcall::Req>::from_token(say)
        .send(bcall::Req::Register { name, seed }, Wait::Forever)
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
pub fn evict(say: PieToken, link: &Endpoint, millis: Wait) -> Result<u8, Fail> {
    // 孔是单槽：与 [`register`] 同一条路，只是这一条报短（长度由形状说）。
    Sender::<bcall::Req>::from_token(say)
        .send(bcall::Req::Evict, Wait::Forever)
        .map_err(|_| Fail::Unknown)?;
    hear_rep(link, millis)
}

/// 收下板路上那一格：**一句答**（登记与退场共用）——[`hear`] 的答话侧对偶。
///
/// 返答话那一格码（[`bcall::OK`] = 板收下了），或 `Unknown`：`recv` 那个失败盖着
/// "期限到了 / 读不懂 / 孔空了"，问的人拿这一个码决定要不要重问。
fn hear_rep(link: &Endpoint, millis: Wait) -> Result<u8, Fail> {
    // 收帧的缓冲由调用方给：这条答话路只有板会写 ⇒ 本族那只空缓冲（[`Message::EMPTY`]）就够。
    let mut buf = bcall::Union::EMPTY;
    link.receiver::<bcall::Union>()
        .recv(buf.as_mut(), millis)
        .map(bcall::Union::get)
        // 两格失败（没收到 / 解不动）在这一侧落同一格：对本端是同一个下一步。
        .map_err(|_| Fail::Unknown)
}

/// 收下板路上那一格：**答话的是谁**（装配侧 `programs/src/system/board/bridge.rs` 的 `tell`
/// 的对偶）。宽度与字节序归 [`Field`](env::wire::Field) 给 [`TaskId`] 那一对
/// `store` / `fetch`——这一格从前在五处各写一遍（那一对里记着）。
///
/// 返 `None` = 期限到了还没到 ⇒ 这条服务没接上板（客人报它自己的超时，不猜）。
///
/// **这一格是裸字节**（8 字节的那个号），不是这一族那两种报 ⇒ 走裸孔，不套手柄。
pub(crate) fn hear(pair: &Endpoint, millis: Wait) -> Option<TaskId> {
    let mut buf = [0u8; TaskId::WIDTH];
    match mail::HolePie::from_token(pair.rx()).pull_timeout(&mut buf, millis) {
        Ok(n) if n == TaskId::WIDTH => TaskId::fetch(&buf),
        _ => None,
    }
}

// ── 建立那一手的失败域的对照表（原住 `protocol` 的 `system/board/call.rs`）────
//
// 入参出自 [`EstablishFail`]（`communication::establish`）、产出的又是本文件自己的
// [`Fail`]，故它与产出的那一格同住。原先有两张（`Seat` / `Claim`）——并回一个 crate 之后
// 只剩一手建立，`Claim` 那一张随之退场（认不到对端那一枚不再是错误，见 `establish`）。

/// 建立那一手的失败域 → 板的失败域。
///
/// **铸不出孔** ⇒ `Denied`（本端这一手没做成）；**交不出去** ⇒ `Unknown`
/// （它最常见的那一支是"对端已不在"）。
pub fn map_establish(fail: EstablishFail) -> Fail {
    match fail {
        EstablishFail::NoHole => Fail::Denied,
        EstablishFail::NoSeed => Fail::Unknown,
    }
}
