//! supply::client — **编排域那一侧**：递一张单子、取回一段记录（[`draw`]），并按坐标取一枚（[`pick`]）
//!
//! **照实记（这一份的前身）**：它从前住那个只做形与据的 crate（`driver::supply::client`）
//! ——那时它手里只有会话核心那条泊位（`Pier::post` / `Pier::pull`），编解还是自由函数。它要用
//! **通信那一层**：它同时看得见"孔"（`runtime`）与"报"（`message`）。**判据一字未改**——
//! 尤其"`Local` 与 `Bad` 分得开"那一条（见 [`draw`]）。

use env::Wait;
use env::wire::Field;
use env::{MailFail, PieToken, TaskId};
use plan::{Key, PAIR_LEN, Pair};

use crate::driver::supply::core::Fail;
use crate::driver::supply::frame::{OK, Order, Reply, ReplyHead, WANT_MAX, Want, code_to_fail};
use crate::communication::establish::Endpoint;
use crate::communication::receiver::RecvFail;

/// 递一张单子、取回那一段记录。返**记录那一段**（`PAIR_LEN` 步长；借着调用方那只收帧缓冲）。
pub fn draw<'r>(
    pair: &Endpoint,
    who: TaskId,
    wants: &[Want],
    reply: &'r mut [u8],
    millis: Wait,
) -> Result<&'r [u8], Fail> {
    if wants.is_empty() || wants.len() > WANT_MAX {
        return Err(Fail::Local);
    }
    // 编一张单子、推过去：**编在本族那只缓冲里**（＝本族最长那一只，在这一帧的栈上）。
    // 泊位那头还没齐（`at_peer` 空）⇒ 与从前 `Pier::post` 自己那一格同一落点：`Local`。
    let order = Order::of(who, wants).ok_or(Fail::Local)?;
    let tx = pair.sender::<Order>().ok_or(Fail::Local)?;
    tx.send(order, Wait::Forever)
        .map_err(|_| Fail::Local)?;
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
        Err(RecvFail::Unread) => return Err(Fail::Bad),
    };
    match said.code() {
        // **记录那一段就是原样交给客人的那一段**：从**调用方那只缓冲**里切——帧长由解出来的
        // 条数定（`fetch` 已判过恰好），故切出来正好。
        OK => reply
            .get(ReplyHead::LEN..ReplyHead::LEN + said.records().len() * PAIR_LEN)
            .ok_or(Fail::Bad),
        code => Err(code_to_fail(code).unwrap_or(Fail::Bad)),
    }
}

/// 按坐标从记录里取一枚——**编排域自己领的那几样**用它（它们不按位次归位）。
///
/// **照实记（这一手的 `unsafe` 退场）**：它从前自己算步长、`read_unaligned` 一条条读
/// （缓冲只保证 1 字节对齐）——今天按 [`Pair`] 那一格解（`Field` 那一对就是为这件事立的），
/// 于是这里只剩一句"切、读、比"。
pub fn pick(records: &[u8], key: Key) -> Option<PieToken> {
    records
        .chunks_exact(PAIR_LEN)
        .filter_map(<Pair as Field>::fetch)
        .find(|pair| pair.key() == Some(key))
        .map(|pair| pair.token())
}
