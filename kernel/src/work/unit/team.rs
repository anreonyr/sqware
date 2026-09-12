// 团队（进程容器）— 持有地址空间 + 成员簿记 + 血缘（sire）。
//
// 生命周期：最后一个线程退出 → Arc<Team> 归零 → 团队回收；内核团队为
// 'static 单例，唯一拥有内核地址空间，永不回收。
//
// 血缘：`sire`（生我者）在构造期定型；`heir`（我生）挂在 **Task** 上（见
// task.rs）——强持有子域，既是撑命源，也是 `spawn` 的授权凭证，还是
// `doom` 级联的遍历源。三者合一，无独立全局表。
//
// **闭合在构造期**（K1）：`TeamBuilder::spawn` 在 sire 非空时立即把新域推进
// sire.heir——「sire 已记 ⇒ 必在 heir 里」是构造义务，不留第二个入口
//（原 `Task::adopt` 独立调用面已并入）。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{Name, TeamId};

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::unit::space::Space;

use super::task::{Task, TaskBuilder};

/// 团队（进程）— 共享地址空间的线程容器。
///
/// tasks 为成员簿记（弱引用，无强环），生命周期仍由引用计数决定。
///
/// space 为 Arc 共享：用户团队独占一份；内核团队由 [`init_kernel`] 注入。
///
/// tasks / held 自带 SpinLock（level 3）。**不变量：持本锁时绝不调用任何
/// space 方法**——与 Space.inner（level 2）只顺序获取、永不嵌套。
pub struct Team {
    /// 地址空间（窗口簿记持有全部分配的页）。Arc 共享：用户团队独占；
    /// 内核团队独占内核 Space。
    pub(crate) space: Arc<Space>,
    /// 成员簿记（弱引用条目；死条目在下次清理时摘除）。
    pub(crate) tasks: SpinLock<Vec<Weak<Task>>>,
    /// 域名字（程序身份；诊断用）。`Build` 时定型，不可改。
    pub(crate) name: Name,
    /// 引导线程（**未放行**，`Held`）——`Option` 把「至多一个」做成类型义务。
    /// `spawn` 填入、`Hatch` 摘出、`kill` 摘出。
    pub(crate) held: SpinLock<Option<Arc<Task>>>,
    /// 本域全局唯一标识（0 = 无效哨兵）。纯身份标识：诊断 + heir 内匹配，不承担
    /// 全局反查（授权走父 task 的 `heir` 表）。
    pub(crate) id: TeamId,
    /// 生我者的 task（弱引用，溯源；构造期定型，boot 顶级域 / 内核域 = 空 Weak）。
    /// 保持 Weak 是防环唯一边：`Task →(heir 强)→ Team →(sire 弱)→ Task`。
    pub(crate) sire: Weak<Task>,
    /// 本域默认执行入口（= 装载 ELF 的 `e_entry`，即镜像 `_start` VA）。
    /// `spawn` 的 `entry=0` 时用它。`OnceLock` 单次写，由 `Build` 写入。
    default_entry: OnceLock<usize>,
}

impl Team {
    /// 成员入簿。
    pub(crate) fn push_task(&self, task: &Arc<Task>) {
        self.tasks.lock().push(Arc::downgrade(task));
    }

    /// 为即将入簿的成员**预留**一格（产生路径不分配）。
    ///
    /// # Errors
    ///
    /// 簿记无法扩容（内存耗尽）→ `Err(())`。
    ///
    /// 与名册 / 就绪队列的预留同旨：把会分配的一步提到装配之前，失败时干净退回
    /// ——「生不出任务」应当是一个返回码，不是一次整机 halt。
    pub(crate) fn try_reserve_task(&self, slot: usize) -> Result<(), ()> {
        self.tasks
            .lock()
            .try_reserve(slot.saturating_add(1))
            .map_err(|_| ())
    }

