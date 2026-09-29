//! rtc::client — **客侧两手**：问一声现在几点、约一个时刻（约成之后从它等那一声）。
//!
//! 客人不碰设备——那台时钟归驱动持有（`ONLY`）；客人只说两句话、收两句话。
//!
//! ```text
//!   now(门牌, ms)        一问一答，自带一枚回信孔，答完就放掉
//!   arm(门牌, after, ms) 约；**那一枚回信孔留下来**——驱动把它收在那一格里，
//!                        到点从那枚孔把"那一声"推回来
//! ```
//!
//! **借孔那一趟的次序是契约的一半**：先铸、先交（`port::ship`），**再**推帧。收的那一侧按
//! "谁给的 + 记号"两格认，多枚时取**最后那一枚**——故最后那一枚一定就是这一趟那一枚。
//!
//! **问走门、答走发送端**：问那一侧推的是那扇**门**（`HolePie::from_token(..).push(..)`，同 `principal`
//! 的客侧），答那一侧是本端自己那枚孔——**上端点的发送端**（`Sender::<Time>` / `Sender::<Status>`：答的
//! 两形各是一张实现了报文约定的表，见 [`super::core::frame`]）。
//!
//! [`Alarm`] 是**约成了才有的东西**：`receive` 只长在它上面，"没约就等"因此写不出来。

use env::PieToken;
use env::{HoleDir, Wait};
use protocol::communication::establish;
use protocol::communication::receiver::Receiver;
use protocol::message::Message;
use runtime::env::mail::{self, HolePie};

use super::core::Fail;
use super::core::frame::{self, Arm, Now, Status, Time};

/// 问一声现在几点：返**驱动读设备那一刻**的纳秒计数。
///
/// 一问一答——这一趟的回信孔只活到这句话答完（同一次往返借一枚，见 [`protocol::communication`]
/// 事实 2：孔是单槽，一个槽只有一个读者，"我推了再读"读到的是自己推的那一句）。
pub fn now(entry: PieToken, millis: Wait) -> Result<u64, Fail> {
    // **借一枚回信孔**（铸 ＋ 交，记号 = 本面自己的 `BACK`）：返 `(本端那一枚, 驱动表里那一枚)`
    // ——后者写进帧，收的人一次 `reserve` 就用，不必扫全表。
    let (back, seed) = establish::lend_out(entry, frame::BACK).map_err(|()| Fail::Denied)?;
    // 编一问：**表上那一手**（定长缓冲，故它不可能失败；`back` 是运输那一格，随动作一起进帧）。
    let mut frame = [0u8; Now::LEN];
    let Some(n) = Now::of(seed).store_at(&mut frame, 0) else {
        // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
        // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        return Err(Fail::Denied);
    };
    // **两半都写出来**（旧 `push` 是合一的）：等轮到自己（单槽门面上撞车是常事）＋ 等这只手
    // 被取走（`frame` 是这一帧的栈，不等它下线就返回，驱动可能复制到一段死栈）。
    let door = HolePie::from_token(entry);
    if door.push(&frame[..n], Wait::Forever).is_err()
        || !matches!(door.wait(HoleDir::Push, Wait::Forever), Ok(true))
    {
        // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
        // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        return Err(Fail::Denied);
    }
    // 收：答话走**这一趟借出去的那一枚孔**（`Receiver::recv`；缓冲由调用方给——这一形 8 字节）。
    // 两格失败（没收到 / 解不动）在这一侧落同一格：`Denied`（对本端是同一个下一步）。
    let mut buf = Time::EMPTY;
    let answer = Receiver::<Time>::from_token(back)
        .recv(buf.as_mut(), millis)
        .map_err(|_| Fail::Denied);
    // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
    // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
    let _ = mail::seal(back);
    let _ = mail::release(back);
    answer
}

