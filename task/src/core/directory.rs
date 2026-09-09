//! Directory — 服务目录（服务侧）：名字 → 入口门闩 token 的注册表 + 协议适配。
//!
//! 目录是**普通 Service**：它跑在 S 态 supervisor 域 `task-dir` 里，只有一个 req
//! hole，内核没有它的入口调用（class 7 已删）。协议规范见 `docs/dispatch.md`。
//!
//! 与「内核闭包版」的差别只有一处：绑定里存的是**入口门闩的 token**（usize），不是
//! `Pie` 对象——门闩一直留在目录自己的权限表里，`Connect` 用 `mail::accord` 转授
//! 子集。身份用 `mail::owned` 查该 token 的 `vestor`（内核在 `Accord` 时赋值，
//! 消息体伪造不了）；没带有效回信 pie 即无身份（`caller = 0`）。

use alloc::vec::Vec;

use env::PieToken;
use env::dispatch::{MSG_LEN, Name, Reply, Request};

use crate::env::mail;

/// 目录转授给调用方的权限：push 请求 + pull 回复。
fn caller_permission() -> env::Permission {
    env::Permission::READ | env::Permission::WRITE
}

/// 目录操作失败域（协议 `Reply` 是它的镜像）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectoryError {
    /// 名字已占用。
    Taken,
    /// 名字无绑定。
    Unknown,
    /// 发起者不是发布者（`binding.owner != caller`）。
    NotOwner,
    /// 门闩不可转授：不在本任务表里 / 无授与人 / 缺 VEST / 已死 / 目标任务不存在。
    NotGrantable,
}

/// 绑定项：名字 → 入口门闩（token 留在目录权限表里）+ 服务 owner task id。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Binding {
    pub name: Name,
    pub entry: usize,
    pub owner: usize,
}

/// 查本任务表里该 token 的 `vestor`（授与人）。
///
/// `None` = 表里没有 / 资源已封印；`Some(0)` = 在表里但没有授与人（原始自持）。
pub fn vestor_of(token: usize) -> Option<usize> {
    if token == 0 {
        return None;
    }
    mail::owned(token).ok().map(|(vestor, _)| vestor.get())
}

/// 服务目录（名字唯一 = 表里至多一条同名绑定）。
#[derive(Default)]
pub struct Directory {
    bindings: Vec<Binding>,
}

impl Directory {
    pub fn new() -> Self {
        Self::default()
    }

    fn position(&self, name: &Name) -> Option<usize> {
        self.bindings.iter().position(|b| b.name == *name)
    }

    /// 绑定：名字未占用 + 入口门闩在本任务权限表里且有授与人（owner = 其 vestor）。
    ///
    /// 注：内核版 `take_entry` 是「取出即移入绑定表」，此处 token 始终留在表里，
    /// 故重复注册同一枚门闩不会被拒——名字唯一仍是硬约束。
    pub fn bind(&mut self, name: Name, entry: usize) -> Result<(), DirectoryError> {
        if self.position(&name).is_some() {
            return Err(DirectoryError::Taken);
        }
        let owner = vestor_of(entry).ok_or(DirectoryError::NotGrantable)?;
        if owner == 0 {
            return Err(DirectoryError::NotGrantable); // 原始自持：无主，不可注册
        }
        self.bindings.push(Binding { name, entry, owner });
        Ok(())
    }

    /// 解绑：仅发布者（= 绑定时记下的 owner）可解绑。
    pub fn unbind(&mut self, name: &Name, who: usize) -> Result<(), DirectoryError> {
        let pos = self.position(name).ok_or(DirectoryError::Unknown)?;
        if self.bindings[pos].owner != who {
            return Err(DirectoryError::NotOwner);
        }
        self.bindings.remove(pos);
        Ok(())
    }

