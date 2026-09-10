//! 用户 task 模块：`closure`/`Join`（域内并发 + 结果回收）。

// 硬不变量：result 单写单取；盒子的释放由 `state` 两位仲裁——子任务完工（DONE）
//             与父方弃权（LEFT）各置一位，**后到者**释放；fetch_or 的原子性同时
//             排除双释放与漏释放。
//             SendSlot 整个传给方法走（whole-struct 捕获，使 Send 生效）。

use alloc::boxed::Box;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{EnvResult, TaskId, TeamId};

use crate::core::tls;
use crate::env::{room, task as env_task};

/// 子任务已完工（result 可取）。
const DONE: usize = 1;
/// 父方已弃权（不再取 result）。
const LEFT: usize = 2;

pub struct Completion<T> {
    state: AtomicUsize,
    result: Option<T>,
}

impl<T> Completion<T> {
    const fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
            result: None,
        }
    }
}

#[derive(Clone, Copy)]
struct SendSlot<T>(*mut Completion<T>);

unsafe impl<T> Send for SendSlot<T> {}
unsafe impl<T> Sync for SendSlot<T> {}

impl<T> SendSlot<T> {
    /// 完工：写结果 → 置 DONE。若父方已弃权（LEFT 在前），本方是后到者 → 释放盒子。
    unsafe fn store_result(self, r: T) {
        unsafe {
            (*self.0).result = Some(r);
            let prev = (*self.0).state.fetch_or(DONE, Ordering::AcqRel);
            if prev & LEFT != 0 {
                drop(Box::from_raw(self.0));
            }
        }
    }
}

pub struct Join<T> {
    slot: *mut Completion<T>,
    /// 子任务句柄（spawn 时 envcall 返的）——**就是喂给 `accord` 的那个类型**，
    /// 故以句柄存，不化回裸数。
    id: TaskId,
}

impl<T> Join<T> {
    /// 子任务句柄——直接可喂 `pie.accord(join.id(), subset)`。
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// 等结果并取走。等到 DONE 说明子任务已完工且**未**释放盒子（它当时看不到
    /// LEFT——本方走 join 就不会置 LEFT），故由本方释放。
    pub fn join(self) -> T {
        let slot = self.slot;
        core::mem::forget(self); // 结果本方取走，不再走弃权路径（Drop）
        loop {
            // SAFETY: DONE 未置位前不读 result。
            if unsafe { (*slot).state.load(Ordering::Acquire) } & DONE != 0 {
                // SAFETY: DONE 置位 ⇒ result 已写入且子任务不再触碰盒子。
                let r = unsafe { (*slot).result.take() }.expect("joined task lost result");
                unsafe { drop(Box::from_raw(slot)) };
                return r;
            }
            let _ = room::wait(slot as usize, 1_000);
        }
    }
}

impl<T> Drop for Join<T> {
    /// 弃权：不等结果。置 LEFT；若子任务已完工（DONE 在前），本方是后到者 →
    /// 释放盒子，否则留给子任务释放。子任务随后的 `wake(slot)` 只把地址当**键**
    /// 用、不解引用，故先释放亦安全。
    fn drop(&mut self) {
        // SAFETY: slot 由 closure 分配、生命周期由本仲裁协议管辖。
        let prev = unsafe { (*self.slot).state.fetch_or(LEFT, Ordering::AcqRel) };
        if prev & DONE != 0 {
            unsafe { drop(Box::from_raw(self.slot)) };
        }
    }
}

/// 域内产线程跑一个闭包，返回 `Join<T>` 取回结果。
///
/// `spawn` 恒产 `Held`，故此处紧接着 `hatch`——域内线程无需跨域授权序，
/// 数据经**共享空间**的 `Completion` 槽传递（不占权限表）。
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
    let task_id = env_task::spawn(
        TeamId(0),
        (utask_trampoline as extern "C" fn(usize) -> !) as usize,
        &[ptr],
        0,
    )
    .expect("task spawn failed");
    env_task::hatch(task_id).expect("task hatch failed");
    Join { slot, id: task_id }
}

/// 当前 task id（`UnitCall::SelfId`）。无上下文 → `TaskId(0)`。
///
/// 返回句柄而非裸数：它是本任务身份、要喂给 `accord`/`revoke` 这类权柄操作，
/// 化回 `usize` 只会让调用点不得不再包一次。
pub fn self_id() -> EnvResult<TaskId> {
    env_task::self_id()
}

#[unsafe(no_mangle)]
pub extern "C" fn utask_trampoline(arg: usize) -> ! {
    // a0 = 启动参数区 VA（`Spawn` 的 args 写在栈顶）；args[0] = 闭包装箱薄指针。
    // 必须在任何调用（tls::alloc）之前读——a0 是 caller-saved。
    let ptr = unsafe { core::ptr::read_volatile(arg as *const usize) };
    let tls_base = tls::alloc().expect("tls alloc failed");
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) tls_base, options(nomem, nostack, preserves_flags));
    }
    let holder: Box<Box<dyn FnOnce(usize) + Send>> =
        unsafe { Box::from_raw(ptr as *mut Box<dyn FnOnce(usize) + Send>) };
    // closure 的参数已封进捕获，此处的形参占位 0。
    holder(0);
    room::exit()
}
