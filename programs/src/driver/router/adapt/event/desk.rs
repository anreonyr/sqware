//! 登记那一句话——报线号 ⇒ 占住那一格 + 接上线
//! ⇒ 回一格状态码。

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::core::lines::Lines;
use crate::dev::plic::{LINE_PRIORITY, Plic};
use ::resource::pile::Pile;
use ::resource::raw::{Hole, table_size};
use env::{MailCondition, TaskId, Wait};
use ipc::session::Held;
use programs::debug;
use router_api::frame as lcall;
use wire::Message;

/// Reply storage stays at a stable address for the duration of the push.
pub struct Reply {
    /// 哪一位客人（回信孔的主人）
    pub who: TaskId,
    /// Response bytes remain at a stable address for the duration of the push.
    pub bytes: [u8; lcall::OccupyReply::LEN],
}

pub struct Replies {
    slots: Vec<Box<Reply>>,
}

impl Replies {
    pub const fn new() -> Self {
        Replies { slots: Vec::new() }
    }

    fn slot(&mut self, who: TaskId) -> Option<&mut Reply> {
        if let Some(at) = self.slots.iter().position(|s| s.who == who) {
            return self.slots.get_mut(at).map(|b| &mut **b);
        }
        self.slots.try_reserve(1).ok()?;
        self.slots.push(Box::new(Reply {
            who,
            bytes: lcall::OccupyReply::EMPTY,
        }));
        self.slots.last_mut().map(|b| &mut **b)
    }
}

/// 门上那一句话：**登记**（带动作码）——报**线号** ⇒ 占住那一格 + 接上线 ⇒ 回一格状态码
/// 答话推到请求明确携带、经来源核验的回信孔上。
pub fn serve(
    lines: &mut Lines,
    plic: &Plic,
    from: TaskId,
    frame: &[u8],
    pile: &Pile,
    replies: &mut Replies,
) {
    if let Some((line, lane_seed, back_seed)) = <lcall::OccupyLane as Message>::fetch(frame) {
        let Some(back) = super::handoff::back(from, back_seed) else {
            debug!("router: invalid reply capability from={}", from.get());
            return;
        };
        let (code, lane_reply) = match take_lane(from, lane_seed) {
            None => (lcall::DENIED, env::PieToken::NONE),
            Some(lane) => {
                let lane_reply = lane.seed();
                match lines.occupy(line, lane) {
                    Ok(()) => {
                        // **接线是登记的直接后果。**
                        plic.enable(line, LINE_PRIORITY);
                        // ——那一格是**事件**，不是节拍（挂的是本端读的那一枚，见 `exhaust`）。
                        if let Some(lane) = lines.lane(line) {
                            let _ = pile.attach(lane.rx(), MailCondition::Pull);
                        }
                        debug!("router: line {line} occupied");
                        (lcall::OK, lane_reply)
                    }
                    Err(fail) => {
                        debug!("router: lane dropped line={line} pies={}", table_size());
                        (lcall::fail_to_code(Some(fail)), env::PieToken::NONE)
                    }
                }
            }
        };
        let reply = Hole::from_raw(back);
        // The fixed reply buffer stays in the slot while the kernel reads it.
        match replies.slot(from) {
            Some(slot) => {
                let response = lcall::OccupyReply::of(code, lane_reply);
                if let Some(n) = response.store(&mut slot.bytes) {
                    let _ = reply.push(&slot.bytes[..n], Wait::POLL);
                }
            }
            None => debug!("router: no reply slot from={}", from.get()),
        }
        let _ = env::pie::release(back, env::ReleaseMode::Revoke);
    }
}

/// 接受请求明确交来的线泊位，并把本端那一枚交给它。
/// 返**那条路的持有者**（Held：本端读的那一枚 ＋ 认下来的写端）：`deliver` 往**它**推
/// 投递（客户读的那一枚），`exhaust` 收**它的**排空
fn take_lane(from: TaskId, token: env::PieToken) -> Option<Held> {
    super::handoff::lane(from, token)
}
