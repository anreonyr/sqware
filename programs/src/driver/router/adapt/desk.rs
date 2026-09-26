//! router::adapt::desk — **门面（适配）**：登记那一句话——报**那一段区** ⇒ 解树（"线 = 区的函数"）
//! ⇒ 占住那一格 + 接上线 ⇒ 回一格状态码。
//!
//! 判定在 `protocol::driver::line::core`（`Lines` 的四原语）与 `crate::core::sources`（区 → 线号）；
//! 本文件只做碰内核与设备的那几手：认泊位、解帧、接线、挂组、答话。**放回那一手也在这里**：
//! 登记被拒时那一条由本文件当场 [`Endpoint::close`]（见 [`serve`] 里那一支）——放下一手不由
//! 作用域替人做（`establish` 的照实记）。

use crate::core::sources::Sources;
use crate::plic::{LINE_PRIORITY, Plic};
use protocol::message::Message;
use env::{HoleDir, Mark, TaskId, Wait};
use protocol::debug;
use protocol::driver::line::{core::Lines, frame as lcall};
use protocol::communication::establish::{self, Endpoint};
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie};

/// 装泊位 / 认泊位的期限（毫秒）。
const QUAY_MS: usize = 1000;

/// 门上那一句话：**登记**（带动作码）——报**那一段区** ⇒ **解树**（"线 = 区的函数"，权威只在
/// 这一处）⇒ 占住那一格 + 接上线 ⇒ 回一格状态码。
///
/// 答话推到**客人借过来的那枚回信孔**上（按记号认：那位给的多枚孔靠记号分开）。
/// **照实记**：这一扇门从前还兼着"招呼"（旧形状：一个名字进、一个名字回）——那条路随旧 32
/// 字节形状一起退休了，今天只有登记一种形状（"找人"走树，见 `guest`）。
pub fn serve(
    lines: &mut Lines,
    plic: &Plic,
    sources: &Sources,
    from: TaskId,
    frame: &[u8],
    pile: &Pile,
) {
    if let Some(key) = <lcall::Occupy as Message>::fetch(frame) {
        let code = match sources.line_of(key) {
            // 树里没这条线 ⇒ 那个坐标不是中断源（线挂在设备上，别的形自然落这一支）。
            None => lcall::UNKNOWN,
            Some(src) => {
                let (line, name) = (src.line, src.name);
                match take_lane(from) {
                    // 客户没把泊位交出来（或交不出来）。
                    None => lcall::DENIED,
                    Some(lane) => match lines.occupy(line, lane) {
                        Ok(()) => {
                            // **接线是登记的直接后果。**
                            plic.enable(line, LINE_PRIORITY);
                            // **排空那条路也在这里挂上**：客人往它写一句"我排空了"，本域就被叫醒
                            // ——那一格是**事件**，不是节拍（挂的是本端读的那一枚，见 `exhaust`）。
                            if let Some(lane) = lines.lane(line) {
                                let _ =
                                    pile.attach(&HolePie::from_token(lane.rx()), HoleDir::Pull);
                            }
                            // 名字只为日志：**当场从树里读**（装不下就打 `?`）。
                            debug!(
                                "router: line {line} = {}",
                                name.as_ref().map(|n| n.as_str()).unwrap_or("?")
                            );
                            lcall::OK
                        }
                        // **拒了就放回去**：这一趟刚交上来的那条路**不在账里**（`Lines::occupy`
                        // 收下了它、又拒了它），故放下它的是**这里**——`Endpoint` 是 `Copy` 的，
                        // 下面这一记 `close` 放的就是本端铸的那一枚（旧形状里这一手是本文件的
                        // `drop_lane`）。
                        //
                        // **照实记（旧 `drop_lane` 那笔账）**：从前不放会**每失败一次就在本域
                        // 表里多留两枚**（本端铸的那一枚 ＋ 从客人手里认下的那一枚），直到本域
                        // 退场——对一个会重试的客户就是无界增长；那一笔账如今由本文件这一记
                        // `close` 收：本端那一枚当场放下，客人那一枚（本域表里的副本）随**客人
                        // 自己**放下它那一枚时一起摘掉（派生边在 `gate::accord` 上）——客户那一
                        // 侧失败几趟就自己收几趟（`Line::occupy` 的 `pair.close()`）。
                        // **读数仍在下面这一行**：`pies=` 是本域表里此刻有几枚，少放一枚这一格
                        // 当场大 1（探针量过：临时关掉那几手，房客那一行从 `9` 涨到 `14`）。
                        //
                        // **拒了不再另递一句话**：客人从**回信孔**读到的那个非 `OK` 码就是这一
                        // 句（旧 `unseat` 那次过线通知的是一件已经说过的事）。
                        Err(fail) => {
                            lane.close();
                            debug!(
                                "router: lane dropped line={line} pies={}",
                                mail::table_size()
                            );
                            lcall::fail_to_code(Some(fail))
                        }
                    },
                }
            }
        };
        if let Some(back) = establish::find(from, lcall::BACK_MARK) {
            let _ = HolePie::from_token(back).push(&[code]);
            // **答完就放下**：这一枚是这一趟借过来的（一问一答一个往返），它不在本域的账里
            // ——账里根本没有它，此后没人会替它收。不放的话，每有一次登记就在本域表里多留
            // 一枚，直到本域退场；读数就带在 `pies=` 那一格上（见上面那一支）。
            let _ = mail::release(back);
        }
    }
}

/// 认下这位客户交出来的**线泊位**（记号 [`lcall::LANE`]），并把本端那一枚交给它。
///
/// 返**那条路的持有者**（[`Endpoint`]：本端读的那一枚 ＋ 认下来的写端）：`deliver` 往**它**推
/// 投递（客户读的那一枚），`exhaust` 收**它的**排空。
///
/// **一手就是"两头都装"**（[`establish::endpoint`]：铸本端那一枚交给它 ＋ 认下它那一枚，判据
/// 两格 = `owner == from` ＋ 记号 `lane`）。客户在推登记之前先铸先交，故这一步通常当场成
/// ——认不到就是它没交（或交不出来），那一趟不算（**本端刚铸的那一枚由调用方 `close`**，
/// 正是 [`serve`] 里那一支要说的那件事）。
fn take_lane(from: TaskId) -> Option<Endpoint> {
    let lane = establish::endpoint(from, Mark::of(lcall::LANE), Wait::AtMost(QUAY_MS)).ok()?;
    // **没有写端就投不出去**（原 `Quay::claim` 答不出来的那一格）：这条泊位不成立。
    lane.tx()?;
    Some(lane)
}
