//! 问一声现在几点、约一个时刻（约成之后从它等那一声）。
//! 客人不碰设备——那台时钟归驱动持有（`ONLY`）；客人只说两句话、收两句话。
//! **问走门、答走发送端**：问那一侧推的是那扇**门**（`Hole::from_raw(..).push(..)`，同 Identity
//! 的客侧），答那一侧是本端自己那枚孔——**上端点的发送端**（`Sender::<Time>` / `Sender::<Status>`：答的
//! 两形各是一张实现了报文约定的表，见 super::core::frame）。
//! Alarm 是**约成了才有的东西**：`receive` 只长在它上面，"没约就等"因此写不出来。

use env::{PieToken, HoleDir, Wait};
use ipc::hand::Receiver;
use ipc::session::establish;
use protocol::debug;
use protocol::wire::message::Message;

use super::core::Fail;
use super::core::frame::{self, Arm, Now, Status, Time};
use env::wire::Span as _;
use env::pie;
use ::resource::raw::{Hole};

/// 问一声现在几点：返**驱动读设备那一刻**的纳秒计数
/// 事实 2：孔是单槽，一个槽只有一个读者，"我推了再读"读到的是自己推的那一句）
pub fn now(entry: PieToken, millis: Wait) -> Result<u64, Fail> {
    // **借一枚回信孔**（铸 ＋ 交，记号 = 本面自己的 `BACK`）：返 `(本端那一枚, 驱动表里那一枚)`
    // ——后者写进帧，收的人一次 `reserve` 就用，不必扫全表。
    let (back, seed) = establish::lend_out(entry, frame::BACK).map_err(|()| Fail::Denied)?;
    // 编一问：**表上那一手**（定长缓冲，故它不可能失败；`back` 是运输那一格，随动作一起进帧）。
    let mut frame = [0u8; Now::LEN];
    let Some(n) = Now::of(seed).store_at(&mut frame, 0) else {
        // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
        let _ = pie::seal(back);
        let _ = pie::release(back);
        return Err(Fail::Denied);
    };
    let door = Hole::from_raw(entry);
    if door.push(&frame[..n], Wait::Forever).is_err()
        || !matches!(door.wait(HoleDir::Push, Wait::Forever), Ok(true))
    {
        // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
        let _ = pie::seal(back);
        let _ = pie::release(back);
        return Err(Fail::Denied);
    }
    // 两格失败（没收到 / 解不动）在这一侧落同一格：`Denied`（对本端是同一个下一步）。
    let mut buf = Time::EMPTY;
    let answer = Receiver::<Time>::from_raw(back)
        .recv(buf.as_mut(), millis)
        .map_err(|_| Fail::Denied);
    // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
    let _ = pie::seal(back);
    let _ = pie::release(back);
    answer
}

/// 约一段**时间**：`after_ns`（相对纳秒，"再过多 long"）。成 ⇒ 返那一次约；到点从那枚孔收那一声
pub fn arm(entry: PieToken, after_ns: u64, millis: Wait) -> Result<Alarm, Fail> {
    // **借一枚回信孔**（铸 ＋ 交，记号 = 本面自己的 `BACK`）：返 `(本端那一枚, 驱动表里那一枚)`
    // ——后者写进帧，收的人一次 `reserve` 就用，不必扫全表。
    let (back, seed) = match establish::lend_out(entry, frame::BACK) {
        Ok(pair) => pair,
        Err(()) => {
            why("lend", 0, "");
            return Err(Fail::Denied);
        }
    };
    let mut frame = [0u8; Arm::LEN];
    let Some(n) = Arm::of(seed, after_ns).store_at(&mut frame, 0) else {
        why("store", 0, "");
        // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
        let _ = pie::seal(back);
        let _ = pie::release(back);
        return Err(Fail::Denied);
    };
    let door = Hole::from_raw(entry);
    if door.push(&frame[..n], Wait::Forever).is_err() {
        why("push", 0, "");
        // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
        let _ = pie::seal(back);
        let _ = pie::release(back);
        return Err(Fail::Denied);
    }
    if !matches!(door.wait(HoleDir::Push, Wait::Forever), Ok(true)) {
        why("take", 0, "");
        // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
        let _ = pie::seal(back);
        let _ = pie::release(back);
        return Err(Fail::Denied);
    }
    // 收那一格答码（**恰好 1 字节**：长短都不是这一形 ⇒ 读不懂 ⇒ `Denied`）。
    let mut one = Status::EMPTY;
    let t_recv = env::chrono::clock();
    let code = match Receiver::<Status>::from_raw(back).recv(one.as_mut(), millis) {
        Ok(code) => code,
        Err(e) => {
            let ms = (env::chrono::clock().saturating_sub(t_recv) / 1_000_000) as usize;
            // **（临时读数）"没等到"与"读不懂"在这里分开**（`RecvFail` 两格），再把这一趟花了
            // 多少毫秒带上——它是"预算到期"与"答话不成形"的唯一分界。前 20 次。
            match e {
                ipc::hand::RecvFail::Mail(f) => {
                    why("recv-mail", ms, &alloc::format!("fail={f:?}"))
                }
                ipc::hand::RecvFail::Unread(len) => {
                    why("recv-unread", ms, &alloc::format!("len={len}"))
                }
            }
            // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
            let _ = pie::seal(back);
            let _ = pie::release(back);
            return Err(Fail::Denied);
        }
    };
    if code == frame::OK {
        // **这一枚不还**：那一格现在收着它，到点从那枚孔回来。
        return Ok(Alarm {
            back: Hole::from_raw(back),
        });
    }
    // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
    let _ = pie::seal(back);
    let _ = pie::release(back);
    why("code", 0, &alloc::format!("got={code}"));
    Err(frame::code_to_fail(code).unwrap_or(Fail::Denied))
}

/// **（临时读数）"二次 arm 折成 `Denied`"的四种子因分开**：`Denied` 把"孔借不出去／帧推不动／
/// 等到期／答话读不懂"折成同一个码，而它们下一步完全不同（见 `core/fail.rs` 那一节）。
fn why(what: &str, ms: usize, extra: &str) {
    static N: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
    if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
        debug::put(&alloc::format!("rtc: ask fail at={what} ms={ms} {extra}"));
    }
}

/// 一次**约**：那一格里收着的，就是它
pub struct Alarm {
    back: Hole,
}

impl Alarm {
    /// 等那一声。返**响的那一刻**（驱动听见闹钟时读到的计数）
    /// **无界等**：客人只有这一件事，而对面一没，这一枚孔就封印 ⇒ 当场答 `Err(())`
    /// 不是永久挂住（寿命边随它的**开者**——这一枚是客人自己铸的）
    pub fn receive(&self) -> Result<u64, ()> {
        let mut buf = Time::EMPTY;
        Receiver::<Time>::from_raw(self.back.token())
            .recv(buf.as_mut(), Wait::Forever)
            .map_err(|_| ())
    }
}
