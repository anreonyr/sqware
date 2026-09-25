//! rtc::client — **客侧两手**：问一声现在几点、约一个时刻（约成之后从它等那一声）。
//!
//! 客人不碰设备——那台时钟归驱动持有（`ONLY`）；客人只说两句话、收两句话。
//!
//! ```text
//!   now(门牌, ms)        一问一答，自带一枚回信孔，答完就放掉
//!   arm(门牌, at, ms)    约；**那一枚回信孔留下来**——驱动把它收在那一格里，
//!                        到点从那枚孔把"那一声"推回来
//! ```
//!
//! **借孔那一趟的次序是契约的一半**：先铸、先交（`port::ship`），**再**推帧。收的那一侧按
//! "谁给的 + 记号"两格认，多枚时取**最后那一枚**——故最后那一枚一定就是这一趟那一枚。
//!
//! [`Alarm`] 是**约成了才有的东西**：`receive` 只长在它上面，"没约就等"因此写不出来。

use env::PieToken;
use runtime::env::mail::{self, HolePie};

use super::call;
use super::core::Fail;

/// 问一声现在几点：返**驱动读设备那一刻**的纳秒计数。
///
/// 一问一答——这一趟的回信孔只活到这句话答完（同一次往返借一枚，见 `protocol::session`
/// 事实 2：孔是单槽，一个槽只有一个读者，"我推了再读"读到的是自己推的那一句）。
pub fn now(entry: PieToken, millis: usize) -> Result<u64, Fail> {
    let (back, seed) = lend_out(entry)?;
    if push(entry, &call::pack_ask(seed)).is_err() {
        let _ = mail::release(back);
        return Err(Fail::Denied);
    }
    let mut buf = [0u8; call::TIME_LEN];
    let answer = HolePie::from_token(back)
        .pull_timeout(&mut buf, millis)
        .ok()
        .and_then(|n| call::unpack_time(&buf[..n]))
        .ok_or(Fail::Denied);
    let _ = mail::release(back);
    answer
}

/// 约一个时刻：`at`（绝对纳秒）。成 ⇒ 返那一次约；到点从那枚孔收那一声。
///
/// 失败域两格都由**驱动说的话**给出（`Taken` / `Past`），第三格 `Denied` 是这一趟自己没
/// 走到——三种情况对客人是三个不同的下一步，故不合并成一格。
pub fn arm(entry: PieToken, at: u64, millis: usize) -> Result<Alarm, Fail> {
    let (back, seed) = lend_out(entry)?;
    if push(entry, &call::pack_arm(seed, at)).is_err() {
        let _ = mail::release(back);
        return Err(Fail::Denied);
    }
    let mut one = [0u8; call::CODE_LEN];
    let code = match HolePie::from_token(back).pull_timeout(&mut one, millis) {
        Ok(n) if n == call::CODE_LEN => one[0],
        _ => {
            let _ = mail::release(back);
            return Err(Fail::Denied);
        }
    };
    if code == call::OK {
        // **这一枚不还**：那一格现在收着它，到点从那枚孔回来。
        return Ok(Alarm {
            back: HolePie::from_token(back),
        });
    }
    let _ = mail::release(back);
    Err(call::code_to_fail(code).unwrap_or(Fail::Denied))
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
    pub fn receive(&self) -> Result<u64, ()> {
        let mut buf = [0u8; call::TIME_LEN];
        let n = self.back.pull(&mut buf).map_err(|_| ())?;
        call::unpack_time(&buf[..n]).ok_or(())
    }
}

/// 借一枚回信孔过去、把这一帧推上那扇门：返**本端那一枚**（答话与那一声都从它回来）。
///
/// 借一枚回信孔（铸 ＋ 交）：返 `(本端那一枚, **在驱动表里那一枚**)`——后者要写进帧
/// （用户裁定甲′：收方拿它一次 `reserve` 就用，不必扫全表）。
///
/// 身体住在 [`protocol::session::call::lend_out`]（那一手与 [`protocol::session::call::lend`]
/// 只差"推不推"这一步），这里只留本面自己的记号。
fn lend_out(entry: PieToken) -> Result<(PieToken, PieToken), Fail> {
    protocol::session::call::lend_out(entry, call::BACK).map_err(|()| Fail::Denied)
}

/// 把一帧推上那扇门（`lend_out` 的后半）。
fn push(entry: PieToken, frame: &[u8]) -> Result<(), Fail> {
    protocol::session::call::push_to(entry, frame).map_err(|()| Fail::Denied)
}
