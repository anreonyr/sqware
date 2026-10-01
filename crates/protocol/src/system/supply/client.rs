//! supply::client — **编排域那一侧**：递一张单子、取回一段记录（[`draw`]），并按坐标取一枚（[`pick`]）

use env::Wait;
use env::{Key, Pair};
use env::{MailFail, PieToken, TaskId};

use crate::communication::establish::Endpoint;
use crate::communication::receiver::RecvFail;
use crate::system::supply::frame::Fail;
use crate::system::supply::frame::{OK, Order, Reply, WANT_MAX, Want, code_to_fail};

/// 递一张单子、取回那一张回单（几条记录由那一帧自己说）。
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
    let order = Order::of(who, wants).ok_or(Fail::Local)?;
    let mut tx = pair.sender::<Order>().ok_or(Fail::Local)?;
    // **递出即返回**：一个 envcall（`Wait::POLL`）；等它下线压到这一趟收完（`reclaim`／`Drop`）。
    tx.send(order).map_err(|_| Fail::Local)?;
    let said = match pair.receiver::<Reply>().recv(reply, millis) {
        Ok(said) => said,
        Err(RecvFail::Mail(e)) => match e {
            MailFail::Dead | MailFail::Denied => return Err(Fail::Denied),
            _ => return Err(Fail::Local),
        },
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
pub fn pick(records: &[Pair], key: Key) -> Option<PieToken> {
    records
        .iter()
        .find(|pair| pair.key() == Some(key))
        .map(|pair| pair.token())
}
