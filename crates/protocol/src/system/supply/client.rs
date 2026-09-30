//! supply::client — **编排域那一侧**：递一张单子、取回一段记录（[`draw`]），并按坐标取一枚（[`pick`]）
//!
//! **照实记（这一份的前身）**：它从前住那个只做形与据的 crate（那一份里的 `supply::client`）
//! ——那时它手里只有会话核心那条泊位（`Pier::post` / `Pier::pull`），编解还是自由函数。它要用
//! **通信那一层**：它同时看得见"孔"（`runtime`）与"报"（`message`）。**判据一字未改**——
//! 尤其"`Local` 与 `Bad` 分得开"那一条（见 [`draw`]）。

use env::Wait;
use env::{Key, Pair};
use env::{MailFail, PieToken, TaskId};

use crate::communication::establish::Endpoint;
use crate::communication::receiver::RecvFail;
use crate::system::supply::frame::Fail;
use crate::system::supply::frame::{OK, Order, Reply, WANT_MAX, Want, code_to_fail};

/// 递一张单子、取回那一张回单（几条记录由那一帧自己说）。
///
/// **照实记（返回那一形为什么从"一段裸字节"改成 [`Reply`]）**：从前它返的是"调用方那只缓冲里
/// 切出来的记录段"——切的那一刀要用 `ReplyHead::LEN` 那个偏移，而头那两格已经并进本表（偏移
/// 不再另有一处）。解码本来就已经把那几条 [`Pair`] 收进 `Reply` 里了（`recv` 里的 `fetch`），
/// 故这里返 `Reply`（`Copy`）**不多一次拷贝**，只是不再第二次解释同一段字节。
pub fn draw(
    pair: &Endpoint,
    who: TaskId,
    wants: &[Want],
    reply: &mut [u8],
    millis: Wait,
) -> Result<Reply, Fail> {
    if wants.is_empty() || wants.len() > WANT_MAX {
        return Err(Fail::Local);
    }
    // 编一张单子、推过去：**编在本族那只缓冲里**（＝本族最长那一只，在这一帧的栈上）。
    // 泊位那头还没齐（`at_peer` 空）⇒ 与从前 `Pier::post` 自己那一格同一落点：`Local`。
    let order = Order::of(who, wants).ok_or(Fail::Local)?;
    let mut tx = pair.sender::<Order>().ok_or(Fail::Local)?;
    // **递出即返回**：一个 envcall（`Wait::POLL`）；等它下线压到这一趟收完（`reclaim`／`Drop`）。
    tx.send(order).map_err(|_| Fail::Local)?;
    // 收一张回单：**三格失败分得开**（[`Land`] 就是为这一格立的）——"期限内没等到" ⇒ `Local`；
    // "这一枚孔用不动了" ⇒ `Denied`（"这一手没做成"）；"收下来解不动" ⇒ `Bad`。前两句与从前
    // `pier.pull` ＋ `fetch` 那两句一字不差，第三句是"孔用不动了"那一格加进来之后才分开的
    // （从前它与 `Expired` 合流）。
    let said = match pair.receiver::<Reply>().recv(reply, millis) {
        Ok(said) => said,
        // **三格失败分得开**（判据与从前那张 `Land` 对照表一字不差）：`Dead` / `Denied`
        // = "这一枚孔用不动了" ⇒ `Denied`；其余（`Busy` 没消息 / `OoM` / `HandedOver`）
        // ⇒ `Local`；"收下来解不动" ⇒ `Bad`。
        Err(RecvFail::Mail(e)) => match e {
            MailFail::Dead | MailFail::Denied => return Err(Fail::Denied),
            _ => return Err(Fail::Local),
        },
        // **"收下来解不动"那一格带上读到了几字节**（照实记见 `RecvFail::Unread`）：从前的空变体
        // 说不出"帧坏了"与"缓冲短了"是哪一件。
        Err(RecvFail::Unread(len)) => {
            crate::debug!("supply: recv unread len={len}");
            return Err(Fail::Bad);
        }
    };
    // 回单回来了（或这一趟判了失败）⇒ 把那一手收口：对面取走了是零代价，没取走就等它取
    // （**那条报不许悬**）。
    let _ = tx.reclaim();
    match said.code() {
        OK => Ok(said),
        code => Err(code_to_fail(code).unwrap_or(Fail::Bad)),
    }
}

/// 按坐标从记录里取一枚——**编排域自己领的那几样**用它（它们不按位次归位）。
///
/// **照实记（这一手的 `unsafe` 退场）**：它从前自己算步长、`read_unaligned` 一条条读
/// （缓冲只保证 1 字节对齐）——今天按 [`Pair`] 那一格解（`Field` 那一对就是为这件事立的），
/// 于是这里只剩一句"读、比"。
pub fn pick(records: &[Pair], key: Key) -> Option<PieToken> {
    records
        .iter()
        .find(|pair| pair.key() == Some(key))
        .map(|pair| pair.token())
}
