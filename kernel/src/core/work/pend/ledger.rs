use alloc::sync::Weak;
use alloc::vec::Vec;
use core::sync::atomic::AtomicUsize;

use env::PieToken;

use crate::core::work::unit::task::Task;
use crate::machine::sync::SpinLock;

use super::HoldTable;
use super::pie::Pie;

/// 一个任务的能力账本：登记表（`serial`）下挂它持有的 pie、它替子任务记住的
/// 父子对（`heirs`）与来源引用（`references`）；
/// `version` 是乐观校验用的抄本号。
pub(crate) struct Pend {
    serial: SpinLock<()>,
    pub(crate) version: AtomicUsize,
    pub(crate) pies: SpinLock<Vec<Pie>>,
    pub(crate) heirs: SpinLock<Vec<(PieToken, Weak<Task>, PieToken)>>,
    pub(crate) references: HoldTable,
}

impl Pend {
    pub(crate) fn new() -> Self {
        Self {
            serial: SpinLock::new(()),
            version: AtomicUsize::new(0),
            pies: SpinLock::new(Vec::new()),
            heirs: SpinLock::new(Vec::new()),
            references: HoldTable::new(),
        }
    }

    pub(crate) fn lock(&self) -> crate::machine::sync::spin::SpinLockGuard<'_, ()> {
        self.serial.lock()
    }

    /// 不自旋地看一眼这把串行锁在不在别人（或自己）手上。
    ///
    /// 自检用：判断一次真实调用（如移交）在执行/通知的某一刻还持不持这把锁，
    /// 而不能去取同一把锁——那就是自己等自己。
    #[cfg(debug_assertions)]
    pub(crate) fn try_lock(&self) -> Option<crate::machine::sync::spin::SpinLockGuard<'_, ()>> {
        self.serial.try_lock()
    }
}
