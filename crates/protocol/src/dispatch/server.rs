//! server — 服务目录（服务侧）：**名字空间表** + 协议适配。
//!
//! 目录是**普通 Service**：跑在 S 态 supervisor 域 `prog-dir` 里，只有一个 req
//! hole，内核没有它的入口调用（class 7 已删）。协议规范见 `docs/dispatch.md`。
//!
//! 与 [`super::client::Directory`] **同名不同角色**：本枚是**服务端**（目录自己那张
//! 预约表），那一枚是**客户端**（跟目录说话）。两侧同住一份协议。
//!
//! 依赖：本文件用 `runtime` 的机制（`runtime::env::mail` 的门闩原语）——协议层
//! **不碰** `env::ecall`，机制一律经运行时。
//!
//! # 一张表 = 预约表
//!
//! 行只能由**父域（root）的预约**产生（`reserve`）：注册只能**填**已存在的行，
//! 注销只能把行里的实例**摘空**。「谁能用哪个名字」因此不是运行时判定，而是表的
//! 形状——这就是名字权限。
//!
//! # 实例与它的门闩
//!
//! 每行至多挂一个实例：调用方 `Accord` 给目录的那枚入口门闩（token）。目录自己
//! 持着它，`Connect` 才能再转授。**顶掉实例时必须释放旧门闩**——它是资源实体的
//! 唯一强引用（`docs/dispatch.md` §7.2），不释放就漏水。释放是内核动作，故经
//! [`Release`] 注入（核心不 `use` 内核）。
//!
//! # 惰性剔除
//!
//! 实例会死（服务退出 / 封印）。判定只看 `vestor(entry)`：`None` 即视同**没有
//! 实例**——`Resolve`/`Enumerate`/`Connect` 自然报 `NotFound`，`publish` 也不把
//! 它当占用（服务重启因此能重新注册）。**读路径不改写**；死实例的门闩留到下一次
//! 写路径被释放。

use alloc::vec::Vec;

use env::Name;

// ── 核心：数据 + 原语（零内核调用）──

/// 事实来源：token → 授与人；`None` = 已死 / 不在本任务权限表里。
pub type Vestor = fn(usize) -> Option<usize>;

/// 释放一枚门闩（自释；Pole 同步 unmap）。
pub type Release = fn(usize);

/// 目录操作失败域（协议 `Reply` 是它的镜像）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectoryError {
    /// 名字已有活实例。
    Taken,
    /// 名字未被预约（不在命名空间里）/ 行内无实例。
    Unknown,
    /// 发起者不是预约者。
    NotOwner,
    /// 门闩不合格：不在目录表里 / 无授与人 / 授与人不是发起者 / 已死。
    NotGrantable,
}

/// 一行 = 一个名字的预约 + 至多一个实例。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Binding {
    name: Name,
    /// 预约者（root 指定）：只有它能 `publish` / `unpublish` / `replace`。
    publisher: usize,
    /// 实例：入口门闩在**目录侧**的 token。`None` = 未注册 / 已注销 / 已死。
    entry: Option<usize>,
}

/// 服务目录（名字唯一 = 表里至多一行同名）。
pub struct Directory {
    bindings: Vec<Binding>,
    vestor: Vestor,
    release: Release,
}

impl Directory {
    pub const fn new(vestor: Vestor, release: Release) -> Self {
        Self {
            bindings: Vec::new(),
            vestor,
            release,
        }
    }

    fn position(&self, name: &Name) -> Option<usize> {
        self.bindings.iter().position(|b| b.name == *name)
    }

    /// 活实例：`None` / 已死 → `None`。
    fn live(&self, entry: Option<usize>) -> Option<usize> {
        let token = entry?;
        (self.vestor)(token).is_some().then_some(token)
    }

    /// 顶掉 `pos` 行的实例并释放它的门闩。
    fn displace(&mut self, pos: usize) {
        if let Some(old) = self.bindings[pos].entry.take() {
            (self.release)(old);
        }
    }

    /// 鉴权：这枚门闩必须是 `who` 亲手交给目录的（`vestor == who`）。
    fn granted(&self, entry: usize, who: usize) -> Result<(), DirectoryError> {
        if (self.vestor)(entry) == Some(who) {
            Ok(())
        } else {
            Err(DirectoryError::NotGrantable)
        }
    }

    /// 预约：名字归 `publisher`（已有行则改写预约者并顶掉旧实例）。
    ///
    /// 前置：`publisher != 0`——无身份的引荐不成立（适配层保证）。
    pub fn reserve(&mut self, name: Name, publisher: usize) {
        match self.position(&name) {
            Some(pos) => {
                self.displace(pos);
                self.bindings[pos].publisher = publisher;
            }
            None => self.bindings.push(Binding {
                name,
                publisher,
                entry: None,
            }),
        }
    }

