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
}
