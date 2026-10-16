//! 有映射移交：把同一个承载从一个合法逻辑页槽交进另一个空槽。
//!
//! 这是唯一可执行的移交入口：两端都是强持来源窗口上的相对页号。
//! 它先在两端的短双锁里收下两端候选关系、再占槽（源 `Moving`、目标 `Reserved`），
//! 然后在不持任何 core 锁的区域里把压着这一页的每一页关系逐页撤下、逐 Nest 确认旧
//! TLB，最后结算；中途任何一步失败，事务守卫都会按同一笔身份把两端原样还回去。
//!
//! 源槽始终 `Moving` 强持那个承载：撤页只是把关系上的一页 PTE 与发布记录转入退休，
//! 不复制数据、不分配数据帧，也不改 Flat/View 身份、VA 与权限。
//! 已经确认撤下的 PTE 不因后续失败回填——那一页要回来只能按需重新发布。
//!
//! 无论成败，`execute` 都交出同一份“这次动过的关系”清单（`TransferOutcome`）；
//! 占槽前的早错没有任何关系被动过，清单为空。

mod error;
mod outcome;

pub use error::TransferError;
pub use outcome::TransferOutcome;

use alloc::vec::Vec;

use crate::core::memory::address::{PageIndex, PageOffset};
use crate::core::memory::flat::FlatRange;
use crate::core::memory::flat::transaction::Transaction;
use crate::core::memory::nest::NestError;
use crate::core::memory::nest::source_views;

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

    /// 执行移交：收下两端候选 → 占槽 → 撤下压着这一页的映射并逐 Nest 确认 → 结算。
    ///
    /// 两端候选在占槽的同一个临界区里收全（只收 `Weak`），所以一占上 `Moving`，
    /// 就没有新的关系能再压到这一页上。占槽后的失败会把两端先还原，再把结果连同
    /// 同一份关系清单交回；中途不用 `?` 抖掉清单。
    pub fn execute(&self) -> TransferOutcome {
        let claimed = Transaction::claim(
            &self.source,
            self.source_page,
            &self.target,
            self.target_page,
            || {
                source_views::candidates_over(&self.source, &self.target)
                    .map_err(|_| TransferError::OutOfMemory)
            },
        );
        let (transaction, candidates) = match claimed {
            Ok(pair) => pair,
            // 占槽前的早错：没有一格槽被改动，也没有关系被这次移交动过。
            Err(error) => {
                return TransferOutcome {
                    result: Err(error),
                    affected: Vec::new(),
                };
            }
        };
        #[cfg(debug_assertions)]
        in_flight::visit(&self.source, self.source_page);
        // 占槽之后、不持任何 core 锁：候选是完整上界，这里按关系自己的窗口收回
        // 真正动过的那几条。
        let affected = source_views::affected_over(
            candidates,
            &self.source,
            self.source_page,
            &self.target,
            self.target_page,
        );
        let result = self.withdraw_pressings(&transaction);
        if result.is_ok() {
            transaction.commit();
        } else {
            // 失败：守卫在这里析构，两端先按同一笔身份还原，之后才把结果交回。
            drop(transaction);
        }
        TransferOutcome { result, affected }
    }

    /// 撤下压着源页的全部映射：每条关系逐 Nest 撤页并确认旧 TLB。
    ///
    /// 占槽之后的枚举才是完整的：这次占槽让源页没有可用的承载，并发的
    /// “抓承载快照 + 写 PTE”（两个发布路径都先取短期驻留）要么已经登记，
    /// 要么当场被这个在途槽拒掉，不会再给这一页添一条新映射。
    fn withdraw_pressings(&self, transaction: &Transaction) -> Result<(), TransferError> {
        let pressings = source_views::pressings_over(&self.source, self.source_page)
            .map_err(|_| TransferError::OutOfMemory)?;
        // 永久拒绝优先于任何撤销：内核映射与全局条目都撤不掉、也确认不了，
        // 失联属主同样无从核实旧 TLB。它们都不能迁，也不该先撤普通页再发现。
        for pressing in &pressings {
            let Some(nest) = &pressing.nest else {
                return Err(TransferError::NotTransferable);
            };
            if nest.asid().is_kernel() || pressing.view.is_global() {
                return Err(TransferError::NotTransferable);
            }
        }
        let carrier = transaction.carrier();
        for pressing in &pressings {
            let nest = pressing
                .nest
                .as_ref()
                .expect("the reachable owners were checked above");
            // 每条关系的这一页都必须正是源槽此刻强持的那一份：撤页原语在取出发布
            // 记录之后、撕下 PTE 之前核它。
            nest.unpublish_page(&pressing.view, pressing.address, &carrier)
                .map_err(unpublish)?;
        }
        Ok(())
    }
}

/// 逐页撤销的失败折成移交失败：分配耗尽、内核永久映射、未确认的同步。
///
/// 未确认的同步是可重试的 `Busy`：重试会接着那次失效继续，或按需重新发布一页。
fn unpublish(error: NestError) -> TransferError {
    match error {
        NestError::OutOfMemory => TransferError::OutOfMemory,
        NestError::Pinned => TransferError::NotTransferable,
        NestError::Shootdown => TransferError::Busy,
        other => unreachable!("unpublish_page does not fail with {other:?}"),
    }
}

#[cfg(debug_assertions)]
mod in_flight {
    use crate::core::memory::address::PageIndex;
    use crate::core::memory::flat::FlatRange;
    use crate::machine::sync::SpinLock;

    static PROBE: SpinLock<Option<fn(&FlatRange, PageIndex)>> = SpinLock::new(None);

    /// 装一个在途探针：占槽之后、撤页之前调用一次。
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