    /// 换绑：服务重启换了门闩、名字不变。仅发布者可换。
    pub fn replace(&mut self, name: &Name, entry: usize, who: usize) -> Result<(), DirectoryError> {
        let pos = self.position(name).ok_or(DirectoryError::Unknown)?;
        if self.bindings[pos].owner != who {
            return Err(DirectoryError::NotOwner);
        }
        let owner = vestor_of(entry).ok_or(DirectoryError::NotGrantable)?;
        if owner == 0 {
            return Err(DirectoryError::NotGrantable);
        }
        self.bindings[pos] = Binding {
            name: *name,
            entry,
            owner,
        };
        Ok(())
    }

    /// 按名取绑定。
    pub fn resolve(&self, name: &Name) -> Option<Binding> {
        self.position(name).map(|pos| self.bindings[pos])
    }

    /// 排序枚举：`after` 之后的第一条名字（None = 从头开始）。
    ///
    /// 排序是契约：表里顺序是插入序，目录必须给调用方一个确定的顺序，
    /// 否则 Enumerate 的结果不可复现。
    pub fn enumerate(&self, after: Option<&Name>) -> Option<Name> {
        let mut names: Vec<Name> = self.bindings.iter().map(|b| b.name).collect();
        names.sort_unstable();
        match after {
            None => names.first().copied(),
            Some(after) => names.into_iter().find(|n| n > after),
        }
    }

    /// 连接：把绑定的入口门闩转授一份给调用方，返新 token（调用方那侧的句柄）。
    ///
    /// 调用方用 `mail::owned(token).owner` 求服务 task id——那是**资源开辟者**
    /// （服务自己 `UnsealHole` 出来的门闩），不受目录转授改写。
    pub fn connect(&self, name: &Name, caller: usize) -> Result<usize, DirectoryError> {
        let binding = self.resolve(name).ok_or(DirectoryError::Unknown)?;
        mail::accord(binding.entry, caller, caller_permission())
            .map_err(|_| DirectoryError::NotGrantable)
    }

    /// 处理一条目录请求（原始 64 字节消息），产出回复。
    ///
    /// `caller` = 调用方 task id（由调用方按请求取自**回信 pie 的 `vestor`**，
    /// 不来自消息体；0 = 无身份）。本函数不做 I/O——回复由调用方（域主循环）推送。
    pub fn serve(&mut self, caller: usize, msg: &[u8]) -> Reply {
        if msg.len() < MSG_LEN {
            return Reply::Denied;
        }
        match Request::decode(msg) {
            Ok(request) => self.handle(caller, &request),
            Err(_) => Reply::Denied,
        }
    }

    fn handle(&mut self, caller: usize, request: &Request) -> Reply {
        match request {
            Request::Register { name, entry } => match self.bind(*name, entry.get()) {
                Ok(()) => Reply::Ok,
                Err(DirectoryError::Taken) => Reply::Taken,
                Err(_) => Reply::Denied,
            },
            Request::Unregister { name } => match self.unbind(name, caller) {
                Ok(()) => Reply::Ok,
                Err(DirectoryError::Unknown) => Reply::NotFound,
                Err(_) => Reply::Denied,
            },
            Request::Replace { name, entry } => {
                if self.position(name).is_none() {
                    return Reply::NotFound;
                }
                match self.replace(name, entry.get(), caller) {
                    Ok(()) => Reply::Ok,
                    Err(DirectoryError::Unknown) => Reply::NotFound,
                    Err(_) => Reply::Denied,
                }
            }
            Request::Resolve { name } => {
                if self.position(name).is_some() {
                    Reply::Found { name: *name }
                } else {
                    Reply::NotFound
                }
            }
            Request::Enumerate { after } => match self.enumerate(after.as_ref()) {
                Some(name) => Reply::Found { name },
                None => Reply::NotFound,
            },
            Request::Connect { name } => match self.connect(name, caller) {
                Ok(entry) => Reply::Connected {
                    entry: PieToken(entry),
                },
                Err(DirectoryError::Unknown) => Reply::NotFound,
                Err(_) => Reply::Denied,
            },
        }
    }
}
