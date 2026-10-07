//! 登记那一句话——报线号 ⇒ 占住那一格 + 接上线
//! ⇒ 回一格状态码。

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::core::lines::Lines;
use crate::dev::plic::{LINE_PRIORITY, Plic};
use env::{HoleDir, Mark, TaskId, Wait};
use ipc::session::establish::{self, Held};
use protocol::debug;
use protocol::driver::line::frame as lcall;
use protocol::wire::message::Message;
use ::resource::pile::Pile;
use env::pie;
use ::resource::raw::{Hole, table_size};

/// 装泊位 / 认泊位的期限（毫秒）
const QUAY_MS: usize = 1000;

/// **一格答话存根**：那**一个字节**（登记那一答只有一格状态码）＋ 它欠着谁
pub struct Reply {
    /// 哪一位客人（回信孔的主人）
    pub who: TaskId,
    /// 那一个字节。**地址必须稳**：那只手记的是推出去那一刻的地址
    pub byte: u8,
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
        self.slots.push(Box::new(Reply { who, byte: 0 }));
        self.slots.last_mut().map(|b| &mut **b)
    }
}

/// 门上那一句话：**登记**（带动作码）——报**线号** ⇒ 占住那一格 + 接上线 ⇒ 回一格状态码
/// 答话推到**客人借过来的那枚回信孔**上（按记号认：那位给的多枚孔靠记号分开）
pub fn serve(
    lines: &mut Lines,
    plic: &Plic,
    from: TaskId,
    frame: &[u8],
    pile: &Pile,
    replies: &mut Replies,
) {
    if let Some(line) = <lcall::Occupy as Message>::fetch(frame) {
        let code = match take_lane(from) {
            // 客户没把泊位交出来（或交不出来）。
            None => lcall::DENIED,
            Some(lane) => match lines.occupy(line, lane) {
                Ok(()) => {
                    // **接线是登记的直接后果。**
                    plic.enable(line, LINE_PRIORITY);
                    // ——那一格是**事件**，不是节拍（挂的是本端读的那一枚，见 `exhaust`）。
                    if let Some(lane) = lines.lane(line) {
                        let _ = pile.attach(lane.rx(), HoleDir::Pull);
                    }
                    debug!("router: line {line} occupied");
                    lcall::OK
                }
                Err(fail) => {
                    debug!(
                        "router: lane dropped line={line} pies={}",
                        table_size()
                    );
                    lcall::fail_to_code(Some(fail))
                }
            },
        };
        if let Some(back) = establish::find(from, lcall::BACK_MARK) {
            let reply = Hole::from_raw(back);
            // **一个字节住进"跟着客人走"的那一格**（见 Reply），**推完就走**：
            match replies.slot(from) {
                Some(slot) => {
                    slot.byte = code;
                    let _ = reply.push(core::slice::from_ref(&slot.byte), Wait::POLL);
                }
                None => debug!("router: no reply slot from={}", from.get()),
            }
            let _ = pie::release(back);
        }
    }
}

/// 认下这位客户交出来的**线泊位**（记号 lcall::LANE），并把本端那一枚交给它
/// 返**那条路的持有者**（Held：本端读的那一枚 ＋ 认下来的写端）：`deliver` 往**它**推
/// 投递（客户读的那一枚），`exhaust` 收**它的**排空
/// **一手就是"两头都装"**（establish::endpoint：铸本端那一枚交给它 ＋ 认下它那一枚，判据
fn take_lane(from: TaskId) -> Option<Held> {
    // **有主地建**（`Held(..)`：那一格"有主"由类型说出来）。
    let lane = Held(establish::endpoint(from, Mark::of(lcall::LANE), Wait::AtMost(QUAY_MS)).ok()?);
    // **没有写端就投不出去**：这条泊位不成立。
    lane.tx()?;
    Some(lane)
}
