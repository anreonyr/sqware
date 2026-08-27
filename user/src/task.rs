//! 用户 task 模块：`spawn`/`closure`/`Join`。

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use crate::env;
use ubi::UResult;

/// 完成槽 — spawn/join 共享的接缝（新任务单写、join 单读，done 作完成栅栏）。
pub struct Completion<T> {
    done: AtomicBool,
    result: Option<T>,
}

impl<T> Completion<T> {
    const fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            result: None,
        }
    }
}

/// 完成槽的 Send 包装：槽受 done-atomic 协议保护（新任务 release 置位、join acquire
/// 取读，result 单写单取），跨线程共享安全；raw 指针默认 !Send，故显式标记。
///
/// 访问一律经方法走整个 `SendSlot`（避开闭合的 disjoint field capture，让 Send 生效）。
#[derive(Clone, Copy)]
struct SendSlot<T>(*mut Completion<T>);

// SAFETY: 槽的跨线程访问全部经 done 完成栅栏，无并发写读同一字段。
unsafe impl<T> Send for SendSlot<T> {}
// SAFETY: 同上——槽不从多个线程同时写 result，仅 join 单读。
unsafe impl<T> Sync for SendSlot<T> {}

impl<T> SendSlot<T> {
    /// 新任务侧：存结果并置完成（done 释放写，join 侧 acquire 读）。
    unsafe fn store_result(self, r: T) {
        // SAFETY: 槽于当前空间堆上、由 `closure` 持有至 join 回收；新任务单写。
        unsafe {
            (*self.0).result = Some(r);
            (*self.0).done.store(true, Ordering::Release);
        }
    }
}

/// 任务句柄：携带完成槽，`join` 等任务跑完并取回 `T`。
pub struct Join<T> {
    slot: *mut Completion<T>,
}

impl<T> Join<T> {
    /// 等任务跑完，取回结果 `T`。
    pub fn join(self) -> T {
        let slot = self.slot;
        loop {
            // SAFETY: 槽由本 Join 独占；done 未置位前不读 result。
            if unsafe { (*slot).done.load(Ordering::Acquire) } {
                // SAFETY: done 置位 ⇒ result 已写入；单写单取。
                let r = unsafe { (*slot).result.take() }.expect("joined task lost result");
                // SAFETY: 槽 boxed 于用户堆，现由本 Join 独占回收。
                unsafe { drop(Box::from_raw(slot)) };
                return r;
            }
            let _ = env::sleep(Duration::from_millis(1));
        }
    }
}

/// 建一 U 任务：`entry` = 用户入口 VA，`arg` = 闭包指针；返回任务句柄。
pub fn spawn(entry: usize, arg: usize) -> UResult<usize> {
    env::spawn(entry, arg)
}

/// 带闭包任务：新线程跑 `f`，join 取回 `T`。
pub fn closure<F, T>(f: F) -> Join<T>
where
    F: FnOnce() -> T + Send + 'static,
{
    // 完成槽：spawn 侧持有、新任务写、join 取。
    let slot = Box::into_raw(Box::new(Completion::new()));
    let send_slot = SendSlot(slot);
    // 内部闭包（无类型 trampoline 可调）：调 f → 存结果 → 置完成。
    // send_slot 整个传给方法（whole-struct 捕获），使 SendSlot 的 Send 生效。
    let inner: Box<dyn FnOnce() + Send> = Box::new(move || {
        let r = f();
        // SAFETY: store_result 内部访问槽；done 栅栏保证 join 侧安全。
        unsafe { send_slot.store_result(r) }
    });
    // 双装箱瘦指针：a0 传该薄指针。
    let holder: Box<Box<dyn FnOnce() + Send>> = Box::new(inner);
    let ptr = Box::into_raw(holder) as usize;
    let _task = spawn((uktask_trampoline as extern "C" fn(usize) -> !) as usize, ptr)
        .expect("task spawn failed");
    Join { slot }
}

/// 新线程共享入口（U 态）：a0 = 双装箱闭包薄指针；调闭包 → 退出。
#[unsafe(no_mangle)]
pub extern "C" fn uktask_trampoline(arg: usize) -> ! {
    // SAFETY: arg 由 `closure` 传入的双装箱瘦指针；本线程独占回收。
    let holder: Box<Box<dyn FnOnce() + Send>> =
        unsafe { Box::from_raw(arg as *mut Box<dyn FnOnce() + Send>) };
    holder(); // Box<Box<dyn FnOnce>>: FnOnce —— 调 inner 闭包（内部已存结果+置完成）
    env::exit()
}
