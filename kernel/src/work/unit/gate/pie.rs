// 门闩（Pie）—— **类型面**（[`PieType`]）＋ **实例面**。
//
// # 类型面
//
// `Hole` / `Pole` / `Nole` / `Tole` 是四种**能力类型**（零尺寸标记），各在**自己的
// 定义处**声明两件事：
//
//     PieType
//     ├── type Mail   —— 它承载什么资源形态（HoleMeta | PoleMeta | NoleMeta | ToleMeta）
//     └── type Mark   —— 它的记号是什么**值类型**（记号的**值**是运行期给的）
//
// `Pie<T>` 由它参数化 ⇒ `Pie<Hole>` 与 `Pie<Pole>` 是**不同类型**，拿孔的 Pie 当页用
// 在编译期即被拦。运行时擦除由 [`AnyPie`] 的 variant 承担——variant 即 tag，四种各一个。
//
// # 实例面
//
// `permission` 控授权、`sire` 是派生来源（父门闩的号；None = 原始自持）、`heir` 是我
// 交出的那一枚（交出时写、判据清）、`token` 是这枚门闩的号（全局唯一；用户态收到的
// 句柄是同一个 `PieToken`）、`mark` 是**这一枚**在协议上算哪条路（badge）、`meta` 是
// **资源实体的唯一强引用**。
//
// **资源寿命 = 能力寿命**：没有全局资源表，最后一份门闩消失即回收。
//
// **只存一条边**（向上的父指针）。另两个方向都是查询：授与人 = 父的持有者、
// 子门闩 = sire 指向我的那些（见 `gate::snap`）——一条关系只存一次。
//
// 用户态：Task 持 `Vec<AnyPie>`（`unit::task::pies`）；envcall 以 token 寻址。

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;

use env::{Mark, PieToken, TaskId};

use super::GateFail;

use crate::work::mail::nole::NoleMeta;
use crate::work::mail::{HoleMeta, PoleMeta, ToleMeta};
use crate::work::unit::task::Task;

// ── 类型面：Mail（资源形态）与 PieType（能力类型）──

/// **资源形态（Mail）**：能被门闩承载的东西——四种，各一族数据面。
///
/// 契约里**只放泛型那一侧真正读得到的那几手**：`Pie<T>` 里 `meta: Arc<T::Mail>` 对泛型
/// 代码是不透明的，`narrow::set_perm` 只能经它读资源——那是 `alive`。
/// **`owner` 不在这份契约里**：它的读者（[`AnyPie::owner`] / [`AnyPie::owner_task`]）
/// 每一臂的 `T` 都是具体类型、走四个 Meta 的**固有**方法，契约那一份没有读者
/// ——按"没有读者的格不留在台面上"不加（出现泛型侧读者时再加回来）。
///
/// 固有方法本身必须留着：`mail` 不引 `gate`（单向边 `gate → mail`），
/// `mail/hole.rs` 里那些 `meta.alive()` 只能走固有那一份。
pub(crate) trait Mail: Send + Sync + 'static {
    /// 资源可用（已封印 → false）。
    fn alive(&self) -> bool;
}

// 四份委托写**显式路径**：`self.alive()` 会解析到固有方法（同效但绕一层），
// `Self::alive` 会解析回本契约（递归）——两个坑都别踩。
impl Mail for HoleMeta {
    fn alive(&self) -> bool {
        HoleMeta::alive(self)
    }
}

impl Mail for PoleMeta {
    fn alive(&self) -> bool {
        PoleMeta::alive(self)
    }
}

impl Mail for NoleMeta {
    fn alive(&self) -> bool {
        NoleMeta::alive(self)
    }
}

impl Mail for ToleMeta {
    fn alive(&self) -> bool {
        ToleMeta::alive(self)
    }
}

/// **能力类型（PieType）**：一种门闩在**自己的定义处**声明它承载什么、记号是什么值类型。
pub(crate) trait PieType {
    /// 它承载的资源形态。
    type Mail: Mail;
    /// 它的记号是什么**值类型**——记号的**值**是运行期给的（`Unseal*` 刻的、`Accord`
    /// 可另刻的），故这一格是"记号长什么样"，不是"哪条路"。今天四种都是 `env::Mark`。
    type Mark;
}

/// 孔的那一种门闩（有槽：消息穿孔）。
pub(crate) struct Hole;
/// 页的那一种门闩（有页：借映视图）。
pub(crate) struct Pole;
/// 无数据面权柄载体的那一种门闩（门铃）。
pub(crate) struct Nole;
/// 多路等待载体的那一种门闩（组）。
pub(crate) struct Tole;

