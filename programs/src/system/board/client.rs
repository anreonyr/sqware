//! board::client — **客侧**：一条会话 ＋ 一问一答（说「我走了」也在这一侧）
//!
//! **照实记（它原先住 `crates/protocol/src/system/board/client.rs`）**：按裁定「board 是编排域的
//! **死信号传感器**，不是第五轴」，这一族从 protocol 那一层退出聚合——那里只留**形与记号**
//! （`frame.rs` / `Fail` / `LINK` / `ASK_MARK` / `TIP_MARK` / `LANE_PREFIX` / `ENTRY_MARK`），
//! **说话的那一侧**（本文件）回到实现侧。故 use 改两处：记号与码取
//! [`protocol::system::board`]，`Session` / `Berth` 取
//! [`protocol::communication::session`]。
//!
//! 三侧分家之后本文件只放**客侧**：**装板路 / 铸问话孔那两手不在这里**——它们与
//! `operator::client` 那两手逐字同构，已按"两台以上逐字同构 ⇒ 收"抬进
//! [`protocol::communication::session`]；本文件只声明**这条路叫什么**（[`BERTH`]）＋ **报到**
//! （[`enroll`]）与一问一答（[`register`] / [`evict`]）。两侧共用的图与次序说明见 [`super`]，
//! 帧与记号见 [`protocol::system::board`]。

use alloc::string::String;
use alloc::string::ToString;
use env::Mark;
use env::Wait;
use env::{PieToken, TaskId};
use protocol::message::Message;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

use protocol::communication::establish::Endpoint;
use protocol::communication::sender::Sender;
use protocol::communication::session::{Berth, Session};
use protocol::system::board as bcall;
use protocol::system::board::Fail;
pub use protocol::system::board::{ASK_MARK, ENTRY_MARK, LINK};

/// **这条路叫什么**：泊位那一格（`LINK` = `board`）＋ 问话孔那一格（`ASK_MARK`）。
///
/// 开会话那一手（[`Session::open`]）要它；本层只把这两格交出去，不替调用方开会话。
pub const BERTH: Berth = Berth {
    link: Mark::of(bcall::LINK),
    ask: bcall::ASK_MARK,
};

/// **报到**：本域那枚服务入口挂上板（板据此按名字分人，也据此看得见本域的死）。
///
/// 返**板的答码**（[`bcall::OK`] = 板收下了）与**那一枚入口**（读数要那一格：`passer` 把它
/// 打出来）。挂不上**不是**本域的失败：终端照旧干活，只是"本域死了"那条信号缺席
/// （见 `programs/src/user/canonical/main.rs`）——故返码、不返 `Result`。
///
/// **它把三件事收成一手**：解本域那枚入口（`ENTRY_MARK`）、把名字编成 [`String`]、经 [`register`]
/// 交出去。四处调用点原先各写一遍（`canonical` / `passer` / `guest` / `sleeper`）。
pub fn enroll(session: &Session, me: &str, millis: Wait) -> (u8, PieToken) {
    let Ok(entry) = mail::unseal_hole(bcall::ENTRY_MARK) else {
        return (bcall::BAD, PieToken::NONE);
    };
    let name = me.to_string();
    match register(
        session.talk,
        &session.link,
        session.host,
        name,
        entry,
        millis,
    ) {
        Ok(code) => (code, entry),
        Err(_) => (bcall::BAD, entry),
    }
}

// 照实记（原先这里的三手 `open` / `ask_hole` / `me`）：它们与 `operator::client` 那三手
// **逐字同构**（只差两个记号与各自的失败域），已抬进 [`protocol::communication::session`]
// ——开会话那一手归地板。本文件因此只剩**这条路的名字**（[`BERTH`]）与这一族那几手。

/// 客侧第二步（**登记那一句**）：报上名字 ＋ 把入口交出去，取一句答。
/// 返答话那一格（[`bcall::OK`] = 板收下了）。
///
/// 问话推 `say`（开会话那一手铸的问话孔，板读），答话从本端这条板路读（板写）。
///
/// **入口要捎上**：它经会话交给板（`Accord` 一份），故写进帧里的是"种在板表里的那个号"
/// ——那个号才是板认得的坐标（两个编号空间不同源，互相拿错正是旧树 `[33..41]` 那一格的病）。
pub fn register(
    say: PieToken,
    link: &Endpoint,
    board: TaskId,
    name: String,
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

/// 收下板路上那一格：**一句答**（登记与退场共用）。
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

// 照实记（原先这里还有两件）：`hear`（收"答话的是谁"）与 `map_establish`（建立那一手的失败域
// 对照表）。前者与 `operator::client` 那一份逐字同构 ⇒ 随开会话那一手抬进
// [`protocol::communication::session`]；后者只服务那一手 ⇒ 与它的唯一读者一起退场。