    /// 清理簿记：摘除已退出线程与全部死条目。
    ///
    /// **不 upgrade**：弱引用提升会让存活条目的强计数瞬时 +1，与「强计数唯一
    /// （==1）」不变量撞车。改为纯指针比较：本线程条目按 Arc 数据指针摘除，
    /// 死条目按强计数为 0 摘除——全程不触碰强计数。
    pub(crate) fn prune_tasks(&self, exited: &Arc<Task>) {
        let exited_ptr = Arc::as_ptr(exited);
        self.tasks.lock().retain(|t| {
            // 死条目（strong == 0）：摘除（弱引用随条目 drop，底层分配随之释放）。
            // 注意此处读 Weak::strong_count 不做任何计数变更（纯 load）。
            if Weak::strong_count(t) == 0 {
                return false;
            }
            // 本线程条目：按数据指针摘除（不用 `upgrade` + `ptr_eq`——那会造成
            // 瞬时强计数提升）。
            !(Weak::as_ptr(t) == exited_ptr)
        });
    }

    /// 成员簿记的 `(条数, 死条数)`：`strong_count == 0` 的条目是**纯垃圾** —— 它
    /// 唯一的作用就是把那个任务的 `ArcInner`（`Kind::Task`，152 B）扣住不放，而载荷
    /// 早已析构。停机普查用（只读，不 upgrade——`upgrade` 会 +1 强计数，用它观测就
    /// 改变了被观测的事实）。
    ///
    /// **为什么这条普查非做不可**：`scheduler::core::rip` 把名册（全世界任务的弱引用）
    /// **放最后**清空，靠"最后一次弱引用归零 → 级联 drop"把外壳全带走。但**名册不是
    /// 唯一的弱引用容器** —— 本表与 [`Self::sire`] 也存 `Weak<Task>`，而
    /// [`KERNEL_TEAM`] 是 `OnceLock<Arc<Team>>` **永不析构**：它这张表里的死条目
    /// 永远不会被那次级联带走。
    pub(crate) fn weak_census(&self) -> (usize, usize) {
        let g = self.tasks.lock();
        (
            g.len(),
            g.iter().filter(|w| Weak::strong_count(w) == 0).count(),
        )
    }

    /// `held`（未放行引导线程，**强引用**）里那枚还在不在。
    pub(crate) fn held_live(&self) -> bool {
        self.held
            .lock()
            .as_ref()
            .is_some_and(|t| Arc::strong_count(t) > 0)
    }

    /// `sire`（建域者的弱引用）是否还指着活对象（空 `Weak` 恒 false）。
    pub(crate) fn sire_live(&self) -> bool {
        self.sire.strong_count() > 0
    }

    /// `sire` 是否是**一枚非空、但对象已死的弱引用** —— 那正是"扣住外壳"的形态：
    /// 空 `Weak`（`Weak::new()`）不占任何分配，而死弱引用会把那个任务的 `ArcInner`
    /// 扣到团队析构。
    ///
    /// **判据不能写成 `!as_ptr().is_null()`**：`Weak::new()` 的指针是**悬垂值**不是
    /// 空值，那样写会把"从未定过 sire"的内核团队报成"扣着一个死对象"（实测：普查里
    /// 恒报 `死弱引用 1`）。故与一枚新建的空 `Weak` 逐址比较。
    pub(crate) fn sire_armed(&self) -> bool {
        self.sire.as_ptr() != Weak::<Task>::new().as_ptr()
    }

    /// 成员簿记快照（cull 遍历用：快照后放锁，锁外逐条处理）。
    pub(crate) fn tasks_snapshot(&self) -> Vec<Weak<Task>> {
        self.tasks.lock().clone()
    }

    /// 本团队产出任务 builder（后续 `.name/.entry/.args/.stack/.hold/.spawn`
    /// 链式构造任务）。
    pub fn task(self: &Arc<Self>) -> TaskBuilder {
        TaskBuilder::new(self.clone())
    }

    /// 记下引导线程（未放行）。`Spawn` 产 Held 时调用。
    pub(crate) fn hold(&self, task: &Arc<Task>) {
        *self.held.lock() = Some(task.clone());
    }

