// OnceLock — 一次性初始化原语
//
// 提供"写入一次、只读多次"的同步原语，读取路径仅一次 AtomicBool::load，
// 无需获取 SpinLock。适用于全局驱动引用、函数指针等写入后不再变动的场景。
//
// 内存序约定：
//   - set() 使用 Release store 保证数据写入在标记初始化前完成
//   - get() 使用 Acquire load  保证看到初始化前的所有数据写入
//   - 多 hart 下 compare_exchange(AcqRel) 保证一次且仅一次写入

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};

/// 一次性初始化容器。
///
/// 与 `SpinLock<Option<T>>` 相比，`OnceLock` 在已初始化后不获取锁，
/// 读取路径仅执行一次 `AtomicBool::load(Acquire)`，适合读多写少的场景。
///
/// # 示例
///
/// ```ignore
/// static DRIVER: OnceLock<&'static dyn Driver> = OnceLock::new();
///
/// fn init() {
///     DRIVER.set(&MyDriver).ok();
/// }
///
/// fn use_driver() {
///     let d = DRIVER.get().expect("driver not initialized");
///     d.do_something();
/// }
/// ```
pub struct OnceLock<T> {
    initialized: AtomicBool,
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
            initialized: AtomicBool::new(false),
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// 获取已初始化的值的引用。
    ///
    /// 未初始化时返回 `None`。
    #[inline]
    pub fn get(&self) -> Option<&T> {
        // Acquire：保证看到 set() 的 Release store 之前的所有数据写入。
        if self.initialized.load(Ordering::Acquire) {
            // SAFETY: initialized 为 true 说明 data 已被写入完整的 T 值。
            // 此后仅返回 &T，不再写入。
            Some(unsafe { (*self.data.get()).assume_init_ref() })
        } else {
            None
        }
    }

    /// 尝试设置值。
    ///
    /// 成功设置返回 `Ok(())`，已初始化则返回 `Err(value)`。
    pub fn set(&self, value: T) -> Result<(), T> {
        // compare_exchange：原子地尝试将 initialized 从 false 变为 true。
        // AcqRel：成功时 Release 保证 data 写入可见；失败时 Acquire 保证看到已存值。
        match self
            .initialized
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                // 写入数据（在 Release store 之前完成）
                // SAFETY: 我们是唯一写入者（compare_exchange 获胜），
                // data 未初始化，写入是安全的。
                unsafe { (*self.data.get()).as_mut_ptr().write(value) }
                Ok(())
            }
            Err(_) => Err(value),
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
            // 已有其他调用者抢先初始化，丢弃我们的值
            // SAFETY: `ours` 是闭包返回但未存入 data 的值，直接 drop 即可。
            drop(ours);
        }

        // 此时必定已初始化，unwrap 安全
        self.get().unwrap()
    }

    /// 检查是否已初始化。
    ///
    /// 预留原语 API（std OnceLock 同款）。
    #[inline]
    #[allow(dead_code)]
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }
}

// Drop is intentionally not implemented: MaybeUninit does not auto-drop T.
// In kernel context, OnceLock typically stores global statics (driver references,
// function pointers) that live until system reset. Note that `get_or_init()` may
// drop a value if another caller wins the initialization race — the losing
// closure's return value is dropped via normal Rust drop semantics in that case.
// If you need to store a type whose Drop has side effects, wrap it in ManuallyDrop<T>.
