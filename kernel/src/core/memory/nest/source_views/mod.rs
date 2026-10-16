//! 来源 ↔ 关系 的完整弱关联：这份来源的窗口被哪些关系压着。
//!
//! 与关系索引分工不同：索引按 VA 找关系、只服务本地址空间；这份关联按**来源**
//! 找关系、跨地址空间，专门给“把这些来源页撤下来”的枚举用；它只负责发现，
//! 不是第二份权威。条目是**一条一次分配**的 Box 链：关系退场就把自己那一条摘掉释放，
//! 所以没有“全局 Vec 容量高水位”这种账。
//! 自身锁内不取关系锁、地址空间锁，也不升级任何临时强引用（死项的 strong_count 仅为清死索引）。

use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicBool, Ordering};

use crate::machine::sync::{Level, OnceLock, SpinLock};

use crate::core::memory::address::{PageIndex, VirtAddr};

use super::super::flat::FlatRange;
use super::NestError;
use super::View;

mod pressing;
mod record;

use pressing::Pressing;
use record::Record;

/// 全核唯一的一份关联：单链头，每条一个关系。
struct SourceViews {
    head: SpinLock<Option<Box<Record>>>,
}

/// 下一次候选收集在准备阶段失败：它必须在任何页槽被改动之前。
#[cfg(debug_assertions)]
static FAIL_CANDIDATES: AtomicBool = AtomicBool::new(false);

/// 让下一次两端候选收集当场耗尽：它发生在占槽之前，两端的槽与承载独占都不应动。
#[cfg(debug_assertions)]
pub(crate) fn fail_candidates() {
    assert!(crate::testing());
    FAIL_CANDIDATES.store(true, Ordering::Relaxed);
}

fn registry() -> &'static SourceViews {
    static REGISTRY: OnceLock<SourceViews> = OnceLock::new();
    REGISTRY.get_or_init(|| SourceViews {
        head: SpinLock::new_level(Level::L3, None),
    })
}

/// 把链走一遍：死项当场退场（清它只靠弱计数，不升级），活项重建；
/// `keep` 对每个活项调用一次（不取锁、不分配）。
///
/// 重建保持相对次序：一次查询不该顺手动别人登记的先后。翻转只用指针，不分配。
fn rebuild(head: &mut Option<Box<Record>>, mut keep: impl FnMut(&Record)) {
    let mut flipped: Option<Box<Record>> = None;
    let mut cursor = head.take();
    while let Some(mut node) = cursor {
        cursor = node.next.take();
        if node.view.strong_count() == 0 {
            continue;
        }
        keep(&node);
        node.next = flipped.take();
        flipped = Some(node);
    }
    *head = reverse(flipped);
}

/// 原地翻转一条链（正向拼回）——只用指针，不分配。
fn reverse(mut chain: Option<Box<Record>>) -> Option<Box<Record>> {
    let mut out: Option<Box<Record>> = None;
    while let Some(mut node) = chain {
        chain = node.next.take();
        node.next = out.take();
        out = Some(node);
    }
    out
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
    // 第一遍：死项退场、链重建、数出候选。
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

/// 枚举压在这份来源 `page` 上的全部活关系。
///
/// 每一条都连同它认住的地址空间与这一页在它窗口里的虚拟页一起交出：
/// 调用方不必自己推窗口偏移。`views_over` 的强引用纪律同样适用——
/// 返回的快照必须在**不持任何 core 锁**的区域里放掉。
///
/// `nest` 为 `None` 的那一条是失联属主：它照样出现在结果里，不在这里被跳过。
pub(crate) fn pressings_over(
    source: &FlatRange,
    page: PageIndex,
) -> Result<Vec<Pressing>, NestError> {
    let candidates = views_over(source)?;
    let mut pressings = Vec::new();
    pressings
        .try_reserve(candidates.len())
        .map_err(|_| NestError::OutOfMemory)?;
    for view in candidates {
        let Some(address) = pressing_page(&view, source, page) else {
            continue;
        };
        pressings.push(Pressing {
            nest: view.reservation.nest(),
            view,
            address,
        });
    }
    Ok(pressings)
}

/// 两端候选关系的一份弱快照：两份来源窗口上登记过的全部关系。
///
/// 与 `views_over` / `pressings_over` 的强快照不同，这里**只收 `Weak`**：它要在
/// `Flat` 锁内调用，不能在那里升级强引用（`Flat → View` 是反向锁阶），也拿不到
/// 关系自己的锁。于是它交出的是一份**完整上界**——谁真的压着本次两端的那一页，
/// 留给占槽后的 `affected_over` 按关系自己的不可变窗口筛。
///
/// 两份来源键各收一次，两端同 Flat 时同一份登记会被收两次；去重不在这里做
/// （占槽前的时间越短越好），留给 `affected_over` 在原地收回。
/// 先按数出的上界一次性备容量，再只 clone `Weak`：链上不留半条、也不在锁内分配多次。
pub(crate) fn candidates_over(
    source: &FlatRange,
    target: &FlatRange,
) -> Result<Vec<Weak<View>>, NestError> {
    #[cfg(debug_assertions)]
    if FAIL_CANDIDATES.swap(false, Ordering::Relaxed) {
        return Err(NestError::OutOfMemory);
    }
    let source_id = source.source_id();
    let target_id = target.source_id();
    let mut head = registry().head.lock();
    // 第一遍：清死项、重建链，数出完整上界（两端各计一次）。
    let mut bound = 0usize;
    rebuild(&mut head, |known| {
        if known.source_id == source_id {
            bound += 1;
        }
        if known.source_id == target_id {
            bound += 1;
        }
    });
    // 链已经归位：之后的可失败步骤不会弄丢任何登记。
    let mut candidates = Vec::new();
    candidates
        .try_reserve(bound)
        .map_err(|_| NestError::OutOfMemory)?;
    let mut cursor = head.as_deref();
    while let Some(node) = cursor {
        if node.source_id == source_id {
            candidates.push(node.view.clone());
        }
        if node.source_id == target_id {
            candidates.push(node.view.clone());
        }
        cursor = node.next.as_deref();
    }
    Ok(candidates)
}

/// 从两端候选里收回这次移交真正动过的关系：压着源页或目标页的那些。
///
/// 在无 core 锁的区域里跑：按每个关系自己（不可变）的来源窗口过滤——窗口之外
/// 的候选是上界里多收的，不是这次动过的；两端同 Flat 时同一份登记会被收两次，
/// 按 `Weak` 身份去掉重复的那条。过滤与去重全在原地做，不分配。
pub(crate) fn affected_over(
    candidates: Vec<Weak<View>>,
    source: &FlatRange,
    source_page: PageIndex,
    target: &FlatRange,
    target_page: PageIndex,
) -> Vec<Weak<View>> {
    let mut affected = candidates;
    affected.retain(|candidate| {
        let Some(view) = candidate.upgrade() else {
            return false;
        };
        pressing_page(&view, source, source_page).is_some()
            || pressing_page(&view, target, target_page).is_some()
    });
    distinct(&mut affected);
    affected
}

/// 同一个关系在清单里只留一条；原地压缩，不分配。
fn distinct(views: &mut Vec<Weak<View>>) {
    let mut kept = 0usize;
    let mut read = 0usize;
    while read < views.len() {
        let seen = (0..kept).any(|at| Weak::ptr_eq(&views[at], &views[read]));
        if seen {
            views.swap_remove(read);
        } else {
            kept += 1;
            read += 1;
        }
    }
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
