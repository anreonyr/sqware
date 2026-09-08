// service::dispatch — 服务目录（注册表核心 + 协议适配）。
//
// 核心 = `Directory` 数据 + 6 个操作（bind/unbind/replace/resolve/enumerate/connect），
// 不依赖协议编解码；适配 = `serve`，把一条 hole 消息翻译成一次核心调用 + 一条回复。
//
// 目录是**普通 Service**：它只有一个 req hole。回信通道由**内核在 boot 期预置**
// （v1 单 client：目录 → 主 client 的 hole），调用方身份也由内核给出——消息体里
// 的任何字段都不参与身份判定。多 client 时改为调用方自带回信 pie（协议里
// [49..57] 那个保留字段即接入点），身份取其 `vestor`。
//
// 两条轴：`Binding` 持的是**入口门闩**（Pie），不是 id + Weak——注册的资格就是
// 「能把门闩交出来」，Connect 的授权就是 `gate::accord` 转授子集，无需新权限系统。

use alloc::sync::Arc;
use alloc::vec::Vec;

use hashbrown::HashMap;

use ubi::dispatch::{MSG_LEN, Name, Reply, Request};
use ubi::{PieToken, TaskId};

use crate::lock::{Level, SpinLock};
use crate::work::mail::hole::HoleMeta;
use crate::work::unit::gate::{self, AnyPie, Need, Permission, Pie};
use crate::work::unit::task::Task;

/// 目录转授给调用方的权限：push 请求 + pull 回复。
fn caller_permission() -> Permission {
    Permission::READ | Permission::WRITE
}

/// 绑定项：名字 → 服务的入口门闩（目录持有；Connect 时转授子集）。
#[derive(Clone)]
pub struct Binding {
    pub name: Name,
    pub entry: Pie<HoleMeta>,
}

/// 目录操作失败域（协议 `Reply` 是它的镜像）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectoryError {
    /// 名字已占用。
    Taken,
    /// 名字无绑定。
    Unknown,
    /// 发起者不是发布者（`entry.vestor() != who`）。
    NotOwner,
    /// 门闩不可转授：缺 VEST / 覆盖不足 / 已死 / 目标任务不存在。
    NotGrantable,
}

/// 服务目录（名字唯一 = HashMap 键）。
#[derive(Default)]
pub struct Directory {
    bindings: HashMap<Name, Binding>,
}

/// 目录注册表（Arc 共享给 dispatcher 闭包，闭包独占持有）。
pub type ServiceRegistry = SpinLock<Directory>;

/// 建空目录。
pub fn new_registry() -> Arc<ServiceRegistry> {
    Arc::new(SpinLock::new_level(Level::L3, Directory::default()))
}

// ── 核心操作 ──

/// 绑定：名字未占用才接受。
pub fn bind(
    reg: &Arc<ServiceRegistry>,
    name: Name,
    entry: &Pie<HoleMeta>,
) -> Result<(), DirectoryError> {
    let mut dir = reg.lock();
    if dir.bindings.contains_key(&name) {
        return Err(DirectoryError::Taken);
    }
    dir.bindings.insert(
        name,
        Binding {
            name,
            entry: entry.clone(),
        },
    );
    Ok(())
}

/// 解绑：仅发布者（= 入口门闩的 vestor）可解绑。
pub fn unbind(
    reg: &Arc<ServiceRegistry>,
    name: &Name,
    who: usize,
) -> Result<(), DirectoryError> {
    let mut dir = reg.lock();
    let binding = dir.bindings.get(name).ok_or(DirectoryError::Unknown)?;
    if binding.entry.vestor() != Some(who) {
        return Err(DirectoryError::NotOwner);
    }
    dir.bindings.remove(name);
    Ok(())
}

/// 换绑：服务重启换了门闩、名字不变。仅发布者可换。
pub fn replace(
    reg: &Arc<ServiceRegistry>,
    name: &Name,
    entry: &Pie<HoleMeta>,
    who: usize,
) -> Result<(), DirectoryError> {
    let mut dir = reg.lock();
    let binding = dir.bindings.get(name).ok_or(DirectoryError::Unknown)?;
    if binding.entry.vestor() != Some(who) {
        return Err(DirectoryError::NotOwner);
    }
    dir.bindings.insert(
        *name,
        Binding {
            name: *name,
            entry: entry.clone(),
        },
    );
    Ok(())
}

/// 按名取绑定（Clone 出锁，不逃逸 SpinLock guard）。
pub fn resolve(reg: &Arc<ServiceRegistry>, name: &Name) -> Option<Binding> {
    reg.lock().bindings.get(name).cloned()
}

