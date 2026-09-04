// 资源表（ResourceTable）—— 按 id 索引全局 Hole / Pole 资源（Weak 查）。
//
// 设计动机：pie 的 `resource` 是全局 id（不是指针），envcall 入口用 id 在表里
// 找 Meta 的 Weak。Weak 升级失败 = 资源已死 = 返 Dead。
//
// 不主动清理：Weak 升级失败时 lookup 返 None，调用方按 Dead 处理；表项保留
// 直到显式 shut 移除（或空闲时 lazy 扫描——v1 不做，留作后续）。

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Weak;
use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};

use super::hole::HoleMeta;
use super::pole::PoleMeta;
use super::pie::PieKind;

/// 全局资源 id（id 自增分配；0 保留未用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId(pub usize);

pub(super) fn alloc_id() -> ResourceId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    ResourceId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

// ── 注册表：id → Weak<Meta> ──

struct ResourceTable {
    holes: HashMap<ResourceId, Weak<HoleMeta>>,
    poles: HashMap<ResourceId, Weak<PoleMeta>>,
}

fn table() -> &'static SpinLock<ResourceTable> {
    static T: OnceLock<SpinLock<ResourceTable>> = OnceLock::new();
    T.get_or_init(|| {
        SpinLock::new_level(
            Level::L3,
            ResourceTable {
                holes: HashMap::new(),
                poles: HashMap::new(),
            },
        )
    })
}

/// 注册新 Hole（Weak 升级 Arc）。
pub(crate) fn insert_hole(id: ResourceId, arc: &alloc::sync::Arc<HoleMeta>) {
    table().lock().holes.insert(id, alloc::sync::Arc::downgrade(arc));
}

/// 注册新 Pole（Weak 升级 Arc）。
pub(crate) fn insert_pole(id: ResourceId, arc: &alloc::sync::Arc<PoleMeta>) {
    table().lock().poles.insert(id, alloc::sync::Arc::downgrade(arc));
}

/// 显式移除（shut 时调用）。
pub(crate) fn remove(id: ResourceId, kind: PieKind) {
    let mut t = table().lock();
    match kind {
        PieKind::Hole => {
            t.holes.remove(&id);
        }
        PieKind::Pole => {
            t.poles.remove(&id);
        }
    }
}

/// 按 kind 查 id：返 Some(Weak) 或 None。
pub(crate) fn lookup_hole(id: ResourceId) -> Option<Weak<HoleMeta>> {
    table().lock().holes.get(&id).cloned()
}

pub(crate) fn lookup_pole(id: ResourceId) -> Option<Weak<PoleMeta>> {
    table().lock().poles.get(&id).cloned()
}