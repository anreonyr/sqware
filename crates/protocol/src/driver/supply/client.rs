//! supply::client — **编排域那一侧**：递一张单子、取回一段记录（[`draw`]），并按名字取一枚（[`pick`]）
//!
//! 正文见 [`super`]；记号、帧与上限见 [`crate::driver::supply::call`]。

use env::{PAIR_LEN, Pair, PieToken, TaskId};

use crate::session::Pier;

use super::call::{OK, WANT_MAX, Want, code_to_fail, pack_order, unpack_reply};
use super::core::Fail;

pub fn draw<'r>(
    pier: &Pier,
    who: TaskId,
    wants: &[Want],
    ask: &mut [u8],
    reply: &'r mut [u8],
    millis: usize,
) -> Result<&'r [u8], Fail> {
    if wants.is_empty() || wants.len() > WANT_MAX {
        return Err(Fail::Local);
    }
    let frame = pack_order(ask, who, wants).ok_or(Fail::Local)?;
    pier.post(frame).map_err(|_| Fail::Local)?;
    let n = pier.pull(reply, millis).map_err(|_| Fail::Local)?;
    let got: &'r [u8] = reply;
    let got = got.get(..n).ok_or(Fail::Bad)?;
    let reply = unpack_reply(got).ok_or(Fail::Bad)?;
    match reply.code() {
        OK => Ok(reply.records()),
        code => Err(code_to_fail(code).unwrap_or(Fail::Bad)),
    }
}

/// 按名字从记录里取一枚——**编排域自己领的那几样**用它（它们不按 `Slot` 归位）。
pub fn pick(records: &[u8], want: &str) -> Option<PieToken> {
    for i in 0..records.len() / PAIR_LEN {
        // SAFETY: 同 [`unpack`]：记录与块同源，步长由编译期断言锁死，缓冲只保证
        // 1 字节对齐 ⇒ `read_unaligned`；越界由上面的除法挡掉。
        let at = unsafe { records.as_ptr().add(i * PAIR_LEN) };
        let record = unsafe { core::ptr::read_unaligned(at.cast::<Pair>()) };
        if record.name().is_some_and(|n| n.as_str() == want) {
            return Some(record.token());
        }
    }
    None
}
