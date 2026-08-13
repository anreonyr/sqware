// TrapGuard — 中断安全守卫，锁框架内部依赖
//
// 通过 sstatus.SIE 控制 S-mode 全局中断：save() 保存当前使能状态并关中断，
// Drop 时恢复。所有需要中断安全的锁（SpinLock/RwLock/RelLock）内部复用此守卫，
// 避免各自重复编写 CSR 访问逻辑。
//
// 不对外暴露：关中断属于锁实现细节，业务代码不应直接操作 SIE。

use riscv::register::sstatus;

// 保存进入临界区前的 SIE 状态，Drop 时恢复。
pub(crate) struct TrapGuard {
    sie_was_enabled: bool,
}

impl TrapGuard {
    /// 保存当前 SIE 并关闭 S-mode 全局中断。
    ///
    /// 返回的守卫析构时恢复进入前的 SIE 使能状态。
    ///
    /// # Safety
    ///
    /// 调用者需保证处于 S-mode，且当前上下文允许屏蔽中断
    /// （关中断本身不构成死锁，恢复由 Drop 保证）。
    #[inline(always)]
    pub(crate) unsafe fn save() -> Self {
        unsafe {
            // 读取当前 SIE，若已使能则清零
            let was = sstatus::read().sie();
            if sstatus::read().sie() {
                sstatus::clear_sie();
            }
            TrapGuard {
                sie_was_enabled: was,
            }
        }
    }
}

impl Drop for TrapGuard {
    #[inline(always)]
    fn drop(&mut self) {
        // 仅当进入前 SIE 使能时才恢复，避免误开中断
        if self.sie_was_enabled {
            // SAFETY: 恢复进入临界区前保存的 SIE 状态，处于 S-mode。
            unsafe {
                sstatus::set_sie();
            }
        }
    }
}
