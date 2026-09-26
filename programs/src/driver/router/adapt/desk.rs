//! router::adapt::desk — **门面（适配）**：登记那一句话——报**那一段区** ⇒ 解树（"线 = 区的函数"）
//! ⇒ 占住那一格 + 接上线 ⇒ 回一格状态码。
//!
//! 判定在 `contract::driver::line::core`（`Lines` 的四原语）与 `crate::core::sources`（区 → 线号）；
//! 本文件只做碰内核与设备的那几手：认泊位、解帧、接线、挂组、答话、清账外的那两枚。

use crate::core::sources::Sources;
use crate::plic::{LINE_PRIORITY, Plic};
use contract::message::Message;
use env::{HoleDir, Mark, Name, TaskId, Wait};
use protocol::debug;
use protocol::driver::line::{core::Lines, frame as lcall};
use protocol::session::call as scall;
use protocol::session::{Pier, Quay};
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
                    Some((mut quay, lane)) => match lines.occupy(line, lane) {
                        Ok(()) => {
                            // **接线是登记的直接后果。**
                            plic.enable(line, LINE_PRIORITY);
                            // **排空那条路也在这里挂上**：客人往它写一句"我排空了"，本域就被叫醒
                            // ——那一格是**事件**，不是节拍（挂的是本端读的那一枚，见 `exhaust`）。
                            if let Some(lane) = lines.lane(line) {
                                let _ =
                                    pile.attach(&HolePie::from_token(lane.hole()), HoleDir::Pull);
                            }
                            // 名字只为日志：**当场从树里读**（装不下就打 `?`）。
                            debug!(
                                "router: line {line} = {}",
                                name.as_ref().map(|n| n.as_str()).unwrap_or("?")
                            );
                            lcall::OK
                        }
                        // **拒了就放回去**：这一趟刚交上来的那条泊位不能留在账外（见 `drop_lane`）。
                        Err(fail) => {
                            drop_lane(&mut quay, lane, line);
                            lcall::fail_to_code(Some(fail))
                        }
                    },
                }
            }
        };
        if let Some(back) = scall::find(from, lcall::BACK_MARK) {
            let _ = HolePie::from_token(back).push(&[code]);
            // **答完就放下**：这一枚是这一趟借过来的（一问一答一个往返），它不在本域的账里
            // ——账里根本没有它，此后没人会替它收。不放的话，每有一次登记就在本域表里多留
            // 一枚，直到本域退场；读数就带在 `pies=` 那一格上（见 `drop_lane`）。
            let _ = mail::release(back);
        }
    }
}

/// 认下这位客户交出来的**线泊位**（记号 [`lcall::LANE`]），并把本端那一枚交给它。
///
/// 返那一格要记的泊位（**连码头一起**：收不下时要放回去，见 [`drop_lane`]）：`post` 往**它**推
/// 投递（客户读的那一枚），`pull` 收**它的**排空。
/// 客户在推登记之前先 `seat`（本端那一枚落在本域表里），故这一步通常当场成——认不到就是
/// 它没交（或交不出来）。
fn take_lane(from: TaskId) -> Option<(Quay, Pier)> {
    let mark = Name::new(lcall::LANE).ok()?;
    let mut quay = Quay::open(from, protocol::session::call::hands());
    quay.seat(mark).ok()?;
    quay.claim(from, Mark::of(lcall::LANE), Wait::AtMost(QUAY_MS))
        .ok()?;
    // **码头一起交出去**：`Lines` 收不下这条泊位时，得由拿着码头的人把它放回去
    // （只有码头知道那一枚是本端铸的，见 [`drop_lane`]）。
    let pier = quay.find(mark).copied()?;
    Some((quay, pier))
}

/// 拒绝那一趟的收尾：**把这一趟刚交上来的泊位放下**——本端铸的那一枚（`unseat`：顺手告诉
/// 对端"这条别用了"）+ 刚从它手里认下的那一枚。
///
/// **为什么非做不可**：`Lines::occupy` 拒了，那两枚就**不在任何账上**（账里根本没有这一格），
/// 故没有别人会替它收；`Quay` 也没有 `Drop`（放下一个 `Pier` 值只丢一个号，孔还在本域表里），
/// 于是每失败一次，本域表里就多两枚，直到本域退场。对一个会重试的客户，那就是无界增长。
///
/// **照实记（客户那一侧已经自己收干净了）**：从前客户把本端 `seat` 出去的那一枚与借出去的
/// 回信孔都留在自己表里（它没有 `claim`，接不到 `UNSEAT`），本域也收不了别人的表——那一枚随它
/// 退场清掉。今天 [`Line::occupy`](protocol::driver::line::client::Line::occupy) 自己收（失败
/// 那几趟 `Quay::shut` + 放下回信孔），读数在房客那一行 **`lodger: pies=`** 上（探针量过：
/// 临时关掉那几手，同一处从 `9` 涨到 `14`）。
///
/// 读数带一格 **`pies=`**（本域表里现在有几枚）：'放了没有'这件事因此**可量**——少放一枚，
/// 这一格当场大 1（判据钉在 `crates/gate/src/soak.rs`（已删）里，涨了就是红）。**答完话那一枚回信孔副本**
/// 也走同一条纪律（见 [`serve`] 尾上那一手）。
fn drop_lane(quay: &mut Quay, lane: Pier, line: u32) {
    if let Some(at_peer) = lane.at_peer() {
        let _ = mail::release(at_peer);
    }
    if let Ok(mark) = Name::new(lcall::LANE) {
        quay.unseat(mark);
    }
    debug!(
        "router: lane dropped line={line} pies={}",
        mail::table_size()
    );
}
