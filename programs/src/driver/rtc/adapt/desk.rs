//! rtc::adapt::desk — **门面（适配）**：解帧 → 认孔 → 喂会话核 → 执行它吐的答形。
//! 判定在 [`Host::ask`]（纯，见 `core/host.rs`）；本文件只做碰内核与设备的那几手：
//! `mail::reserve` 认那枚回信孔、从设备读这一刻的钟、走 `Sender` 发答、放下那一枚、武装设备。

use crate::dev::rtc;
use env::{PieToken, TaskId};
use programs::driver::rtc::core::frame::{self, Status, Time};
use programs::driver::rtc::core::host::{Answer, Host};
use protocol::communication::sender::Sender;
use protocol::debug;
use runtime::core::res::dock::View;
use runtime::env::mail;

/// 门上那一句话：**解帧 → 认孔 → 喂核 → 从这一趟自带的那枚孔答回去**。
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**：那一格是"客人
/// 交进来的那一枚**在我表里**是几号"，而"是谁给的、刻的什么"仍要当场读出来核对——否则客人
/// 能让本域往**别人的孔**里写。`reserve` 一次代替全表扫。
/// **拒了的那一趟也要收尾**：那一枚孔不在任何账上（那一格根本没占上），此后没人会替它收
/// ⇒ 答完当场放下。
pub fn serve(host: &mut Host, view: View, from: TaskId, frame: &[u8]) {
    let Some((back, ask)) = frame::Wire::take(frame) else {
        // 不是那个形状：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    // **一次 `reserve`，代替一次全表扫**：判据与旧那一扫**逐字同一条**（谁给的 ＋ 记号），
    if !matches!(
        mail::reserve(back),
        Ok((_vestor, owner, mark)) if owner == from && mark == frame::BACK
    ) {
        debug!("rtc: no back hole from {}", from.get());
        return;
    }
    let now = rtc::now(view);
    match host.ask(ask, back, now) {
        // 一问一答：答话**走 `Sender`**，这一枚孔这一趟就用完了（一问一答一个往返）。
        Answer::Time(now) => {
            ship_time(back, now);
            let _ = mail::release(back);
            debug!("rtc: asked now={now}");
        }
        // **设备那一手紧随原语之后**（账记下了，硬件跟上）——与线那一层
        // "接线是登记的直接后果"同一条分工。
        Answer::Armed { at } => {
            rtc::arm(view, at);
            debug!(
                "rtc: armed at={at} ier={} alarm={}",
                rtc::irq_enabled(view),
                rtc::armed(view)
            );
            // 答码**先于**那一声：那一格已经占上，而设备要过一会儿才拉线。
            // **这一枚不还**：那一格现在收着它，到点从那枚孔回来。
            ship_code(back, frame::OK);
        }
        // 拒了：答一格码 + 放下这一枚，并留一行读数。
        Answer::Refused { code, at } => {
            ship_code(back, code);
            let _ = mail::release(back);
            debug!(
                "rtc: refused={code} at={at} now={now} late_ns={}",
                now.saturating_sub(at)
            );
        }
    }
}

/// 把那一声答出去（一个时刻）。
fn ship_time(back: PieToken, now: u64) {
    // **写端跟着这一趟走**：落出作用域时等这只手被取走（`Drop`）——那位客人不来取，卡的是
    // 他自己那一趟。
    let mut tx = Sender::<Time>::from_token(back);
    let _ = tx.send(Time::of(now));
}

/// 把那一格码答出去。
fn ship_code(back: PieToken, code: u8) {
    let mut tx = Sender::<Status>::from_token(back);
    let _ = tx.send(Status::of(code));
}
