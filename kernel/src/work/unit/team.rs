// 团队（进程容器）— 持有地址空间 + 成员簿记。
//
// 生命周期：最后一个线程退出 → Arc<Team> 归零 → 团队回收；内核团队为
// 'static 单例，唯一拥有内核地址空间，永不回收。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use hashbrown::HashMap;
use ubi::TeamId;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::unit::space::Space;

use super::elftable::ElfTable;
use super::task::{Task, TaskBuilder};

/// 团队（进程）— 共享地址空间的线程容器。
///
/// tasks 为成员簿记（弱引用，无强环），生命周期仍由引用计数决定。
///
/// space 为 Arc 共享：用户团队独占一份；内核团队由 [`init_kernel`] 注入。
///
/// tasks 自带 SpinLock（level 3）。**不变量：持本锁时绝不调用任何 space 方法**
/// ——与 Space.inner（level 2）只顺序获取、永不嵌套。
///
/// 血缘（域生域）：`sire` 是生我的那个 task（弱引用，溯源）——`spawn_team` 建的
/// 子域才有；boot 顶级域 / 内核域 `sire = None`。`heir` 是我生的子域（弱引用，
/// 级联回收入口）。两者都弱引用，不撑命、无弧。
pub struct Team {
    /// 地址空间（窗口簿记持有全部分配的页）。Arc 共享：用户团队独占；
    /// 内核团队独占内核 Space。
    pub(crate) space: Arc<Space>,
    /// 成员簿记（弱引用条目；死条目在下次清理时摘除）。
    pub(crate) tasks: SpinLock<Vec<Weak<Task>>>,
    /// 本团队程序的符号表（内核团队 = 内核表；用户团队 = 装载时构建）。None = 未建。
    pub(crate) elftable: Option<Arc<ElfTable>>,
    /// 本域全局唯一标识（0 = 无效哨兵；`spawn_team`/`TeamBuilder` 分配并登记）。
    pub(crate) id: TeamId,
    /// 生我者的 task（弱引用；`spawn_team` 建的子域才 set，boot 顶级域不 set）。
    /// `OnceLock`：单次写（血缘定型）；读经 `get()`。
    pub(crate) sire: OnceLock<Weak<Task>>,
    /// 我生的子域（弱引用；级联回收从 `heir` 递归全杀）。`sire`/`heir` 全 Weak 防环。
    pub(crate) heir: SpinLock<Vec<Weak<Team>>>,
    /// 本域默认执行入口（= 装载 ELF 的 `e_entry`，即镜像 `_start` VA）。
    /// `spawn_team` 子域 set；`spawn_task` 的 `entry=0` 时用它。`OnceLock` 单次写。
    pub(crate) default_entry: OnceLock<usize>,
}

impl Team {
    /// 成员入簿。
    pub(crate) fn push_task(&self, task: &Arc<Task>) {
        self.tasks.lock().push(Arc::downgrade(task));
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

    /// 本团队产出任务 builder（后续 `.name/.entry/.arg/.closure/.spawn` 链式构造任务）。
    pub fn task(self: &Arc<Self>) -> TaskBuilder {
        TaskBuilder::new(self.clone())
    }

    /// 血缘：记录生我者的 task（`spawn_team` 建子域时调用；boot 顶级域不设，默认 None）。
    pub(crate) fn set_sire(&self, sire: Weak<Task>) {
        // OnceLock 单次写：血缘定型后不可改。若已被设（重复做 sire），静默忽略。
        let _ = self.sire.set(sire);
    }

    /// 本域默认执行入口（`spawn_team` 装载时设；供 `spawn_task` 的 `entry=0` 用）。
    pub(crate) fn set_default_entry(&self, entry: usize) {
        let _ = self.default_entry.set(entry);
    }

    /// 本域默认执行入口（`spawn_task` 的 `entry=0` 时取）。未设（boot 顶级域）→ 0。
    pub(crate) fn default_entry(&self) -> usize {
        self.default_entry.get().copied().unwrap_or(0)
    }
}

/// 团队构建器：把已装载程序的地址空间容器化为团队。
pub struct TeamBuilder {
    space: Space,
    elftable: Option<Arc<ElfTable>>,
}

impl TeamBuilder {
    /// 接收已装载程序的 Space（owned；此后 Space 归团队）。
    pub fn new(space: Space) -> TeamBuilder {
        TeamBuilder {
            space,
            elftable: None,
        }
    }

    /// 绑定本团队程序的符号表（可选；装载后由调用方传入）。
    pub fn elftable(mut self, elftable: Option<Arc<ElfTable>>) -> TeamBuilder {
        self.elftable = elftable;
        self
    }

    /// 容器化：包 Arc<Space> + 建空簿记，返回团队句柄。
    /// 分配 TeamId 并登记到全局表（boot 顶级域 / spawn_team 子域都登记，供
    /// `spawn_task` 按 id 解 team）。`sire`/`heir` 默认空（boot 顶级域无血缘；
    /// spawn_team 子域的血缘由调用方另行设置）。
    pub fn spawn(self) -> Arc<Team> {
        let id = alloc_team_id();
        let team = Arc::new(Team {
            space: Arc::new(self.space),
            tasks: SpinLock::new_level(Level::L3, Vec::new()),
            elftable: self.elftable,
            id,
            sire: OnceLock::new(),
            heir: SpinLock::new_level(Level::L3, Vec::new()),
            default_entry: OnceLock::new(),
        });
        register_team(&team);
        team
    }
}

/// 内核团队单例（拥有内核地址空间；内核任务挂此团队）。
pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();

/// 把内核地址空间封包进内核团队单例（恰好一次）。
pub(crate) fn init_kernel(space: Arc<Space>) -> &'static Arc<Team> {
    KERNEL_TEAM.get_or_init(|| {
        let id = alloc_team_id();
        let team = Arc::new(Team {
            space,
            tasks: SpinLock::new_level(Level::L3, Vec::new()),
            elftable: None,
            id,
            sire: OnceLock::new(),
            heir: SpinLock::new_level(Level::L3, Vec::new()),
            default_entry: OnceLock::new(),
        });
        register_team(&team);
        team
    })
}

/// 内核团队访问器（宽容形）：未注入 → None，调用方自行降级。
pub fn kernel() -> Option<&'static Arc<Team>> {
    KERNEL_TEAM.get()
}

// ── TeamId 登记表 ─────────────────────────────────────────

/// 全局团队 id 序列（自 1；0 = 无效哨兵）。
static NEXT_TEAM_ID: AtomicUsize = AtomicUsize::new(1);

/// TeamId → Weak<Team> 全局表（不撑命：team 死则条目随 Weak drop）。
fn team_table() -> &'static SpinLock<HashMap<usize, Weak<Team>>> {
    static T: OnceLock<SpinLock<HashMap<usize, Weak<Team>>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 分配一个新 TeamId（自 1 递增；0 = 无效哨兵）。
pub(crate) fn alloc_team_id() -> TeamId {
    TeamId::new(NEXT_TEAM_ID.fetch_add(1, Ordering::Relaxed))
}

/// 把已持 `id` 的团队登记到全局表（供 `spawn_task` 按 id 解 team）。
pub(crate) fn register_team(team: &Arc<Team>) {
    team_table().lock().insert(team.id.get(), Arc::downgrade(team));
}

/// 按 TeamId 查团队（Weak 升级；已死 / 未登记 → None）。
pub(crate) fn lookup_team(id: TeamId) -> Option<Arc<Team>> {
    team_table().lock().get(&id.get()).and_then(|w| w.upgrade())
}
