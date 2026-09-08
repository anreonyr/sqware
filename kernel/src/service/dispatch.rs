// service::dispatch — 服务分发（dispatcher 的服务注册表）。
//
// 注册表 Arc<SpinLock<Vec<ServiceEntry>>> 由 boot 创建 → 注册 echo 等服务 →
// clone Arc 给 dispatcher Task 闭包捕获。dispatcher 闭包独占持有（最后 Arc），
// 闭包退出时 registry drop。
//
// 命名约定：服务以名字（UTF-8 字符串）注册与查找。ServiceId（枚举）作
// uabi 边界（shell 字符串 → ServiceId 转换在用户态）。

use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use crate::lock::{Level, SpinLock};
use crate::work::mail::HoleMeta;
use crate::work::mail::memo::ResourceId;

/// 服务表项：name → 该服务的 req/rep hole（Weak 因为 dispatcher 不需要保活）。
#[derive(Clone)]
pub struct ServiceEntry {
    pub name: String,
    pub req_id: ResourceId,
    pub req: Weak<HoleMeta>,
    pub rep_id: ResourceId,
    pub rep: Weak<HoleMeta>,
}

/// 服务注册表（Arc 共享给 dispatcher 闭包）。
pub type ServiceRegistry = SpinLock<Vec<ServiceEntry>>;

/// 注册一个服务（push 到 registry 末尾；不查重——boot 期由人保证）。
pub fn register(
    registry: &Arc<ServiceRegistry>,
    name: &str,
    req_id: ResourceId,
    req: Weak<HoleMeta>,
    rep_id: ResourceId,
    rep: Weak<HoleMeta>,
) {
    registry.lock().push(ServiceEntry {
        name: String::from(name),
        req_id, req,
        rep_id, rep,
    });
}

/// 按名字查找服务。
pub fn lookup<'a>(
    registry: &'a Arc<ServiceRegistry>,
    name: &str,
) -> Option<alloc::vec::Vec<ServiceEntry>> {
    // 返回 Vec 是因为 SpinLock guard 不能跨闭包；实际只 0/1 个匹配。
    registry.lock().iter().find(|e| e.name == name).cloned().map(|e| {
        let mut v = Vec::new();
        v.push(e);
        v
    })
}

/// 创建空的注册表。
pub fn new_registry() -> Arc<ServiceRegistry> {
    Arc::new(SpinLock::new_level(Level::L3, Vec::new()))
}
