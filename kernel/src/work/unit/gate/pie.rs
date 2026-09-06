// Pie<M> — 能力门闩，泛型直指资源 Meta 类型（mail 的 HoleMeta | PoleMeta）。
//
// 编译期类型安全：M = HoleMeta | PoleMeta，`weak: Weak<M>` 精确指资源 Meta，
// 拿 Hole pie 当 Pole 用在编译期即被拦。运行时擦除由 [`AnyPie`] 的 variant 承担
// ——variant 即 tag，不再需要 marker 类型 / ResourceKind trait / PieKind 枚举。
//
// 运行时身份：每 Pie 持 resource（全局 id）+ permission + vestor（授与来源，
// None=原始自持）+ token（全局唯一，用户句柄 + accord 撤销句柄）+ weak（检存活）。
//
// 用户态：Task 持 `Vec<AnyPie>`（`unit::task::pies`）；envcall 以 token 寻址。

use core::sync::atomic::{AtomicU64, Ordering};

use alloc::sync::Weak;

use crate::work::mail::{HoleMeta, PoleMeta, ResourceId};

// ── 权限位（bitflags）──
//
// 单一真相在 `ubi::Permission`，本处 re-export 维持 `gate::Permission` 引用路径。

pub use ubi::Permission;

/// 全局 pie 身份序列号（自 1 递增）。用户句柄 + accord 撤销句柄。
fn next_pie_token() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 单个门闩：`resource` 指向门洞、`permission` 控授权、`vestor` 是授与来源
/// （revoke 验 vestor == me）、`token` 是用户句柄、`weak` 检存活。
pub struct Pie<M> {
    pub(crate) resource: ResourceId,
    pub(crate) permission: Permission,
    /// 授与本 pie 的人：None = 原始自持；Some(id) = 经 accord 来自 task id。
    pub(crate) vestor: Option<usize>,
    pub(crate) token: u64,
    pub(crate) weak: Weak<M>,
}

// 手动 Clone：Weak<M> 无需 M: Clone；显式写清字段复制（弱引用计数 +1，不碰 Meta）。
impl<M> Clone for Pie<M> {
    fn clone(&self) -> Self {
        Self {
            resource: self.resource,
            permission: self.permission,
            vestor: self.vestor,
            token: self.token,
            weak: self.weak.clone(),
        }
    }
}

impl<M> Pie<M> {
    pub fn resource(&self) -> ResourceId {
        self.resource
    }

    pub fn permission(&self) -> Permission {
        self.permission
    }

    pub fn vestor(&self) -> Option<usize> {
        self.vestor
    }

    pub fn token(&self) -> u64 {
        self.token
    }

    /// 存活：`Weak::upgrade` 成功 = Meta 仍活。
    pub fn alive(&self) -> bool {
        self.weak.upgrade().is_some()
    }
}

// ── AnyPie ──

/// `Vec<AnyPie>` 元素：variant 即运行时 tag。
#[derive(Clone)]
pub enum AnyPie {
    Hole(Pie<HoleMeta>),
    Pole(Pie<PoleMeta>),
}

impl AnyPie {
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

    pub fn vestor(&self) -> Option<usize> {
        match self {
            AnyPie::Hole(p) => p.vestor,
            AnyPie::Pole(p) => p.vestor,
        }
    }

    pub fn token(&self) -> u64 {
        match self {
            AnyPie::Hole(p) => p.token,
            AnyPie::Pole(p) => p.token,
        }
    }

    pub fn alive(&self) -> bool {
        match self {
            AnyPie::Hole(p) => p.alive(),
            AnyPie::Pole(p) => p.alive(),
        }
    }
}

/// 造 pie（accord / envcall 创建共用）：token 在此分配。
pub(crate) fn new_pie<M>(
    resource: ResourceId,
    permission: Permission,
    vestor: Option<usize>,
    weak: Weak<M>,
) -> Pie<M> {
    Pie {
        resource,
        permission,
        vestor,
        token: next_pie_token(),
        weak,
    }
}

// ── GateError ──

/// 能力门闩错误类型（D1 负码：见 `GateError::code`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateError {
    /// 权限不足 / pie 不存在 / 类型不匹配。
    Denied,
    /// Meta 已 seal 或 Weak upgrade 失败。
    Dead,
    /// Hole 槽满 / 槽空（条件未就绪）。
    Busy,
    /// 资源耗尽。
    OOM,
    /// 字节数非页对齐 / 非法。
    NotAligned,
}

impl GateError {
    pub const fn code(self) -> isize {
        match self {
            GateError::Denied => -1,
            GateError::Dead => -2,
            GateError::Busy => -3,
            GateError::OOM => -4,
            GateError::NotAligned => -5,
        }
    }
}
