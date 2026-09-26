#![no_std]
#![feature(allocator_api)]

//! 内核的**库面** —— 十个模块的所在，外加两个入口函数。
//!
//! **照实记（为什么有这一份，用户裁定"迁移到 embedded-test"）**：内核原先是**纯 bin**
//! （`src/main.rs` 既是 crate 根又是入口）。而 `embedded-test` 的用例住在**测试目标**
//! （`tests/embedded.rs`）里——那是**另一个 crate**，够不着 bin 的私有模块。
//! 故模块树搬到这里，bin 与测试目标各留自己那一份 `_start`：
//!
//! ```text
//!   src/lib.rs          十个模块 + init（起设施到 trap 就绪）+ main（整机启动，不返回）
//!   src/main.rs         `_start` 汇编 + `#[no_mangle] fn main` → `kernel::main`
//!   tests/embedded.rs   `_start` 汇编（多存一个 dtp）+ `#[embedded_test::tests] mod`
//! ```
//!
//! 公开面**只多出五样**：[`init`] / [`main`] / [`testing_mode`] / [`health`] / [`boot`]
//! （用例体住在后两层里，由测试目标叫）。其余八个模块仍是私有。

extern crate alloc;

/// 世界的装配与起跑（两个入口函数：装台 [`boot::init`] / 开演 [`boot::run`]）。
/// `pub` 的理由与 `health` 同——测试目标的用例体就是这两句。
pub mod boot;
mod console;
mod hart;
/// 内核自检用例的**身体**（八个）。`pub` 是因为测试目标是另一个 crate——
/// 见 `health/mod.rs` 的头注。
pub mod health;
mod layout;
mod lock;
mod memory;
mod platform;
mod runtime;
mod work;

use core::sync::atomic::{AtomicBool, Ordering};

use crate::memory::allocator;
use crate::runtime::chrono::clock;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::trap;
use crate::work::unit;

/// "现在在跑用例"那一格——panic 通道的读点，见 [`runtime::diagnose::halt`]。
///
/// **一个字的发布**：测试目标的 `#[init]` 调 [`testing_mode`] 置起，此后所有 panic 都按
/// "这一例失败"出口（semihosting abort → QEMU 退出码 → runner 判红）。
///
/// **照实记（它替掉了什么）**：原先这一问的答案是 `framework::running()`——`.tests` 段里
/// "当前在跑哪一例"的那个指针。框架删掉之后内核**不必**知道是哪一例（名字由 embedded-test
/// 经 semihosting 给），只要知道"在不在跑用例"，故收成一格布尔。
static TESTING: AtomicBool = AtomicBool::new(false);

/// 置"在跑用例"。由 `tests/embedded.rs` 的 `#[init]` 调用。
pub fn testing_mode() {
    TESTING.store(true, Ordering::Relaxed);
}

/// 现在在跑用例吗（[`runtime::diagnose::halt`] 的 panic 通道读它）。
pub(crate) fn testing() -> bool {
    TESTING.load(Ordering::Relaxed)
}

/// 起内核所需的**全部设施**——到 `trap` 就绪为止，**不起服务**。
///
/// 测试目标的 `#[init]` 用的就是这一半：用例要的正是调度器、分配器、页表、`Space` 原语，
/// 而**不要**一套起了服务、装了槽、等着收场的整机。
pub fn init(dtp: usize) {
    console::init();
    platform::machine::init(dtp);
    allocator::init().unwrap_or_else(|e| panic!("allocator init failed: {e}"));
    unit::init().unwrap_or_else(|e| panic!("unit init failed: {e}"));
    clock::init().unwrap_or_else(|e| panic!("clock init failed: {e}"));
    trace::init().unwrap_or_else(|e| panic!("trace init failed: {e}"));
    trap::init();
}

/// 整机启动：装台 [`boot::init`] 之后开演 [`boot::run`]，**不返回**（收场那一刀在
/// `conductor::halt`）。
pub fn main(_hartid: usize, dtp: usize) -> ! {
    init(dtp);
    boot::banner();
    boot::init();
    boot::run()
}