/// 约一段**时间**：`after_ns`（相对纳秒，"再过多 long"）。成 ⇒ 返那一次约；到点从那枚孔收那一声。
///
/// **照实记（从"时刻"改成"时长"）**：绝对时刻那版要客侧自己补一个送达延迟的猜（见
/// `frame::Wire::Arm` 那一格的照实记）；相对量由收帧的驱动算，客侧不必知道路有多长。
///
/// 失败域两格都由**驱动说的话**给出（`Taken` / `Past`），第三格 `Denied` 是这一趟自己没
/// 走到——三种情况对客人是三个不同的下一步，故不合并成一格。
pub fn arm(entry: PieToken, after_ns: u64, millis: Wait) -> Result<Alarm, Fail> {
    // **借一枚回信孔**（铸 ＋ 交，记号 = 本面自己的 `BACK`）：返 `(本端那一枚, 驱动表里那一枚)`
    // ——后者写进帧，收的人一次 `reserve` 就用，不必扫全表。
    let (back, seed) = establish::lend_out(entry, frame::BACK).map_err(|()| Fail::Denied)?;
    let mut frame = [0u8; Arm::LEN];
    let Some(n) = Arm::of(seed, after_ns).store_at(&mut frame, 0) else {
        // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
        // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        return Err(Fail::Denied);
    };
    let door = HolePie::from_token(entry);
    if door.push(&frame[..n], Wait::Forever).is_err()
        || !matches!(door.wait(HoleDir::Push, Wait::Forever), Ok(true))
    {
        // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
        // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        return Err(Fail::Denied);
    }
    // 收那一格答码（**恰好 1 字节**：长短都不是这一形 ⇒ 读不懂 ⇒ `Denied`）。
    let mut one = Status::EMPTY;
    let code = match Receiver::<Status>::from_token(back).recv(one.as_mut(), millis) {
        Ok(code) => code,
        Err(_) => {
            // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
            // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
            let _ = mail::seal(back);
            let _ = mail::release(back);
            return Err(Fail::Denied);
        }
    };
    if code == frame::OK {
        // **这一枚不还**：那一格现在收着它，到点从那枚孔回来。
        return Ok(Alarm {
            back: HolePie::from_token(back),
        });
    }
    // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
    // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
    let _ = mail::seal(back);
    let _ = mail::release(back);
    Err(frame::code_to_fail(code).unwrap_or(Fail::Denied))
}

/// 一次**约**：那一格里收着的，就是它。
pub struct Alarm {
    back: HolePie,
}

impl Alarm {
    /// 等那一声。返**响的那一刻**（驱动听见闹钟时读到的计数）。
    ///
    /// **无界等**：客人只有这一件事，而对面一没，这一枚孔就封印 ⇒ 当场答 `Err(())`，
    /// 不是永久挂住（寿命边随它的**开者**——这一枚是客人自己铸的）。
    ///
    /// **照实记（这一格从裸 `pull` 换成 `Receiver::recv` 的"永久"那一档）**：判据一字不改（没到 ⇒ 等、
    /// 孔封印 ⇒ 当场错），变的是内核那一侧的等法——`Wait::Forever` 在 `pull` 里落成一个
    /// **到不了的点**（`u64::MAX`），于是这一等每 ~100 ms 被叫醒一次、自己复探（理由与实测见
    /// `HolePie::wait`）。
    pub fn receive(&self) -> Result<u64, ()> {
        let mut buf = Time::EMPTY;
        Receiver::<Time>::from_token(self.back.token())
            .recv(buf.as_mut(), Wait::Forever)
            .map_err(|_| ())
    }
}

// **照实记（本文件那两层转发退场）**：这里从前有 `fn lend_out(entry)` 与
// `fn push(entry, frame)` 两个**一行正文**的转发（各自只做 `map_err(|()| Fail::Denied)`）。
// "借一枚回信孔"只有一具身体（[`establish::lend_out`]），而本面自己那一格记号是
// [`frame::BACK`]（**调用点上写出来**，比藏在一层函数里更看得见）⇒ 两层壳一起删。
