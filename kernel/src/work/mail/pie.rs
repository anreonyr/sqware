// Pie<T> — mail 的门闩。
//
// 类型级见证：T 决定 Pie 持哪个 Meta 的 Weak（`Pie<Hole>::weak: Weak<HoleMeta>`，
// `Pie<Pole>::weak: Weak<PoleMeta>`），编译期保证类型义务，运行时不可混淆。
//
// 运行时身份：每个 Pie 持 resource_id（全局 Hole/Pole id）+ permission
// （READ/WRITE/VEST 已用，BACK 留位）；alive() 经 Weak::upgrade 检测 Meta 是否仍活。
//
// 用户态用法：Task 持 `Vec<AnyPie>`，每条 AnyPie 是 `Hole(Pie<Hole>)` 或
// `Pole(Pie<Pole>)`。envcall 入口 dispatch 通过 pie_idx 索引 Vec，按 kind 分派
// 到 hole:: / pole:: / vest 数据面。

use bitflags::bitflags;
use core::marker::PhantomData;

use alloc::sync::{Arc, Weak};

use super::hole::HoleMeta;
use super::pole::PoleMeta;
use super::resource_table::ResourceId;

// ── 权限位（bitflags）──

bitflags! {
    /// 门闩权限位掩码。
    ///
    /// v1.1：`READ | WRITE | VEST` 已实现（创建者天然能 Vest）；`BACK` 留位未实现。
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Permission: u32 {
        /// Read 权：观察 / 接收 / 重读。
        const READ  = 1 << 0;
        /// Write 权：修改 / 投递 / 写入。
        const WRITE = 1 << 1;
        /// Vest 权：把 pie 复制给其他 Task（自身 permission 不变）。
        const VEST  = 1 << 2;
        /// Back 权（预留，v1.1 不实现）：只能 Vest 回 grantor。
        const BACK  = 1 << 3;
    }
}

/// Hole 单消息字节数（内核邮路槽字大小；栈拷贝，无动态分配）。
pub const HOLE_MSG_LEN: usize = 64;

/// Hole marker——零大小，编译期区分 Hole 类 pie。
#[derive(Clone, Copy)]
pub struct Hole;
/// Pole marker——零大小，编译期区分 Pole 类 pie。
#[derive(Clone, Copy)]
pub struct Pole;

/// 资源种类（运行时 tag）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PieKind {
    Hole,
    Pole,
}

/// 把 marker 类型与 Meta 类型绑定。
pub trait ResourceKind {
    const KIND: PieKind;
    type Meta;
}

impl ResourceKind for Hole {
    const KIND: PieKind = PieKind::Hole;
    type Meta = HoleMeta;
}

impl ResourceKind for Pole {
    const KIND: PieKind = PieKind::Pole;
    type Meta = PoleMeta;
}

// ── Pie<T> ──

/// 单个门闩：`resource` 指向门洞、`permission` 控授权、`weak` 检存活。
pub struct Pie<T: ResourceKind> {
    pub(crate) resource: ResourceId,
    pub(crate) permission: Permission,
    pub(crate) weak: Weak<T::Meta>,
    _t: PhantomData<T>,
}

// 手动 impl Clone：避免 derive 加上 T::Meta: Clone 约束（Meta 含 SpinLock 不能 Clone）。
impl<T: ResourceKind> Clone for Pie<T> {
    fn clone(&self) -> Self {
        Self {
            resource: self.resource,
            permission: self.permission,
            weak: self.weak.clone(),
            _t: PhantomData,
        }
    }
}

impl<T: ResourceKind> Pie<T> {
    pub fn resource(&self) -> ResourceId {
        self.resource
    }

    pub fn permission(&self) -> Permission {
        self.permission
    }

    /// L1 存活：`Weak::upgrade` 成功 = Meta 仍活。
    pub fn alive(&self) -> bool {
        self.weak.upgrade().is_some()
    }
}

// ── AnyPie ──

/// `Vec<AnyPie>` 元素：variant 即 T 标签，运行时 kind 由 variant 决定。
#[derive(Clone)]
pub enum AnyPie {
    Hole(Pie<Hole>),
    Pole(Pie<Pole>),
}

impl AnyPie {
    pub fn kind(&self) -> PieKind {
        match self {
            AnyPie::Hole(_) => PieKind::Hole,
            AnyPie::Pole(_) => PieKind::Pole,
        }
    }

    pub fn resource(&self) -> ResourceId {
        match self {
            AnyPie::Hole(p) => p.resource,
            AnyPie::Pole(p) => p.resource,
        }
    }

    pub fn permission(&self) -> Permission {
        match self {
            AnyPie::Hole(p) => p.permission,
            AnyPie::Pole(p) => p.permission,
        }
    }

    pub fn alive(&self) -> bool {
        match self {
            AnyPie::Hole(p) => p.alive(),
            AnyPie::Pole(p) => p.alive(),
        }
    }
}

/// 测试 / 内部用：从 Arc 派生 Weak 包装 Pie。
pub(super) fn new_pie<T: ResourceKind>(
    resource: ResourceId,
    permission: Permission,
    weak: Weak<T::Meta>,
) -> Pie<T> {
    Pie {
        resource,
        permission,
        weak,
        _t: PhantomData,
    }
}

/// 测试 / 内部用：从 Arc 直接派生 Pie。
pub(super) fn pie_from_arc<T: ResourceKind>(
    resource: ResourceId,
    permission: Permission,
    arc: &Arc<T::Meta>,
) -> Pie<T> {
    new_pie(resource, permission, Arc::downgrade(arc))
}

// ── MailError ──

/// mail 错误类型（D1 负码：见 `MailError::code`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailError {
    /// 权限不足 / pie 不存在 / 类型不匹配。
    Denied,
    /// Meta 已 shut 或 Weak upgrade 失败。
    Dead,
    /// Hole 槽满 / 槽空（条件未就绪）。
    Busy,
    /// 资源耗尽。
    OOM,
    /// 字节数非页对齐 / 非法。
    NotAligned,
}

impl MailError {
    pub const fn code(self) -> isize {
        match self {
            MailError::Denied => -1,
            MailError::Dead => -2,
            MailError::Busy => -3,
            MailError::OOM => -4,
            MailError::NotAligned => -5,
        }
    }
}