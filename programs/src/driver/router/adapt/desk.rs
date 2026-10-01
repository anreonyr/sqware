//! router::adapt::desk — **门面（适配）**：登记那一句话——报**线号** ⇒ 占住那一格 + 接上线
//! ⇒ 回一格状态码。
//! 判定在 `crate::core::lines`（`Lines` 的四原语）；本文件只做碰内核与设备的那几手：认泊位、
//! 解帧、接线、挂组、答话。**放回那一手不在这里**：登记被拒时那一条由 [`Held`] 的 `Drop`

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::core::lines::Lines;
use crate::plic::{LINE_PRIORITY, Plic};
use env::{HoleDir, Mark, TaskId, Wait};
use protocol::communication::establish::{self, Held};
use protocol::debug;
use protocol::driver::line::frame as lcall;
use protocol::message::Message;
use runtime::core::res::pile::Pile;
use runtime::env::mail::{self, HolePie};

/// 装泊位 / 认泊位的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// **一格答话存根**：那**一个字节**（登记那一答只有一格状态码）＋ 它欠着谁。
pub struct Reply {
    /// 哪一位客人（回信孔的主人）。
    pub who: TaskId,
    /// 那一个字节。**地址必须稳**：那只手记的是推出去那一刻的地址。
    pub byte: u8,
}

pub struct Replies {
    slots: Vec<Box<Reply>>,
}

impl Replies {
    pub const fn new() -> Self {
        Replies { slots: Vec::new() }
    }

    /// 取这位客人那一格（没有就立一格）。**备不下 ⇒ `None`**：那一趟不答（宁可少答一句，
    /// 也不让本域为一位客人停住，更不把一段活不过这一帧的字节指给客人）。
    fn slot(&mut self, who: TaskId) -> Option<&mut Reply> {
        if let Some(at) = self.slots.iter().position(|s| s.who == who) {
            return self.slots.get_mut(at).map(|b| &mut **b);
        }
        self.slots.try_reserve(1).ok()?;
        self.slots.push(Box::new(Reply { who, byte: 0 }));
        self.slots.last_mut().map(|b| &mut **b)
    }
}

/// 门上那一句话：**登记**（带动作码）——报**线号** ⇒ 占住那一格 + 接上线 ⇒ 回一格状态码。
/// 答话推到**客人借过来的那枚回信孔**上（按记号认：那位给的多枚孔靠记号分开）。
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
                    // **排空那条路也在这里挂上**：客人往它写一句"我排空了"，本域就被叫醒
                    // ——那一格是**事件**，不是节拍（挂的是本端读的那一枚，见 `exhaust`）。
                    if let Some(lane) = lines.lane(line) {
                        let _ = pile.attach(&HolePie::from_token(lane.rx()), HoleDir::Pull);
                    }
                    debug!("router: line {line} occupied");
                    lcall::OK
                }
                // **拒了就放回去**：这一趟刚交上来的那条路**不在账里**（`Lines::occupy`
                // 收下了它、又拒了它）——`lane` 是 [`Held`]，**它自己放下**，这里一行都
                // 不用写（旧形状里这一手是本文件的 `drop_lane`）。
                Err(fail) => {
                    debug!(
                        "router: lane dropped line={line} pies={}",
                        mail::table_size()
                    );
                    lcall::fail_to_code(Some(fail))
                }
            },
        };
        if let Some(back) = establish::find(from, lcall::BACK_MARK) {
            let reply = HolePie::from_token(back);
            // **一个字节住进"跟着客人走"的那一格**（见 [`Reply`]），**推完就走**：
            match replies.slot(from) {
                Some(slot) => {
                    slot.byte = code;
                    let _ = reply.push(core::slice::from_ref(&slot.byte), Wait::POLL);
                }
                // 备不下那一格：**这一趟不答**（客人自己的期限会叫它回头）；不拿这一帧的栈去顶。
                None => debug!("router: no reply slot from={}", from.get()),
            }
            // **答完就放下**：这一枚是这一趟借过来的（一问一答一个往返），它不在本域的账里
            // ——账里根本没有它，此后没人会替它收。不放的话，每有一次登记就在本域表里多留
            let _ = mail::release(back);
        }
    }
}

/// 认下这位客户交出来的**线泊位**（记号 [`lcall::LANE`]），并把本端那一枚交给它。
/// 返**那条路的持有者**（[`Held`]：本端读的那一枚 ＋ 认下来的写端）：`deliver` 往**它**推
/// 投递（客户读的那一枚），`exhaust` 收**它的**排空。
/// **一手就是"两头都装"**（[`establish::endpoint`]：铸本端那一枚交给它 ＋ 认下它那一枚，判据
fn take_lane(from: TaskId) -> Option<Held> {
    // **有主地建**（`Held(..)`：那一格"有主"由类型说出来）。
    let lane = Held(establish::endpoint(from, Mark::of(lcall::LANE), Wait::AtMost(QUAY_MS)).ok()?);
    // **没有写端就投不出去**：这条泊位不成立。
    lane.tx()?;
    Some(lane)
}