    /// 摘出引导线程（`Hatch` / `kill` 用）；空则 None。
    pub(crate) fn take_held(&self) -> Option<Arc<Task>> {
        self.held.lock().take()
    }

    /// 域名字（诊断）。
    pub(crate) fn name(&self) -> Name {
        self.name
    }

    /// 本域默认执行入口（`spawn` 的 `entry=0` 时取）。未设（内核域）→ 0。
    pub(crate) fn default_entry(&self) -> usize {
        self.default_entry.get().copied().unwrap_or(0)
    }

    /// 写入默认执行入口（`Build` 装载后调用；单次写）。
    pub(crate) fn set_default_entry(&self, va: usize) {
        let _ = self.default_entry.set(va);
    }

    /// 溯源：生我者的 task id（boot 顶级域 / 内核域 → None）。
    /// 这是「不可伪造的父身份源」——由内核在建域时强制，非父自愿告知。
    pub(crate) fn sire(&self) -> Option<usize> {
        self.sire.upgrade().map(|t| t.ident.id)
    }
}

/// 团队构建器：把已装载程序的地址空间容器化为团队。
pub struct TeamBuilder {
    space: Space,
    sire: Weak<Task>,
    name: Name,
}

impl TeamBuilder {
    /// 接收已装载程序的 Space（owned；此后 Space 归团队）。
    pub fn new(space: Space) -> TeamBuilder {
        TeamBuilder {
            space,
            sire: Weak::new(),
            name: Name::new("team").expect("default team name"),
        }
    }

    /// 定生我者（boot 顶级域 / 内核域默认空 Weak）。构造期定型：sire 不可后改。
    pub fn sire(mut self, sire: Weak<Task>) -> TeamBuilder {
        self.sire = sire;
        self
    }

    /// 定域名字（程序身份；`Build` 用清单名）。
    pub fn name(mut self, name: Name) -> TeamBuilder {
        self.name = name;
        self
    }

    /// 容器化：包 Arc<Space> + 建空簿记，返回团队句柄。
    ///
    /// **血缘闭合**：sire 非空 ⇒ 立即推进 sire.heir（强持有）。见文件头 K1。
    pub fn spawn(self) -> Arc<Team> {
        let id = alloc_team_id();
        let team = Arc::new(Team {
            space: Arc::new(self.space),
            tasks: SpinLock::new_level(Level::L3, Vec::new()),
            name: self.name,
            held: SpinLock::new_level(Level::L3, None),
            id,
            sire: self.sire,
            default_entry: OnceLock::new(),
        });
        if let Some(sire) = team.sire.upgrade() {
            sire.adopt(team.clone());
        }
        #[cfg(feature = "audit")]
        register(&team);
        team
    }
}

/// **全部团队的弱引用**（普查用；仅 audit）。
///
/// 为什么需要它：`ArcInner<Task>` 的最后一门是**弱引用**，而全仓的 `Weak<Task>`
/// 容器除名册（`rip` 清空）之外还有 `Team.tasks` / `Team.sire` —— 那些弱引用随**团队
/// 析构**才消失。停机普查若只看内核团队（唯一永不析构的那个）就会**瞎**：真凶可能是
/// 任何一个"还活着"的团队。本表让普查能走遍全部团队，逐个报出"死弱引用"有几条。
///
/// 代价与安全：只存 `Weak`（不给团队续命），条目数 = 本次开机建立过的团队数（个位数），
/// 且每次普查顺带把死条目剔掉。**仅 audit 档存在**：产品档不需要它。
#[cfg(feature = "audit")]
static TEAMS: OnceLock<SpinLock<Vec<Weak<Team>>>> = OnceLock::new();

#[cfg(feature = "audit")]
fn teams() -> &'static SpinLock<Vec<Weak<Team>>> {
    TEAMS.get_or_init(|| SpinLock::new_level(Level::L3, Vec::new()))
}

/// 登记一个团队（构造点调用；仅 audit）。
#[cfg(feature = "audit")]
pub(crate) fn register(team: &Arc<Team>) {
    teams().lock().push(Arc::downgrade(team));
}

