// Pie<M> — 能力门闩，泛型直指资源 Meta 类型（mail 的 HoleMeta | PoleMeta）。
//
// 编译期类型安全：M = HoleMeta | PoleMeta，`meta: Arc<M>` 精确指资源 Meta，
// 拿 Hole pie 当 Pole 用在编译期即被拦。运行时擦除由 [`AnyPie`] 的 variant 承担
// ——variant 即 tag，不再需要 marker 类型 / ResourceKind trait / PieKind 枚举。
//
// 运行时身份：每 Pie 持 permission + sire（派生来源：父门闩的 token；None = 原始
// 自持）+ heir（我交出的那一枚的坐标；None = 没交出过）+ token（全局唯一，用户句柄）
// + meta（**资源实体的唯一强引用**）。
//
// **资源寿命 = 能力寿命**：没有全局资源表，最后一份门闩消失即回收。
//
// **只存一条边**（向上的父指针）。另两个方向都是查询：授与人 = 父的持有者、
// 子门闩 = sire 指向我的那些（见 `gate::snap`）——一条关系只存一次。
//
// 用户态：Task 持 `Vec<AnyPie>`（`unit::task::pies`）；envcall 以 token 寻址。

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;

use crate::work::mail::{HoleMeta, PoleMeta, ToleMeta};

// ── 权限位（bitflags）──
//
// 单一真相在 `env::Permission`，本处 re-export 维持 `gate::Permission` 引用路径。

pub use env::Permission;

/// 子门闩的**坐标**：我交出的那一枚落到了谁手里。
///
/// 与 `sire` 成对（向上 / 向下），但**不对称是必须的**：`sire` 只存 token，因为
/// "谁持有它"可以由快照查出来（`holder`，冷路径够用）；`heir` 必须连 `task` 一起存，
/// 因为"谁持有它"正是**热路径**（数据面判权）要问的问题，而反向查询要吃全世界快照
/// （`snap()` 要分配一个 `Vec<TaskWeak>`，数据面从不这么干）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Heir {
    pub(crate) task: usize,
    pub(crate) token: usize,
}

/// 数据面操作所需的权利位（gate 核心判定授权，不感知资源实体）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// 取用 / 观察：pull / hush / open / Await —— 需 `FETCH`。
    Fetch,
    /// 投递 / 改动：push / ring / hang —— 需 `STORE`。
    Store,
    /// accord 转授 / 交出 —— 需 `VEST`（唯一的目标位）。
    Grant,
}

/// 全局 pie 身份序列号（自 1 递增）。用户句柄 + accord 撤销句柄。
fn next_pie_token() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 单个门闩：`permission` 控授权、`sire` 是派生来源（父门闩的 token；None = 原始
/// 自持）、`heir` 是我交出的那一枚（交出时写、判据清）、`token` 是用户句柄、`meta`
/// 是资源实体（唯一的强引用）。
pub struct Pie<M> {
    pub(crate) permission: Permission,
    /// 我从哪一枚派生（父门闩的 token）：None = 原始自持；Some = 经 accord 得到。
    /// 构造期定型，无 setter。
    pub(crate) sire: Option<usize>,
    /// 我交出的那一枚（交出时写、判据发现它已不在时清）。**至多一个**：
    /// "已交出"既是"我不可用"的理由，也是"我不能再交出"的理由。
    pub(crate) heir: Option<Heir>,
    pub(crate) token: usize,
    /// 资源实体：**唯一强引用**——资源随最后一份门闩一起消亡。
    ///
    /// 纪律：门闩必须在**锁外** drop（最后一份 drop 会跑 `Meta::drop`，它唤醒
    /// 等待者 / 撤映射 / 还帧，全是 L3 或更外层的活）。
    pub(crate) meta: Arc<M>,
}

