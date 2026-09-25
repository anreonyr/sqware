//! supply::client — **编排域那一侧**：递一张单子、取回一段记录（[`draw`]），并按坐标取一枚（[`pick`]）
//!
//! 正文见 [`super`]；记号、帧与上限见 [`crate::driver::supply::frame`]。

use env::wire::Field;
use env::Wait;
use env::{PieToken, TaskId};
use plan::{Key, PAIR_LEN, Pair};

use crate::message::Message;
use crate::session::Pier;

use crate::driver::supply::core::Fail;
use crate::driver::supply::frame::{OK, Order, Reply, ReplyHead, WANT_MAX, Want, code_to_fail};

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
    // 编一张单子（**一处编**：头那三格 ＋ 尾巴那一段），推过去。
    let order = Order::of(who, wants).ok_or(Fail::Local)?;
    let mut frame = Order::EMPTY;
    let n = order.store(&mut frame).ok_or(Fail::Local)?;
    pier.post(&frame[..n]).map_err(|_| Fail::Local)?;
    // 收一张回单：**长度那两格照旧**（"短一字节" / "条数说谎"都由 `fetch` 判）。
    let n = pier.pull(reply, millis).map_err(|_| Fail::Local)?;
    let got = reply.get(..n).ok_or(Fail::Bad)?;
    let said = <Reply as Message>::fetch(got).ok_or(Fail::Bad)?;
    match said.code() {
        // **记录那一段就是原样交给客人的那一段**（帧长已由 `fetch` 判过 ⇒ 切出来正好）。
        OK => got.get(ReplyHead::LEN..).ok_or(Fail::Bad),
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
