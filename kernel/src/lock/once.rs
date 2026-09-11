// OnceLock — 一次性初始化原语
//
// 提供"写入一次、只读多次"的同步原语，读取路径仅一次状态 load，无需获取 SpinLock。
// 适用于全局驱动引用、函数指针等写入后不再变动的场景。
//
// 状态三态（`AtomicU8`）：`EMPTY(0) → WRITING(1) → READY(2)`，只增不退。
//
// 内存序：
//   - `set()` 先 CAS 到「写入中」抢占唯一写入者，**写完 data 之后**才 Release store
//     「就绪」——发布晚于写入，故「读到就绪 ⇒ 数据已可见」（Acquire/Release 配对）
//   - `get()` 只在「就绪」时给出 `&T`；「写入中」对外一律不可见（返回 `None`）
//   - 抢不到「写入中」的一方等它落到「就绪」，再报「已初始化」
//
// 为什么不是 `AtomicBool`（旧版）：布尔只有两位信息，CAS 一旦成功就等于**立刻发布**，
// 而 data 的写入排在 CAS 之后——Release 覆盖不到它，另一 hart 可见「已初始化」却读到
// 尚未写入的 data。三态把「抢到写入权」与「对读者可见」拆成两件事。
//
// 今日四处消费者（`HERTZ` / `TRAP_STACK_PHYS` / `POOL` / `MACHINE`）都在副核拉起之前
// 就 `set` 完了，故旧版从未暴露；那是**启动次序的巧合**，不是本原语的性质。

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU8, Ordering};

/// 未写入。
const EMPTY: u8 = 0;
/// 已抢到写入权、正在写——对读者仍不可见。
const WRITING: u8 = 1;
/// 已写完并发布——读到它即读到完整的 `T`。
const READY: u8 = 2;

/// 一次性初始化容器。
///
/// 与 `SpinLock<Option<T>>` 相比，`OnceLock` 在已初始化后不获取锁，
/// 读取路径仅执行一次状态 `load(Acquire)`，适合读多写少的场景。
pub struct OnceLock<T> {
    state: AtomicU8,
    data: UnsafeCell<MaybeUninit<T>>,
}

// SAFETY: 初始化时由 set()/get_or_init() 写入 T 一次，之后仅提供 &T 不可变引用。
// T: Send + Sync 保证跨 hart 共享引用安全。
unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}

impl<T> OnceLock<T> {
    /// 创建一个空的 OnceLock。
    #[inline]
    pub const fn new() -> Self {
        OnceLock {
            state: AtomicU8::new(EMPTY),
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// 获取已初始化的值的引用。
    ///
    /// 未初始化（含**正在写入**）时返回 `None`。
    #[inline]
    pub fn get(&self) -> Option<&T> {
        // Acquire：与 set() 收尾的 Release store 配对 ⇒ 见到 READY 即见到 data。
        if self.state.load(Ordering::Acquire) == READY {
            // SAFETY: READY 只由写完 data 的那一方置位，且此后不再写。
            Some(unsafe { (*self.data.get()).assume_init_ref() })
        } else {
            None
        }
    }

    /// 尝试设置值。
    ///
    /// 成功设置返回 `Ok(())`，已初始化（或别人正在写）则返回 `Err(value)`。
    pub fn set(&self, value: T) -> Result<(), T> {
        // 抢占唯一写入者：CAS 成功时**没有任何读者**能看到 READY。
        match self
            .state
            .compare_exchange(EMPTY, WRITING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                // SAFETY: CAS 获胜 = 唯一写入者，data 尚未初始化。
                unsafe { (*self.data.get()).as_mut_ptr().write(value) }
                // 发布晚于写入：这一次 Release 正好覆盖上面那次写入。
                self.state.store(READY, Ordering::Release);
                Ok(())
            }
            Err(_) => {
                // 别人正在写（WRITING）或已写好（READY）。等它落到 READY 再报「已初始化」，
                // 免得调用方（`get_or_init` 的 `get().unwrap()`）撞上「已初始化但还看不见」
                // 的窗口；写方若被抢占，本循环可被中断，故不会死等。
                while self.state.load(Ordering::Acquire) == WRITING {
                    core::hint::spin_loop();
                }
                Err(value)
            }
        }
    }

    /// 获取已初始化的值的引用，若未初始化则通过闭包初始化。
    ///
    /// 即使有多个调用者并发调用 `get_or_init`，保证闭包最多执行一次。
    /// 若闭包被调用但返回时发现其他调用者已先完成初始化，返回的值会被丢弃。
    pub fn get_or_init<F>(&self, f: F) -> &T
    where
        F: FnOnce() -> T,
    {
        // 快速路径：直接检查是否已初始化
        if let Some(val) = self.get() {
            return val;
        }

        // 慢速路径：调用闭包并尝试设置
        let val = f();
        if let Err(ours) = self.set(val) {
            // 已有其他调用者抢到写入权，丢弃我们的值
            // SAFETY: `ours` 是闭包返回但未存入 data 的值，直接 drop 即可。
            drop(ours);
        }

        // 此时必定已就绪（`set` 失败也等到了 READY），unwrap 安全
        self.get().unwrap()
    }

    /// 检查是否已初始化（**正在写入不算**）。
    #[inline]
    #[allow(dead_code)]
    pub fn is_initialized(&self) -> bool {
        self.state.load(Ordering::Acquire) == READY
    }
}
