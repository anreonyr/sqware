// Pie<M> — 能力门闩，泛型直指资源 Meta 类型（mail 的 HoleMeta | PoleMeta）。
//
// 编译期类型安全：M = HoleMeta | PoleMeta，`weak: Weak<M>` 精确指资源 Meta，
// 拿 Hole pie 当 Pole 用在编译期即被拦。运行时擦除由 [`AnyPie`] 的 variant 承担
// ——variant 即 tag，不再需要 marker 类型 / ResourceKind trait / PieKind 枚举。
//
// 运行时身份：每 Pie 持 resource（全局 id）+ permission + sire（派生来源：父门闩的
// token；None = 原始自持）+ token（全局唯一，用户句柄）+ weak（检存活）。
//
// **只存一条边**（向上的父指针）。另两个方向都是查询：授与人 = 父的持有者、
// 子门闩 = sire 指向我的那些（见 `gate::snap`）——一条关系只存一次。
//
// 用户态：Task 持 `Vec<AnyPie>`（`unit::task::pies`）；envcall 以 token 寻址。

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Weak;

use crate::work::mail::{HoleMeta, PoleMeta, ResourceId};

// ── 权限位（bitflags）──
//
// 单一真相在 `env::Permission`，本处 re-export 维持 `gate::Permission` 引用路径。

pub use env::Permission;

/// 数据面操作所需的权利位（gate 核心判定授权，不感知资源实体）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// pull / map / unmap —— 需 R。
    Read,
    /// push —— 需 W。
    Write,
    /// accord 转授 —— 需 VEST 或 BACK。
    Grant,
}

/// 全局 pie 身份序列号（自 1 递增）。用户句柄 + accord 撤销句柄。
fn next_pie_token() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 单个门闩：`resource` 指向门洞、`permission` 控授权、`sire` 是派生来源（父门闩的
/// token；None = 原始自持）、`token` 是用户句柄、`weak` 检存活。
pub struct Pie<M> {
    pub(crate) resource: ResourceId,
    pub(crate) permission: Permission,
    /// 我从哪一枚派生（父门闩的 token）：None = 原始自持；Some = 经 accord 得到。
    /// 构造期定型，无 setter。
    pub(crate) sire: Option<usize>,
    pub(crate) token: usize,
    pub(crate) weak: Weak<M>,
}

// 手动 Clone：Weak<M> 无需 M: Clone；显式写清字段复制（弱引用计数 +1，不碰 Meta）。
impl<M> Clone for Pie<M> {
    fn clone(&self) -> Self {
        Self {
            resource: self.resource,
            permission: self.permission,
            sire: self.sire,
            token: self.token,
            weak: self.weak.clone(),
        }
    }
}

impl<M> Pie<M> {
    /// 存活：`Weak::upgrade` 成功 = Meta 仍活。
    pub fn alive(&self) -> bool {
        self.weak.upgrade().is_some()
    }

    /// 单权利位检查（**不含 alive**：Denied/Dead 语义仍由调用方逐条区分）。
    pub fn allows(&self, need: Need) -> bool {
        match need {
            Need::Read => self.permission.contains(Permission::READ),
            Need::Write => self.permission.contains(Permission::WRITE),
            // Grant = VEST 或 BACK（原始鉴权 OR 语义：有其一即转授）。
            Need::Grant => {
                self.permission.contains(Permission::VEST)
                    || self.permission.contains(Permission::BACK)
            }
        }
    }

    /// 覆盖子集：非空且 ⊆ 当前权限。
    pub fn covers(&self, subset: Permission) -> bool {
        self.permission.contains(subset) && !subset.is_empty()
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

    /// 派生来源（父门闩的 token）；None = 原始自持。
    pub fn sire(&self) -> Option<usize> {
        match self {
            AnyPie::Hole(p) => p.sire,
            AnyPie::Pole(p) => p.sire,
        }
    }

    /// 资源开辟者（`EnvCall::Mail(MailCall::Owned)` 的 `owner` 一侧）。
    ///
    /// `None` = Meta 已封印：开辟者随资源消失，答不出完整事实。
    /// 读 `weak.upgrade()` 而非查 memo——不引入新锁序（pies 与 memo 同为 L3）。
    pub fn owner(&self) -> Option<usize> {
        match self {
            AnyPie::Hole(p) => p.weak.upgrade().map(|m| m.owner()),
            AnyPie::Pole(p) => p.weak.upgrade().map(|m| m.owner()),
        }
    }

    pub fn token(&self) -> usize {
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

    /// 单权利位检查（**不含 alive**：Denied/Dead 语义仍由调用方逐条区分）。
    pub fn allows(&self, need: Need) -> bool {
        match self {
            AnyPie::Hole(p) => p.allows(need),
            AnyPie::Pole(p) => p.allows(need),
        }
    }

    /// 覆盖子集：非空且 ⊆ 当前权限。
    pub fn covers(&self, subset: Permission) -> bool {
        match self {
            AnyPie::Hole(p) => p.covers(subset),
            AnyPie::Pole(p) => p.covers(subset),
        }
    }
}

/// 造 pie（accord / envcall 创建共用）：token 在此分配。
pub(crate) fn new_pie<M>(
    resource: ResourceId,
    permission: Permission,
    sire: Option<usize>,
    weak: Weak<M>,
) -> Pie<M> {
    Pie {
        resource,
        permission,
        sire,
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
    OoM,
    /// 字节数非页对齐 / 非法。
    NotAligned,
    /// 镜像不可装载（`Build` 的 parse / 装载任一步失败）。
    BadImage,
}

impl GateError {
    pub const fn code(self) -> isize {
        match self {
            GateError::Denied => -1,
            GateError::Dead => -2,
            GateError::Busy => -3,
            GateError::OoM => -4,
            GateError::NotAligned => -5,
            GateError::BadImage => -6,
        }
    }
}