// 手动 Clone：显式写清字段复制（Arc 强计数 +1）。
impl<M> Clone for Pie<M> {
    fn clone(&self) -> Self {
        Self {
            permission: self.permission,
            sire: self.sire,
            heir: self.heir,
            token: self.token,
            meta: self.meta.clone(),
        }
    }
}

impl<M> Pie<M> {
    /// 资源实体（唯一强引用）。
    pub(crate) fn meta(&self) -> &Arc<M> {
        &self.meta
    }

    /// 单权利位检查（**不含 alive**：Denied/Dead 语义仍由调用方逐条区分）。
    pub fn allows(&self, need: Need) -> bool {
        match need {
            Need::Fetch => self.permission.contains(Permission::FETCH),
            Need::Store => self.permission.contains(Permission::STORE),
            // Grant = 持 VEST（唯一的目标位）：ONLY 是形态位，不授予任何事。
            Need::Grant => self.permission.contains(Permission::VEST),
        }
    }

    /// 覆盖子集：非空且 ⊆ 当前权限。
    pub fn covers(&self, subset: Permission) -> bool {
        self.permission.contains(subset) && !subset.is_empty()
    }
}

/// **形态位一致**：`ONLY` 不是调用方的选择，而是资源事实——授出时两边必须相同。
///
/// 不一致的两种情形都到此为止：想**复制**一枚独占资源（源带、subset 不带），
/// 或想给一枚**共享**资源按上形态位（源不带、subset 带）。
/// 一致时这次授出的形态由**源枚**定：带 `ONLY` ⇒ 移交；不带 ⇒ 复制。
pub(crate) fn form_ok(src: Permission, subset: Permission) -> bool {
    src.contains(Permission::ONLY) == subset.contains(Permission::ONLY)
}

// ── AnyPie ──

/// `Vec<AnyPie>` 元素：variant 即运行时 tag。
#[derive(Clone)]
pub enum AnyPie {
    Hole(Pie<HoleMeta>),
    Pole(Pie<PoleMeta>),
    /// 无数据面的权柄载体（见 `work::mail::nole`）：只有身份与存活，
    /// 故它承载**无载荷通信**（门铃）——消息要走 Hole，页要走 Pole。
    Nole(Pie<crate::work::mail::nole::NoleMeta>),
    /// 多路等待的载体（见 `work::mail::tole`）：自己不装载荷，只记着"哪几枚孔"。
    Tole(Pie<ToleMeta>),
}

impl AnyPie {
    pub fn permission(&self) -> Permission {
        match self {
            AnyPie::Hole(p) => p.permission,
            AnyPie::Pole(p) => p.permission,
            AnyPie::Nole(p) => p.permission,
            AnyPie::Tole(p) => p.permission,
        }
    }

    /// 派生来源（父门闩的 token）；None = 原始自持。
    pub fn sire(&self) -> Option<usize> {
        match self {
            AnyPie::Hole(p) => p.sire,
            AnyPie::Pole(p) => p.sire,
            AnyPie::Nole(p) => p.sire,
            AnyPie::Tole(p) => p.sire,
        }
    }

    /// 我交出的那一枚的坐标（本地锚；`None` = 没交出过）。
    ///
    /// 它是**缓存**，不是第二处真相：真相是"存在一枚我交出的子门闩、其 `sire`
    /// 指向我"。锚只是让热路径 O(1) 地读它——陈旧只会推迟自愈，方向保守（继续拒）。
    ///
    /// 只有**独占资源**（源枚带 `ONLY`）的授出才写锚：那次是**移交**，源枚在子枚
    /// 存活期间不可用；子枚消亡 ⇒ 锚陈旧 ⇒ [`usable`] 当场清锚、源枚复原。
    pub fn heir(&self) -> Option<&Heir> {
        match self {
            AnyPie::Hole(p) => p.heir.as_ref(),
            AnyPie::Pole(p) => p.heir.as_ref(),
            AnyPie::Nole(p) => p.heir.as_ref(),
            AnyPie::Tole(p) => p.heir.as_ref(),
        }
    }

