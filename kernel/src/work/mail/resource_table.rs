// 资源表（ResourceTable）—— 按 id 索引全局 Hole / Pole 资源。
//
// 设计动机：pie 的 `resource` 是全局 id（不是指针），envcall 入口用 id 在表里
// 找 Meta 的 **Arc**（anchor）。表持 Arc 而不是 Weak：hole_create 末尾局部
// `Arc<Meta>` drop 后没人持强引用会让 Meta 立刻死——必须表自己持 Arc 保活。
//
// 不主动清理：shut 显式 remove（drop Arc → Meta 死 → 所有 Weak 失效）；关
// 机时统一清表（资源随 OS 一起消失，不需回收）。

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;
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

// ── 注册表：id → Arc<Meta>（anchor，保 Meta 活到 shut）──

struct ResourceTable {
    holes: HashMap<ResourceId, Arc<HoleMeta>>,
    poles: HashMap<ResourceId, Arc<PoleMeta>>,
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

/// 注册新 Hole（克隆 Arc 锚定到表）。
pub(crate) fn insert_hole(id: ResourceId, arc: &Arc<HoleMeta>) {
    table().lock().holes.insert(id, arc.clone());
}

/// 注册新 Pole（克隆 Arc 锚定到表）。
pub(crate) fn insert_pole(id: ResourceId, arc: &Arc<PoleMeta>) {
    table().lock().poles.insert(id, arc.clone());
}

/// 显式移除（shut 时调用）：drop Arc → 若无其他 Arc 持有者，Meta drop。
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

/// 按 kind 查 id：返 Some(Arc) 或 None（资源已被 shut/移除）。
pub(crate) fn lookup_hole(id: ResourceId) -> Option<Arc<HoleMeta>> {
    table().lock().holes.get(&id).cloned()
}

pub(crate) fn lookup_pole(id: ResourceId) -> Option<Arc<PoleMeta>> {
    table().lock().poles.get(&id).cloned()
}