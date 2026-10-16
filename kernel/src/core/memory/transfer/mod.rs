//! 无映射移交：把同一个承载从一个合法逻辑页槽交进另一个空槽。
//!
//! 这是唯一可执行的移交入口：两端都是强持来源窗口上的相对页号。
//! 它先在两端的短双锁里占槽（源 `Moving`、目标 `Reserved`），再在不持任何 core 锁
//! 的区域里枚举压在这份来源上的全部关系，最后结算；中途任何一步失败，事务守卫都会
//! 按同一笔身份把两端原样还回去。
//!
//! 不复制数据、不分配数据帧、不碰 PTE/TLB，也不改 Flat 身份、VA 与权限。

mod error;

pub use error::TransferError;

use crate::core::memory::address::{PageIndex, PageOffset};
use crate::core::memory::flat::FlatRange;
use crate::core::memory::flat::transaction::Transaction;
use crate::core::memory::nest::source_views::{self, Clearance};

/// 一次移交：两端各自页号相对各自窗口，不是 Flat 绝对页号。
pub struct Transfer {
    source: FlatRange,
    source_page: PageIndex,
    target: FlatRange,
    target_page: PageIndex,
}

impl Transfer {
    /// 换两端绝对页号：两端各自显式检查范围；同一份来源上的同一页当场拒。
    pub fn new(
        source: FlatRange,
        source_page: PageOffset,
        target: FlatRange,
        target_page: PageOffset,
    ) -> Result<Self, TransferError> {
        let source_page = source
            .page_at(source_page)
            .map_err(|_| TransferError::OutOfRange)?;
        let target_page = target
            .page_at(target_page)
            .map_err(|_| TransferError::OutOfRange)?;
        Ok(Self {
            source,
            source_page,
            target,
            target_page,
        })
    }

    /// 执行移交：占槽 → 锁外核实这份来源上没有别的关系压着这一页 → 结算。
    pub fn execute(&self) -> Result<(), TransferError> {
        let transaction = Transaction::claim(
            &self.source,
            self.source_page,
            &self.target,
            self.target_page,
        )?;
        #[cfg(debug_assertions)]
        in_flight::visit(&self.source, self.source_page);
        self.clearance()?;
        transaction.commit();
        Ok(())
    }

    /// 不持任何 core 锁时枚举压在这份来源上的全部关系。
    ///
    /// 活动关系覆盖这一页或地址空间仍待失效时拒绝移交。
    /// 原属主失联无法核实旧 TLB，永久拒绝。
    fn clearance(&self) -> Result<(), TransferError> {
        let verdict = source_views::clearance(&self.source, self.source_page)
            .map_err(|_| TransferError::OutOfMemory)?;
        match verdict {
            Clearance::Clear => Ok(()),
            Clearance::Blocked => Err(TransferError::Busy),
            Clearance::Unreachable => Err(TransferError::NotTransferable),
        }
    }
}

#[cfg(debug_assertions)]
mod in_flight {
    use crate::core::memory::address::PageIndex;
    use crate::core::memory::flat::FlatRange;
    use crate::machine::sync::SpinLock;

    static PROBE: SpinLock<Option<fn(&FlatRange, PageIndex)>> = SpinLock::new(None);

    /// 装一个在途探针：占槽之后、裁决之前调用一次。
    ///
    /// 用例借它在**真实**的在途窗口里核对排除关系；置回 `None` 就拆掉。
    pub(crate) fn arm(probe: Option<fn(&FlatRange, PageIndex)>) {
        assert!(crate::testing());
        *PROBE.lock() = probe;
    }

    /// 调用在途探针；探针在锁外跑，不持任何 core 锁。
    pub(super) fn visit(source: &FlatRange, page: PageIndex) {
        let probe = *PROBE.lock();
        if let Some(probe) = probe {
            probe(source, page);
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) use in_flight::arm as arm_in_flight;
