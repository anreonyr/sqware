//! 用户 task 模块：`spawn`/`closure`/`Join`/`Builder`。

// 硬不变量：done 释放写由新任务侧、Acquire 读由 join 侧；result 单写单取。
//             SendSlot 整个传给方法走（whole-struct 捕获，使 Send 生效）。

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};

use ubi::UResult;

use crate::core::tls;
use crate::env::{room, task as env_task};

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

#[derive(Clone, Copy)]
struct SendSlot<T>(*mut Completion<T>);

unsafe impl<T> Send for SendSlot<T> {}
unsafe impl<T> Sync for SendSlot<T> {}

impl<T> SendSlot<T> {
    unsafe fn store_result(self, r: T) {
        unsafe {
            (*self.0).result = Some(r);
            (*self.0).done.store(true, Ordering::Release);
        }
    }
}

pub struct Join<T> {
    slot: *mut Completion<T>,
}

impl<T> Join<T> {
    pub fn join(self) -> T {
        let slot = self.slot;
        loop {
            // SAFETY: Join 独占；done 未置位前不读 result。
            if unsafe { (*slot).done.load(Ordering::Acquire) } {
                // SAFETY: done 置位 ⇒ result 已写入。
                let r = unsafe { (*slot).result.take() }.expect("joined task lost result");
                unsafe { drop(Box::from_raw(slot)) };
                return r;
            }
            let _ = room::wait(slot as usize, 1_000);
        }
    }
}

pub struct Builder {
    entry: usize,
    arg: usize,
    stack: usize,
}

impl Builder {
    pub const fn new(entry: usize, arg: usize) -> Self {
        Self {
            entry,
            arg,
            stack: 0,
        }
    }

    pub const fn stack(mut self, s: usize) -> Self {
        self.stack = s;
        self
    }

    pub fn spawn(self) -> UResult<usize> {
        env_task::spawn(self.entry, self.arg, self.stack)
    }
}

pub fn spawn(entry: usize, arg: usize) -> UResult<usize> {
    env_task::spawn(entry, arg, 0)
}

pub fn closure<F, T>(f: F) -> Join<T>
where
    F: FnOnce() -> T + Send + 'static,
{
    let slot = Box::into_raw(Box::new(Completion::new()));
    let send_slot = SendSlot(slot);
    // 先 done 后 wake：done Release 发表于 ecall 之前；唤醒后 Acquire 重查必见真值。
    let inner: Box<dyn FnOnce() + Send> = Box::new(move || {
        let r = f();
        let slot_ptr = send_slot.0;
        unsafe { send_slot.store_result(r) }
        let _ = room::wake(slot_ptr as usize);
    });
    let holder: Box<Box<dyn FnOnce() + Send>> = Box::new(inner);
    let ptr = Box::into_raw(holder) as usize;
    let _task = env_task::spawn(
        (utask_trampoline as extern "C" fn(usize) -> !) as usize,
        ptr,
        0,
    )
    .expect("task spawn failed");
    Join { slot }
}

#[unsafe(no_mangle)]
pub extern "C" fn utask_trampoline(arg: usize) -> ! {
    let tls_base = tls::alloc().expect("tls alloc failed");
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) tls_base, options(nomem, nostack, preserves_flags));
    }
    let holder: Box<Box<dyn FnOnce() + Send>> =
        unsafe { Box::from_raw(arg as *mut Box<dyn FnOnce() + Send>) };
    holder();
    room::exit()
}