    /// 注册：把实例挂上空槽。`who` = 内核盖章的发起者。
    pub fn publish(&mut self, name: &Name, entry: usize, who: usize) -> Result<(), DirectoryError> {
        let pos = self.position(name).ok_or(DirectoryError::Unknown)?;
        if self.bindings[pos].publisher != who {
            return Err(DirectoryError::NotOwner);
        }
        if self.live(self.bindings[pos].entry).is_some() {
            return Err(DirectoryError::Taken);
        }
        self.granted(entry, who)?;
        self.displace(pos);
        self.bindings[pos].entry = Some(entry);
        Ok(())
    }

    /// 换绑：覆盖槽，不要求槽空。
    pub fn replace(&mut self, name: &Name, entry: usize, who: usize) -> Result<(), DirectoryError> {
        let pos = self.position(name).ok_or(DirectoryError::Unknown)?;
        if self.bindings[pos].publisher != who {
            return Err(DirectoryError::NotOwner);
        }
        self.granted(entry, who)?;
        self.displace(pos);
        self.bindings[pos].entry = Some(entry);
        Ok(())
    }

    /// 注销：摘实例、保留预约行。
    pub fn unpublish(&mut self, name: &Name, who: usize) -> Result<(), DirectoryError> {
        let pos = self.position(name).ok_or(DirectoryError::Unknown)?;
        if self.bindings[pos].publisher != who {
            return Err(DirectoryError::NotOwner);
        }
        self.displace(pos);
        Ok(())
    }

    /// 查实例（只读；死实例视同无）。
    pub fn entry_of(&self, name: &Name) -> Option<usize> {
        let pos = self.position(name)?;
        self.live(self.bindings[pos].entry)
    }

    /// 按名排序的下一页（只含有活实例的名字）。
    ///
    /// 排序是契约：表里顺序是插入序，目录必须给调用方一个确定的顺序，
    /// 否则 Enumerate 的结果不可复现。
    pub fn enumerate(&self, after: Option<&Name>) -> Option<Name> {
        let mut names: Vec<Name> = self
            .bindings
            .iter()
            .filter(|b| self.live(b.entry).is_some())
            .map(|b| b.name)
            .collect();
        names.sort_unstable();
        match after {
            None => names.first().copied(),
            Some(after) => names.into_iter().find(|n| n > after),
        }
    }
}

// ── 协议适配：线格式 → 核心原语（唯一碰内核处，经 runtime 的机制）──

use env::{Permission, PieToken, TaskId};

use runtime::env::mail;

use super::wire::{MSG_LEN, Reply, Request};

/// 目录转授给调用方的权限：push 请求 + pull 回复。
fn caller_permission() -> Permission {
    Permission::READ | Permission::WRITE
}

/// 查本任务表里该 token 的 `vestor`（授与人）——注入给核心的**事实来源**。
///
/// `None` = 表里没有 / 资源已封印；`Some(0)` = 在表里但没有授与人（原始自持）。
pub fn vestor_of(token: usize) -> Option<usize> {
    if token == 0 {
        return None;
    }
    mail::reserve(PieToken::new(token))
        .ok()
        .map(|(vestor, _)| vestor.get())
}

/// 释放一枚门闩（自释）——注入给核心的**释放动作**。失败即已不在表里，忽略。
pub fn release_pie(token: usize) {
    let _ = mail::release(token);
}

impl Directory {
    /// 处理一条目录请求（原始 64 字节消息），产出回复。
    ///
    /// `caller` = 调用方 task id，**由内核在 `Push` 时盖章**（`Pull` 交回），不来自
    /// 消息体；0 = 无身份。本函数不做 I/O——回复由调用方（域主循环）推送。
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
            Request::Register { name, entry } => match self.publish(name, entry.get(), caller) {
                Ok(()) => Reply::Ok,
                Err(DirectoryError::Taken) => Reply::Taken,
                Err(DirectoryError::Unknown) => Reply::NotFound,
                Err(_) => Reply::Denied,
            },
            Request::Unregister { name } => match self.unpublish(name, caller) {
                Ok(()) => Reply::Ok,
                Err(DirectoryError::Unknown) => Reply::NotFound,
                Err(_) => Reply::Denied,
            },
            Request::Replace { name, entry } => match self.replace(name, entry.get(), caller) {
                Ok(()) => Reply::Ok,
                Err(DirectoryError::Unknown) => Reply::NotFound,
                Err(_) => Reply::Denied,
            },
            Request::Resolve { name } => match self.entry_of(name) {
                Some(_) => Reply::Found { name: *name },
                None => Reply::NotFound,
            },
            Request::Enumerate { after } => match self.enumerate(after.as_ref()) {
                Some(name) => Reply::Found { name },
                None => Reply::NotFound,
            },
            Request::Connect { name } => match self.entry_of(name) {
                Some(entry) => match mail::accord(
                    PieToken::new(entry),
                    TaskId::new(caller),
                    caller_permission(),
                ) {
                    Ok(token) => Reply::Connected { entry: token },
                    Err(_) => Reply::Denied,
                },
                None => Reply::NotFound,
            },
        }
    }
}
