//! 来源 ↔ 关系 的完整弱关联：这份来源的窗口被哪些关系压着。
//!
//! 与关系索引分工不同：索引按 VA 找关系、只服务本地址空间；这份关联按**来源**
//! 找关系、跨地址空间，专门给“把这些来源页撤下来”的枚举用；它只负责发现，
//! 不是第二份权威。条目是**一条一次分配**的 Box 链：关系退场就把自己那一条摘掉释放，
//! 所以没有“全局 Vec 容量高水位”这种账。
//! 自身锁内不取关系锁、地址空间锁，也不升级任何临时强引用（死项的 strong_count 仅为清死索引）。

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::machine::sync::{Level, OnceLock, SpinLock};

use crate::core::memory::address::{PageIndex, VirtAddr};

use super::super::flat::FlatRange;
use super::NestError;
use super::View;

mod clearance;
mod record;

pub(crate) use clearance::Clearance;
use record::Record;

/// 全核唯一的一份关联：单链头，每条一个关系。
struct SourceViews {
    head: SpinLock<Option<Box<Record>>>,
}

fn registry() -> &'static SourceViews {
    static REGISTRY: OnceLock<SourceViews> = OnceLock::new();
    REGISTRY.get_or_init(|| SourceViews {
        head: SpinLock::new_level(Level::L3, None),
    })
}

/// 把链走一遗：死项当场退场（清它只靠弱计数，不升级），活项重建；
/// `keep` 对每个活项调用一次（不取锁、不分配）。
fn rebuild(head: &mut Option<Box<Record>>, mut keep: impl FnMut(&Record)) {
    let mut live: Option<Box<Record>> = None;
    let mut cursor = head.take();
    while let Some(mut node) = cursor {
        cursor = node.next.take();
        if node.view.strong_count() == 0 {
            continue;
        }
        keep(&node);
        node.next = live.take();
        live = Some(node);
    }
    *head = live;
}

/// 关系退场：它那一条从链上摘下并释放。在 `View::drop` 里调用：
/// 那时它的强计数已经归零，重建就会把这一条摘掉。
pub(crate) fn forget() {
    let mut head = registry().head.lock();
    rebuild(&mut head, |_| {});
}

/// 登记一段来源窗口与一个关系的关联：同一关系在同一份来源上只留一条。
///
/// 关系的首条 PTE 之前调用；失败即整次安装失败，不留半条。
/// 键按**强持的** `FlatRange` 取（调用方持有它），不缓存可回收的裸身份。
pub(crate) fn register(source: &FlatRange, view: &Arc<View>) -> Result<(), NestError> {
    let source_id = source.source_id();
    // 先备好节点：锁内不分配也不升级任何强引用。
    let record = Box::try_new(Record {
        source_id,
        view: Arc::downgrade(view),
        next: None,
    })
    .map_err(|_| NestError::OutOfMemory)?;
    let identity = Arc::as_ptr(view);
    let mut head = registry().head.lock();
    let mut duplicate = false;
    rebuild(&mut head, |known| {
        if known.source_id == source_id && known.view.as_ptr() == identity {
            duplicate = true;
        }
    });
    if !duplicate {
        let mut record = record;
        record.next = head.take();
        *head = Some(record);
    }
    Ok(())
}

/// 压在这份来源上的活关系快照；页与发布记录的判断留给锁外的调用方。
///
/// 升级出的强引用直接进返回的 Vec，在锁外才交出去：调用方必须在**不持任何 core 锁**
/// 的区域里放掉这个快照（最后一份 Arc<View> 的退场可能回调索引）。
pub(crate) fn views_over(source: &FlatRange) -> Result<Vec<Arc<View>>, NestError> {
    let source_id = source.source_id();
    let mut head = registry().head.lock();
    // 第一遗：死项退场、链重建、数出候选。
    let mut matches = 0usize;
    rebuild(&mut head, |known| {
        if known.source_id == source_id {
            matches += 1;
        }
    });
    // 链已经归位：之后的可失败步骤不会弄丢任何登记。
    let mut views = Vec::new();
    views
        .try_reserve(matches)
        .map_err(|_| NestError::OutOfMemory)?;
    let mut cursor = head.as_deref();
    while let Some(node) = cursor {
        if node.source_id == source_id
            && let Some(view) = node.view.upgrade()
        {
            views.push(view);
        }
        cursor = node.next.as_deref();
    }
    Ok(views)
}

/// 枚举压在这份来源上的全部关系，裁决 `page` 能不能安全撤下/移交。
///
/// 只在 `views_over` 内部持索引锁：裁决本身不持任何 core 锁，
/// 全部强引用快照也在这里放开（退场的那一份可能回调索引）。
pub(crate) fn clearance(source: &FlatRange, page: PageIndex) -> Result<Clearance, NestError> {
    let mut blocked = false;
    for view in &views_over(source)? {
        let Some(address) = pressing_page(view, source, page) else {
            continue;
        };
        let Some(nest) = view.reservation.nest() else {
            return Ok(Clearance::Unreachable);
        };
        // 撤页也按 Nest → View 持锁；形状与同步状态不能拼接自不同时间点。
        let state = nest.state.lock();
        if !state.invalidation.is_clean() || view.overlaps(address.as_usize(), address.as_usize()) {
            blocked = true;
        }
    }
    Ok(if blocked {
        Clearance::Blocked
    } else {
        Clearance::Clear
    })
}

/// 这段关系把来源 `source` 上的 `page` 放在哪个虚拟页上；不是它的窗口就没有。
fn pressing_page(view: &View, source: &FlatRange, page: PageIndex) -> Option<VirtAddr> {
    let inner = view.state.lock();
    if inner.source.source_id() != source.source_id() {
        return None;
    }
    let offset = page.get().checked_sub(inner.source.index().get())?;
    if offset >= inner.source.page_count().get() {
        return None;
    }
    let address = view
        .initial_start()
        .as_usize()
        .checked_add(offset.checked_mul(crate::core::memory::PAGE_SIZE)?)?;
    Some(VirtAddr::from_raw(address))
}

/// 链上现存条数（连尚未清理的死项）：元数据用量的真数，自检用。
#[cfg(debug_assertions)]
pub(crate) fn len() -> usize {
    let head = registry().head.lock();
    let mut count = 0usize;
    let mut cursor = head.as_deref();
    while let Some(node) = cursor {
        count += 1;
        cursor = node.next.as_deref();
    }
    count
}
