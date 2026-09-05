// memo — 全局资源备忘：id → Arc<Meta>（anchor 保活到 seal）。
//
// 动机：pie 的 `resource` 是全局 id，memo 按 id 找 Meta 的 Arc 保活——表持 Arc
// 而非 Weak：unseal 末尾局部 Arc drop 后没人持强引用会让 Meta 立刻死。
//
// 单表擦除形态：`Meta` enum 装两种资源 Arc。不主动清理：seal 显式 remove；关机
// 统一清（资源随 OS 一起消失）。

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;
use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};

use super::hole::HoleMeta;
use super::pole::PoleMeta;

/// 全局资源 id（自增分配；0 保留未用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId(pub usize);

pub(super) fn alloc_id() -> ResourceId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    ResourceId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// 资源擦除形态：memo 只按 id 查 Arc 保活，不参与编译期类型区分。
#[derive(Clone)]
pub enum Meta {
    Hole(Arc<HoleMeta>),
    Pole(Arc<PoleMeta>),
}

struct Memo {
    metas: HashMap<ResourceId, Meta>,
}

fn memo() -> &'static SpinLock<Memo> {
    static T: OnceLock<SpinLock<Memo>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, Memo { metas: HashMap::new() }))
}

/// 注册资源（克隆 Arc 锚定到表）。
pub(crate) fn insert(id: ResourceId, meta: Meta) {
    memo().lock().metas.insert(id, meta);
}

/// 显式移除（seal 时调用）：drop Arc → 若无其他持有者，Meta drop。
pub(crate) fn remove(id: ResourceId) {
    memo().lock().metas.remove(&id);
}

/// 按 id 查：Some(Meta) 或 None（已 seal / 移除）。
pub(crate) fn lookup(id: ResourceId) -> Option<Meta> {
    memo().lock().metas.get(&id).cloned()
}