/// **逐团队弱引用普查**：`(活团队数, 抄不下的多余额, 死条目总数, 首 4 个 (团队 id, tasks 死, sire 死))`。
///
/// 顺带把**已死团队的条目**从登记表里剔掉（那些条目自己也占着团队外壳）。
///
/// # 锁纪律（第一版在这里撞了 lockdep）
///
/// `TEAMS` 与 `Team.tasks` **都是 L3**：持前者再取后者即"同层嵌套"，lockdep 当场
/// 报违规（`lock/depend.rs:112`）—— 而且是在关机钩子里炸，整轮报 panic。故分两段：
/// ① 持 `TEAMS` 锁时只做"剔除死条目 + 抄出活团队的强引用"，随即放锁；
/// ② 出锁后逐个团队读 `tasks`（每次只有一把 L3 在手）。
#[cfg(feature = "audit")]
pub(crate) fn weak_census_all() -> (usize, usize, usize, [(TeamId, usize, bool); 4]) {
    /// 一次能普查的团队数上限（团队是"每个程序一个"，量级个位；超出只记数）。
    const CAP: usize = 24;
    let mut live_refs: [Option<Arc<Team>>; CAP] = [const { None }; CAP];
    let mut live = 0usize;
    let mut more = 0usize;
    {
        let mut g = teams().lock();
        g.retain(|w| match w.upgrade() {
            None => false,
            Some(t) => {
                if live < CAP {
                    live_refs[live] = Some(t);
                    live += 1;
                } else {
                    more += 1;
                }
                true
            }
        });
    }
    let mut dead_total = 0usize;
    let mut head = [(TeamId::new(0), 0usize, false); 4];
    let mut n = 0usize;
    for slot in live_refs.iter_mut().take(live) {
        let Some(t) = slot.take() else { continue };
        let (_, dead_tasks) = t.weak_census();
        let dead_sire = !t.sire_live() && t.sire_armed();
        dead_total += dead_tasks + usize::from(dead_sire);
        if (dead_tasks > 0 || dead_sire) && n < 4 {
            head[n] = (t.id, dead_tasks, dead_sire);
            n += 1;
        }
    }
    (live, more, dead_total, head)
}

/// 内核团队单例（拥有内核地址空间；内核任务挂此团队）。
pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();

/// 把内核地址空间封包进内核团队单例（恰好一次）。
pub(crate) fn init_kernel(space: Arc<Space>) -> &'static Arc<Team> {
    KERNEL_TEAM.get_or_init(|| {
        let id = alloc_team_id();
        let t = Arc::new(Team {
            space,
            tasks: SpinLock::new_level(Level::L3, Vec::new()),
            name: Name::new("kernel").expect("kernel team name"),
            held: SpinLock::new_level(Level::L3, None),
            id,
            sire: Weak::new(),
            default_entry: OnceLock::new(),
        });
        #[cfg(feature = "audit")]
        register(&t);
        t
    })
}

/// 内核团队访问器（宽容形）：未注入 → None，调用方自行降级。
pub fn kernel() -> Option<&'static Arc<Team>> {
    KERNEL_TEAM.get()
}

/// 全局团队 id 序列（自 1；0 = 无效哨兵）。
static NEXT_TEAM_ID: AtomicUsize = AtomicUsize::new(1);

/// 分配一个新 TeamId（自 1 递增；0 = 无效哨兵）。
pub(crate) fn alloc_team_id() -> TeamId {
    TeamId::new(NEXT_TEAM_ID.fetch_add(1, Ordering::Relaxed))
}

// ── 装载错误（UnitError）──────────────────────────────────────

/// 镜像拼装结果错误（parse / build / load 任一步失败）。
///
/// 三步失败坍缩成一个变体：内核原语只把「成 / 不成」透给用户态（TeamId vs
/// 负码 `-6 BadImage`），具体失败步由 `erra` 上下文（annotate 链）留痕，无需细分
/// 枚举在 ABI 上传。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitError {
    /// parser / SpaceBuilder / loader 任一步失败（不落，无脏域）。
    Load,
}