impl PieType for Hole {
    type Mail = HoleMeta;
    type Mark = Mark;
}

impl PieType for Pole {
    type Mail = PoleMeta;
    type Mark = Mark;
}

impl PieType for Nole {
    type Mail = NoleMeta;
    type Mark = Mark;
}

impl PieType for Tole {
    type Mail = ToleMeta;
    type Mark = Mark;
}

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
    pub(crate) task: TaskId,
    pub(crate) token: PieToken,
}

/// 数据面操作所需的权利位（gate 核心判定授权，不感知资源实体）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// 取用 / 观察：pull / hush / open / Await —— 需 `FETCH`。
    Fetch,
    /// 投递 / 改动：push / ring / attach —— 需 `STORE`。
    Store,
    /// accord 转授 / 交出 —— 需 `VEST`（唯一的目标位）。
    Grant,
}

/// 全局门闩号序列（自 1 递增、永不复用）——与 `mail` 那三枚号的 `alloc_id` 同款。
///
/// 号的**出生地**在这里（`PieToken::mint`）：线上与用户态收到的都是同一个类型。
fn alloc_id() -> PieToken {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    PieToken::mint(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// 单个门闩：`permission` 控授权、`sire` 是派生来源（父门闩的号；None = 原始
/// 自持）、`heir` 是我交出的那一枚（交出时写、判据清）、`token` 是这枚门闩的号、
/// `mark` 是这一枚的记号（badge）、`meta` 是资源实体（唯一的强引用）。
pub struct Pie<T: PieType> {
    pub(crate) permission: Permission,
    /// 我从哪一枚派生（父门闩的号）：None = 原始自持；Some = 经 accord 得到。
    /// 构造期定型，无 setter。
    pub(crate) sire: Option<PieToken>,
    /// 我交出的那一枚（交出时写、判据发现它已不在时清）。**至多一个**：
    /// "已交出"既是"我不可用"的理由，也是"我不能再交出"的理由。
    pub(crate) heir: Option<Heir>,
    pub(crate) token: PieToken,
    /// **记号（badge）**：这一枚在协议上算哪条路。
    ///
    /// 与 `meta` 的分工：`meta` 是**资源**（同一扇门的所有副本共享同一份 `Arc`）；
    /// 记号是**这一枚**的（`Accord` 可给子枚另刻一枚；`Mark::NONE` = 照源枚）。
    /// 内核只保管、只递回，**从不解释、从不比较**。
    pub(crate) mark: T::Mark,
    /// 资源实体：**唯一强引用**——资源随最后一份门闩一起消亡。
    ///
    /// 纪律：门闩必须在**锁外** drop（最后一份 drop 会跑 `Meta::drop`，它唤醒
    /// 等待者 / 撤映射 / 还帧，全是 L3 或更外层的活）。
    pub(crate) meta: Arc<T::Mail>,
}

// SAFETY: 本类型唯一的 `!Send` 字段是 `token`——`PieToken` 的 `!Send + !Sync` 说的是
// **号只在它那张表里成立**（用户态怕的是句柄被线程 / 闭包捕获出去）。内核这一侧的表就是
// `Task.pies`，而核际迁移是**连着表一起走**的：号没有离开它的表，作数据随表迁移没有内存
// 安全含义。故此处只否掉"跨对象搬运"那一层禁令（`Mail` 一侧照旧由 `Arc<T::Mail>` 的
// `Send + Sync` 约束管）。
unsafe impl<T: PieType> Send for Pie<T> where T::Mark: Send {}

// 手动 Clone：显式写清字段复制（Arc 强计数 +1）。
impl<T: PieType> Clone for Pie<T>
where
    T::Mark: Copy,
{
    fn clone(&self) -> Self {
        Self {
            permission: self.permission,
            sire: self.sire,
            heir: self.heir,
            token: self.token,
            mark: self.mark,
            meta: self.meta.clone(),
        }
    }
}

impl<T: PieType> Pie<T> {
    /// 资源实体（唯一强引用）。
    pub(crate) fn meta(&self) -> &Arc<T::Mail> {
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

/// `Vec<AnyPie>` 元素：variant 即运行时 tag——四种 PieType 各一个。
#[derive(Clone)]
pub enum AnyPie {
    Hole(Pie<Hole>),
    Pole(Pie<Pole>),
    /// 无数据面的权柄载体（见 `work::mail::nole`）：只有身份与存活，
    /// 故它承载**无载荷通信**（门铃）——消息要走 Hole，页要走 Pole。
    Nole(Pie<Nole>),
    /// 多路等待的载体（见 `work::mail::tole`）：自己不装载荷，只记着"哪几枚孔"。
    Tole(Pie<Tole>),
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

    /// 派生来源（父门闩的号）；None = 原始自持。
    pub fn sire(&self) -> Option<PieToken> {
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

    /// 资源开辟者（`EnvCall::Pie(PieCall::Reserve)` 的 `owner` 一侧）。
    ///
    /// `None` = Meta 已封印（`Seal` 之后）：答不出完整事实。
    ///
    /// 判"谁能封印"用（`Seal` 在 envcall 适配层过它）：封印后 `None` **正是要的答案**
    /// ——已死的东西不再接受第二次封印。
    pub fn owner(&self) -> Option<TaskId> {
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
    pub fn owner_task(&self) -> TaskId {
        match self {
            AnyPie::Hole(p) => p.meta.owner(),
            AnyPie::Pole(p) => p.meta.owner(),
            AnyPie::Nole(p) => p.meta.owner(),
            AnyPie::Tole(p) => p.meta.owner(),
        }
    }

    pub fn token(&self) -> PieToken {
        match self {
            AnyPie::Hole(p) => p.token,
            AnyPie::Pole(p) => p.token,
            AnyPie::Nole(p) => p.token,
            AnyPie::Tole(p) => p.token,
        }
    }

    /// **记号**（`Collect` / `Reserve` 的第三格）：这一枚在协议上算哪条路。
    ///
    /// **每一枚都答得出**——记号是 Pie 的事实，不是孔的事实（`Mark::NONE` = 没刻过）。
    /// 与 [`AnyPie::owner`] 分工：那一手答**资源**的来历（四种 Mail 都答得出），
    /// 而 `Collect` 的 owner 那一格只对**活着的孔**有意义——那道闸在 ABI 那一层
    /// （见 `envcall/pie.rs::collect`），不在这里。
    pub fn mark(&self) -> Mark {
        match self {
            AnyPie::Hole(p) => p.mark,
            AnyPie::Pole(p) => p.mark,
            AnyPie::Nole(p) => p.mark,
            AnyPie::Tole(p) => p.mark,
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

/// 造 pie（accord / envcall 创建共用）：号在此分配。
pub(crate) fn new_pie<T: PieType>(
    meta: Arc<T::Mail>,
    mark: T::Mark,
    permission: Permission,
    sire: Option<PieToken>,
) -> Pie<T> {
    Pie {
        permission,
        sire,
        heir: None,
        token: alloc_id(),
        mark,
        meta,
    }
}

// ── 按 token 取用：判据顺序的唯一一处 ──

/// 在**这张表**里按 token 定位那一枚——**不过任何闸**（死活、权限都不看）。
///
/// 用它的是"要那一枚**本身**"的动词：`Release`（放下）、`Reserve`（查来历）、`Accord`
/// 的源枚（它自己的四道闸在 [`super::accord`] 里）。其余动词要 [`accede`]。
///
/// **表里没有 → `Denied`**（不是我的东西）。整枚在放锁前克隆出来：最后一份 clone 落在
/// 锁外 drop，`Meta::drop`（唤醒等待者 / 撤映射 / 还帧）是 L3 或更外层的活，绝不能压在
/// `pies` 锁上。
pub(crate) fn locate(task: &Arc<Task>, token: PieToken) -> Option<AnyPie> {
    let pies = task.pies.lock();
    pies.iter().find(|p| p.token() == token).cloned()
}

/// 定位 + 过两关：**已封印 → `Dead`；权不够 → `Denied`**。
///
/// # 顺序只写在这一处，全轴共用
///
/// 表里没有 → `Denied`；已封印 → `Dead`；权不够 → `Denied`。**同一个已封印的 token 不因
/// 动词不同换一个答案**——`Narrow` 曾经把"覆盖子集"排在死活之前，于是「已封印 + 越权
/// 子集」报 `Denied` 而不是 `Dead`：一个声明过的码在重叠那一格上取不到。判据下沉到这里
/// 之后，那种"按动词分叉"在结构上就写不出来了（要分叉得再手写一遍查找）。
///
/// **不查「被关住」**（第三维，见 `envcall::pie::usable`）：它要摸**别人**的表，必须在
/// 放开本任务 `pies` 之后判——故调用方拿到这里返回的抄件、放锁之后再问它。
pub(crate) fn accede<E: GateFail>(
    task: &Arc<Task>,
    token: PieToken,
    need: Need,
) -> Result<AnyPie, E> {
    let pie = locate(task, token).ok_or_else(E::denied)?;
    if !pie.alive() {
        return Err(E::dead());
    }
    if !pie.allows(need) {
        return Err(E::denied());
    }
    Ok(pie)
}
