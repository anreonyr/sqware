//! supply::client — **编排域那一侧**：递一张单子、取回一段记录（[`draw`]），并按坐标取一枚（[`pick`]）
//!
//! **照实记（这一份为什么搬到 `protocol`）**：它从前住「约」（`contract::driver::supply::client`）
//! ——那时它手里只有会话核心那条泊位（`Pier::post` / `Pier::pull`），编解还是自由函数。搬过来是
//! 因为它要用**船台**：`Slip` 同时看得见"孔"（`runtime`）与"报"（`contract`），而那一层只有
//! `protocol` 有。**判据一字未改**——尤其"`Local` 与 `Bad` 分得开"那一条（见 [`draw`]）。

use env::wire::Field;
use env::Wait;
use env::{PieToken, TaskId};
use plan::{Key, PAIR_LEN, Pair};

use crate::driver::supply::core::Fail;
use crate::driver::supply::frame::{OK, Order, Reply, ReplyHead, WANT_MAX, Want, code_to_fail};
use crate::session::Pier;
use crate::session::slip::{Land, Slip};

/// 递一张单子、取回那一段记录。返**记录那一段**（`PAIR_LEN` 步长；借着调用方那只收帧缓冲）。
pub fn draw<'r>(
    pier: &Pier,
    who: TaskId,
    wants: &[Want],
    reply: &'r mut [u8],
    millis: Wait,
) -> Result<&'r [u8], Fail> {
    if wants.is_empty() || wants.len() > WANT_MAX {
        return Err(Fail::Local);
    }
    // 编一张单子、推过去：**上船台**（编进船台自己那只缓冲＝本族最长那一只）。
    // 泊位那头还没齐（`at_peer` 空）⇒ 与从前 `Pier::post` 自己那一格同一落点：`Local`。
    let order = Order::of(who, wants).ok_or(Fail::Local)?;
    let at_peer = pier.at_peer().ok_or(Fail::Local)?;
    Slip::<Order>::seal(at_peer)
        .load(order)
        .ship()
        .map_err(|_| Fail::Local)?;
    // 收一张回单：**两格失败分得开**（[`Land`] 就是为这一格立的）——"期限内没等到" ⇒ `Local`；
    // "收下来解不动" ⇒ `Bad`。这两句话与从前 `pier.pull` ＋ `fetch` 那两句一字不差。
    let slip = Slip::<Reply>::seal(pier.hole());
    let said = match slip.land(reply, millis) {
        Ok(said) => said,
        Err(Land::Expired) => return Err(Fail::Local),
        Err(Land::Unread) => return Err(Fail::Bad),
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