/// 排序枚举：`after` 之后的第一条名字（None = 从头开始）。
///
/// 排序是契约：HashMap 迭代顺序无保证，目录必须给调用方一个确定的顺序，
/// 否则 Enumerate 的结果不可复现。
pub fn enumerate(reg: &Arc<ServiceRegistry>, after: Option<&Name>) -> Option<Name> {
    let dir = reg.lock();
    let mut names: Vec<Name> = dir.bindings.keys().copied().collect();
    names.sort_unstable();
    match after {
        None => names.first().copied(),
        Some(after) => names.into_iter().find(|n| n > after),
    }
}

/// 连接：把绑定的入口门闩转授一份给调用方，返 (新 token, 服务 owner task id)。
///
/// owner = `entry.vestor()`——调用方据此把回信 hole `Accord` 给服务。
pub fn connect(
    reg: &Arc<ServiceRegistry>,
    name: &Name,
    caller: usize,
    grantor: usize,
) -> Result<(PieToken, TaskId), DirectoryError> {
    let binding = resolve(reg, name).ok_or(DirectoryError::Unknown)?;
    let owner = binding.entry.vestor().ok_or(DirectoryError::NotGrantable)?;
    let subset = caller_permission();
    if !binding.entry.alive()
        || !binding.entry.allows(Need::Grant)
        || !binding.entry.covers(subset)
    {
        return Err(DirectoryError::NotGrantable);
    }
    let target = crate::work::room::scheduler::core::lookup_task_by_id_weak(caller)
        .ok_or(DirectoryError::NotGrantable)?;
    let src = AnyPie::Hole(binding.entry);
    let token =
        gate::accord(&src, &target, subset, grantor).map_err(|_| DirectoryError::NotGrantable)?;
    Ok((PieToken(token), TaskId(owner)))
}

// ── 适配层：一条请求 → 一条回复 ──

/// 处理一条目录请求，产出回复。
///
/// `caller` = 调用方 task id（**由内核给**，不来自消息体）；`me` = 目录自己的
/// task（Register/Replace 从它的权限表取登记时委托来的入口门闩）。
/// 回信由调用方（dispatcher 闭包）推送到预置通道——本函数不做 I/O。
pub fn serve(
    reg: &Arc<ServiceRegistry>,
    me: &Arc<Task>,
    caller: usize,
    msg: &[u8; MSG_LEN],
) -> Reply {
    match Request::decode(msg) {
        Ok(request) => handle(reg, me, caller, &request),
        Err(_) => Reply::Denied,
    }
}

/// 从目录自己的权限表取出登记时委托来的入口门闩（取出即移入绑定表）。
fn take_entry(me: &Arc<Task>, token: u64) -> Option<Pie<HoleMeta>> {
    let mut pies = me.pies.lock();
    let pos = pies
        .iter()
        .position(|p| p.token() == token && matches!(p, AnyPie::Hole(_)))?;
    match pies.remove(pos) {
        AnyPie::Hole(p) => Some(p),
        _ => unreachable!("position 已过滤非 Hole"),
    }
}

fn handle(reg: &Arc<ServiceRegistry>, me: &Arc<Task>, caller: usize, request: &Request) -> Reply {
    match request {
        Request::Register { name, entry, .. } => {
            // 先查名字再取门闩：Taken 时门闩仍在接待表里（目录是唯一写者，
            // 且 serve 串行，故此处无 TOCTOU）。
            if resolve(reg, name).is_some() {
                return Reply::Taken;
            }
            let Some(entry_pie) = take_entry(me, entry.get()) else {
                return Reply::Denied;
            };
            match bind(reg, *name, &entry_pie) {
                Ok(()) => Reply::Ok,
                Err(DirectoryError::Taken) => Reply::Taken,
                Err(_) => Reply::Denied,
            }
        }
        Request::Unregister { name, .. } => match unbind(reg, name, caller) {
            Ok(()) => Reply::Ok,
            Err(DirectoryError::Unknown) => Reply::NotFound,
            Err(_) => Reply::Denied,
        },
        Request::Replace { name, entry, .. } => {
            match resolve(reg, name) {
                None => Reply::NotFound,
                Some(binding) if binding.entry.vestor() != Some(caller) => Reply::Denied,
                Some(_) => {
                    let Some(entry_pie) = take_entry(me, entry.get()) else {
                        return Reply::Denied;
                    };
                    match replace(reg, name, &entry_pie, caller) {
                        Ok(()) => Reply::Ok,
                        Err(DirectoryError::Unknown) => Reply::NotFound,
                        Err(_) => Reply::Denied,
                    }
                }
            }
        }
        Request::Resolve { name, .. } => match resolve(reg, name) {
            Some(_) => Reply::Found { name: *name },
            None => Reply::NotFound,
        },
        Request::Enumerate { after, .. } => match enumerate(reg, after.as_ref()) {
            Some(name) => Reply::Found { name },
            None => Reply::NotFound,
        },
        Request::Connect { name, .. } => match connect(reg, name, caller, me.ident.id) {
            Ok((entry, owner)) => Reply::Connected { entry, owner },
            Err(DirectoryError::Unknown) => Reply::NotFound,
            Err(_) => Reply::Denied,
        },
    }
}
