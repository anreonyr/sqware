//! 一个 Task 的**引用表**：它登记的那些普通引用。
//!
//! 句柄号是**全局单调**的（铸一次、永不复用），所以旧号不可能在后来的表里命中另一个对象，
//! 跨 Task 的同一个数字也不成立——表自己按 id 查找，找不到就是不属于这张表。
//! 号到符号上界就到了：那是运行时守住的条件，不用 debug 断言代替。

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::AtomicUsize;

use crate::core::memory::address::{PageCount, PageOffset};
use crate::core::memory::flat::Residency;
use crate::core::memory::nest::Access;
use crate::machine::sync::SpinLock;
use env::PieFail;

use super::hold::Hold;
use super::source_authorization::SourceAuthorization;

pub(crate) struct HoldTable {
    holds: SpinLock<Vec<Arc<Hold>>>,
}

/// 铸一个全局唯一的引用号；耗尽（或超出符号上界）即明确失败，不回收旧号。
fn next_id() -> Result<usize, PieFail> {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    crate::machine::sync::next_id(&NEXT).ok_or(PieFail::OoM)
}

impl HoldTable {
    pub(crate) const fn new() -> Self {
        Self {
            holds: SpinLock::new(Vec::new()),
        }
    }

    /// 登记一份新引用；容量与弧先备齐，失败不留下半份引用。
    pub(crate) fn insert(
        &self,
        source: Arc<SourceAuthorization>,
        offset: PageOffset,
        count: PageCount,
        ceiling: Access,
        retained: Option<Residency>,
        branch: Option<(env::PieToken, Arc<crate::core::work::grant::Grant>)>,
    ) -> Result<usize, PieFail> {
        let mut holds = self.holds.lock();
        holds.try_reserve(1).map_err(|_| PieFail::OoM)?;
        let id = next_id()?;
        let hold = Arc::try_new(Hold::new(
            id, source, offset, count, ceiling, retained, branch,
        ))
        .map_err(|_| PieFail::OoM)?;
        if !hold.retains() {
            hold.source().claim_hold();
        }
        holds.push(hold);
        Ok(id)
    }

    pub(crate) fn get(&self, id: usize) -> Option<Arc<Hold>> {
        self.holds
            .lock()
            .iter()
            .find(|hold| hold.id() == id)
            .cloned()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn len(&self) -> usize {
        self.holds.lock().len()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn capacity(&self) -> usize {
        self.holds.lock().capacity()
    }

    /// 解除这一份引用；只从这个表里摘掉它，不动其他引用，也不分配。
    pub(crate) fn remove(&self, id: usize) -> Option<Arc<Hold>> {
        let mut holds = self.holds.lock();
        let at = holds.iter().position(|hold| hold.id() == id)?;
        let hold = holds.remove(at);
        drop(holds);
        if !hold.retains() {
            hold.source().release_hold();
        }
        Some(hold)
    }

    /// Task 走到尽头：结清本表全部引用，未消费的资格随之结束。
    pub(crate) fn clear(&self) {
        let mut holds = self.holds.lock();
        let drained = core::mem::take(&mut *holds);
        drop(holds);
        for hold in &drained {
            if !hold.retains() {
                hold.source().release_hold();
            }
        }
    }
}

unsafe impl Send for HoldTable {}
unsafe impl Sync for HoldTable {}
