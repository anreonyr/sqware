//! rtc::adapt::desk — **门面（适配）**：解帧 → 认孔 → 喂会话核 → 执行它吐的答形。
//!
//! 判定在 [`Host::ask`]（纯，见 `core/host.rs`）；本文件只做碰内核与设备的那几手：
//! `mail::reserve` 认那枚回信孔、从设备读这一刻的钟、上船台发答、放下那一枚、武装设备。

use crate::rtc;
use crate::say;
use alloc::format;
use env::{PieToken, TaskId};
use programs::driver::rtc::core::frame::{self, Status, Time};
use programs::driver::rtc::core::host::{Answer, Host};
use protocol::session::slip::Slip;
use runtime::core::dock::View;
use runtime::env::mail;

/// 门上那一句话：**解帧 → 认孔 → 喂核 → 从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**（用户裁定甲′）：那一格是"客人
/// 交进来的那一枚**在我表里**是几号"，而"是谁给的、刻的什么"仍要当场读出来核对——否则客人
/// 能让本域往**别人的孔**里写。旧写法是扫全表按 `(谁给的, 记号)` 找（每帧 ~6.5 ms，表 16 枚
/// 时 O(n²)，读数量在 `crates/gate`（已删）的 soak 那一门）。
///
/// **拒了的那一趟也要收尾**：那一枚孔不在任何账上（那一格根本没占上），此后没人会替它收
/// ⇒ 答完当场放下。这与线那一刀 `drop_lane` 是同一条纪律、同一个理由。
pub fn serve(host: &mut Host, view: View, from: TaskId, frame: &[u8]) {
    let Some((back, ask)) = frame::Wire::take(frame) else {
        // 不是那个形状：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    // **一次 `reserve`，代替一次全表扫**：判据与旧那一扫**逐字同一条**（谁给的 ＋ 记号），
    // 只是从"扫遍全表找 match"变成"验这一格 match"。
    //
    // **但它不再无声**（照实记）：这一格从前直接 `return`，于是"我把它丢了"与"客人根本
    // 没推上来"在读数里**长得一模一样**（两边都是客人超时）——`harness/sleeper` 那张脸
    // 在真机上查了很久才缩到这一步。丢一趟留一行，谁丢的、丢给谁。
    if !matches!(
        mail::reserve(back),
        Ok((_vestor, owner, mark)) if owner == from && mark == frame::BACK
    ) {
        say(&format!("rtc: no back hole from {}", from.get()));
        return;
    }
    let now = rtc::now(view);
    match host.ask(ask, back, now) {
        // 一问一答：答话**上船台**，这一枚孔这一趟就用完了（一问一答一个往返）。
        Answer::Time(now) => {
            ship_time(back, now);
            let _ = mail::release(back);
            say(&format!("rtc: asked now={now}"));
        }
        // **设备那一手紧随原语之后**（账记下了，硬件跟上）——与线那一层
        // "接线是登记的直接后果"同一条分工。
        Answer::Armed { at } => {
            rtc::arm(view, at);
            say(&format!(
                "rtc: armed at={at} ier={} alarm={}",
                rtc::irq_enabled(view),
                rtc::armed(view)
            ));
            // 答码**先于**那一声：那一格已经占上，而设备要过一会儿才拉线。
            // **这一枚不还**：那一格现在收着它，到点从那枚孔回来。
            ship_code(back, frame::OK);
        }
        // 拒了：答一格码 + 放下这一枚，并留一行读数。
        Answer::Refused { code, at } => {
            ship_code(back, code);
            let _ = mail::release(back);
            // **照实记（这一行为什么在，以及为什么排在这里）**：拒绝路从前一个字都不
            // 打，于是"sleeper 那台偶尔少一台"只剩客人侧一句 `alarm err=2`——**迟到
            // 多少**量不出来。这一行把那格交出来：`late_ns` = 我拿自己的钟比对时 `at`
            // 已经过去了多久（`Past` 那一支 `at <= now`，故它 ≥ 0；`Taken` 那一支
            // `at` 还在前头，按 0 记）。形状声明在 `crates/gate/src/soak.rs`（已删）的读数表里。
            //
            // **照实记（它为什么在发答话之后）**：第一版排在那一手之前（那时是裸
            // `push`，今天是船台的 `ship`），而 `say` 是**同步 UART**（一行 ~1 ms）——
            // 量的人自己站进了被测的那条路上，把客人等答话的时间撑长了。故答话先走、
            // 读数后打：这一行不许改变它要量的东西。
            say(&format!(
                "rtc: refused={code} at={at} now={now} late_ns={}",
                now.saturating_sub(at)
            ));
        }
    }
}

/// 把那一声答出去（一个时刻）。
fn ship_time(back: PieToken, now: u64) {
    // `.ok()`：装不上那一格按构造到不了（`Buf` 由本族 `Message` 自己给，见 `Slip::load` 的
    // 照实记）；真到了那里，那一层是 `None`，与"推不出去"同一行读数。
    let _ = Slip::<Time>::seal(back)
        .load(Time::of(now))
        .ok()
        .map(|s| s.ship());
}

/// 把那一格码答出去。
fn ship_code(back: PieToken, code: u8) {
    let _ = Slip::<Status>::seal(back)
        .load(Status::of(code))
        .ok()
        .map(|s| s.ship());
}
