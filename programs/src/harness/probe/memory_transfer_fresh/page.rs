//! 本景握手页那一面：控制单元与读者读写同一页的那几个字。
//!
//! 页的线格式在 [`handshake`](super::handshake)（内核夹具 include 同一份文件）；本文件只把它
//! 包成“一次请求 / 一枚读数”这两个动作。程序与内核夹具对同一页中的对齐字均作
//! Release store / Acquire load；请求以最后写入的 MODE 发布，不假设任务始终在同一 hart。

use core::sync::atomic::{AtomicU64, Ordering};

use super::handshake;

/// 本景起手：写标记、拉平请求、状态与读数（那时还没有第二台）。
pub fn reset() {
    publish(handshake::off::TAG, handshake::TAG);
    publish(handshake::off::START, 0);
    publish(handshake::off::VIEW, 0);
    publish(handshake::off::READY, 0);
    publish(handshake::off::STATE, handshake::state::IDLE);
    publish(handshake::off::VERDICT, handshake::VERDICT_NONE);
    publish(handshake::off::MODE, handshake::mode::OFF);
}

/// 下一次移交请内核夹具做哪一件事（目标那一页的关系身份一起交过去）。
pub fn request(mode: u64, view: usize) {
    publish(handshake::off::VERDICT, handshake::VERDICT_NONE);
    publish(handshake::off::VIEW, view as u64);
    publish(handshake::off::STATE, handshake::state::PREPARED);
    publish(handshake::off::MODE, mode);
}

/// 这一景收工：内核夹具不再插手之后的任何一次移交。
pub fn retire() {
    publish(handshake::off::MODE, handshake::mode::OFF);
}

/// 内核夹具写回来的那一步。
pub fn state() -> u64 {
    observe(handshake::off::STATE)
}

/// 读者报“我已经在自旋了”。
pub fn mark_ready() {
    publish(handshake::off::READY, 1);
}

/// 读者是不是已经在自旋了。
pub fn ready() -> u64 {
    observe(handshake::off::READY)
}

/// 读者等的那一枚字：内核在占槽之后把它放开。
///
/// **这是本景唯一的跨 hart 放行**：它用 `Acquire`，故读者看到 `RELEASE` 之后那次
/// 对目标页的访存不在它之前。
pub fn start() -> u64 {
    observe(handshake::off::START)
}

/// 读者把自己的读数写回来（控制者按 `Acquire` 读，它后面的收尾排在这次发布之后）。
pub fn publish_verdict(value: u64) {
    publish(handshake::off::VERDICT, value);
}

/// 读者写回来的那一枚读数（还没报就是 [`handshake::VERDICT_NONE`]）。
pub fn verdict() -> u64 {
    observe(handshake::off::VERDICT)
}

/// 内核夹具那一步的名字（只入控制台，不进判定）。
pub fn state_name() -> &'static str {
    match state() {
        handshake::state::IDLE => "idle",
        handshake::state::PREPARED => "prepared",
        handshake::state::ARMED => "armed",
        handshake::state::TIMEOUT => "timeout",
        _ => "?",
    }
}

/// 这一页上第 `at` 个字节处那一格。
fn cell(at: usize) -> &'static AtomicU64 {
    // SAFETY: 握手页在本空间的映射里、按页对齐，故 `VA + at` 是指向有效 `u64` 的
    // 8 字节对齐地址（`at` 取自 [`handshake::off`]）；它括到本域走完，且两个方向
    // 都只按同一份线格式的那几个字寻址。
    unsafe { &*((handshake::VA + at) as *const AtomicU64) }
}

/// 发布一格：另一颗 hart 看到它时，本次发布之前的写都已经能看见。
fn publish(at: usize, value: u64) {
    cell(at).store(value, Ordering::Release);
}

/// 读一格：读到别人发布的那个值之后，本次读之前的那些写都已经能看见。
fn observe(at: usize) -> u64 {
    cell(at).load(Ordering::Acquire)
}