    /// 资源开辟者（`EnvCall::Mail(MailCall::Owned)` 的 `owner` 一侧）。
    ///
    /// `None` = Meta 已封印（`Seal` 之后）：答不出完整事实。
    ///
    /// 判"谁能封印"用（`Seal` 在 envcall 适配层过它）：封印后 `None` **正是要的答案**
    /// ——已死的东西不再接受第二次封印。
    pub fn owner(&self) -> Option<usize> {
        match self {
            AnyPie::Hole(p) => p.meta.alive().then(|| p.meta.owner()),
            AnyPie::Pole(p) => p.meta.alive().then(|| p.meta.owner()),
            AnyPie::Nole(p) => p.meta.alive().then(|| p.meta.owner()),
            AnyPie::Tole(p) => p.meta.alive().then(|| p.meta.owner()),
        }
    }

    /// **这扇门是谁开的**（不问死活，纯读 Meta 字段）。
    ///
    /// 与 [`AnyPie::owner`] 的差别只有一个 `alive()` 闸，而这一格正是**退场钩子**
    /// 要的：它按"开者是谁"决定封印哪些资源（`gate::doom`）。用 `owner()` 会漏掉
    /// "已经封印但表项还在"的那些——那不影响结论（封印幂等），却让判据变成
    /// "取决于封印先后"，而这条边应当是确定性的。
    ///
    /// 契约：**只在自家 `pies` 锁里叫**（`p.meta` 的存活与否由 Meta 自己的锁管，
    /// 与本函数的调用点无关）。
    pub fn owner_task(&self) -> usize {
        match self {
            AnyPie::Hole(p) => p.meta.owner(),
            AnyPie::Pole(p) => p.meta.owner(),
            AnyPie::Nole(p) => p.meta.owner(),
            AnyPie::Tole(p) => p.meta.owner(),
        }
    }

    pub fn token(&self) -> usize {
        match self {
            AnyPie::Hole(p) => p.token,
            AnyPie::Pole(p) => p.token,
            AnyPie::Nole(p) => p.token,
            AnyPie::Tole(p) => p.token,
        }
    }

    /// 资源可用：`Live`（**已封印 → false**；已回收的资源根本无门闩可查）。
    pub fn alive(&self) -> bool {
        match self {
            AnyPie::Hole(p) => p.meta.alive(),
            AnyPie::Pole(p) => p.meta.alive(),
            AnyPie::Nole(p) => p.meta.alive(),
            AnyPie::Tole(p) => p.meta.alive(),
        }
    }

    /// 单权利位检查（**不含 alive**：Denied/Dead 语义仍由调用方逐条区分）。
    pub fn allows(&self, need: Need) -> bool {
        match self {
            AnyPie::Hole(p) => p.allows(need),
            AnyPie::Pole(p) => p.allows(need),
            AnyPie::Nole(p) => p.allows(need),
            AnyPie::Tole(p) => p.allows(need),
        }
    }

    /// 覆盖子集：非空且 ⊆ 当前权限。
    pub fn covers(&self, subset: Permission) -> bool {
        match self {
            AnyPie::Hole(p) => p.covers(subset),
            AnyPie::Pole(p) => p.covers(subset),
            AnyPie::Nole(p) => p.covers(subset),
            AnyPie::Tole(p) => p.covers(subset),
        }
    }
}

/// 造 pie（accord / envcall 创建共用）：token 在此分配。
pub(crate) fn new_pie<M>(meta: Arc<M>, permission: Permission, sire: Option<usize>) -> Pie<M> {
    Pie {
        permission,
        sire,
        heir: None,
        token: next_pie_token(),
        meta,
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
    /// 这一枚被我交出去了（接收方手里的那一枚还在）：交回即复原，不是失败。
    Caged,
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
            GateError::Caged => -7,
        }
    }
}
