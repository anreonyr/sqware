// 团队（进程容器）— 持有地址空间 + 成员簿记 + 血缘（sire）。
//
// 生命周期：最后一个线程退出 → Arc<Team> 归零 → 团队回收；内核团队为
// 'static 单例，唯一拥有内核地址空间，永不回收。
//
// 血缘：`sire`（生我者）在构造期定型；`heir`（我生）挂在 **Task** 上（见
// task.rs）——强持有子域，既是撑命源，也是 `spawn_task` 的授权凭证，还是
// `doom` 级联的遍历源。三者合一，无独立全局表。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use ubi::{Spawnee, TeamId};

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
pub struct Team {
    /// 地址空间（窗口簿记持有全部分配的页）。Arc 共享：用户团队独占；
    /// 内核团队独占内核 Space。
    pub(crate) space: Arc<Space>,
    /// 成员簿记（弱引用条目；死条目在下次清理时摘除）。
    pub(crate) tasks: SpinLock<Vec<Weak<Task>>>,
    /// 本团队程序的符号表（内核团队 = 内核表；用户团队 = 装载时构建）。None = 未建。
    pub(crate) elftable: Option<Arc<ElfTable>>,
    /// 本域全局唯一标识（0 = 无效哨兵）。纯身份标识：诊断 + heir 内匹配，不承担
    /// 全局反查（授权走父 task 的 `heir` 表）。
    pub(crate) id: TeamId,
    /// 生我者的 task（弱引用，溯源；构造期定型，boot 顶级域 / 内核域 = 空 Weak）。
    /// 保持 Weak 是防环唯一边：`Task →(heir 强)→ Team →(sire 弱)→ Task`。
    ///
    /// 溯源占位：当前无读者（授权走父侧 `heir`，不查本字段）；留待诊断场景
    /// （panic 现场打印血缘树 / 孤儿域溯源）消费。显式 `allow(dead_code)` 标记
    /// 「有意保留、尚未接入」。
    #[allow(dead_code)]
    pub(crate) sire: Weak<Task>,
    /// 本域默认执行入口（= 装载 ELF 的 `e_entry`，即镜像 `_start` VA）。
    /// `spawn_team` 子域 set；`spawn_task` 的 `entry=0` 时用它。`OnceLock` 单次写。
    default_entry: OnceLock<usize>,
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

    /// 成员簿记快照（cull 遍历用：快照后放锁，锁外逐条处理）。
    pub(crate) fn tasks_snapshot(&self) -> Vec<Weak<Task>> {
        self.tasks.lock().clone()
    }

    /// 本团队产出任务 builder（后续 `.name/.entry/.arg/.closure/.spawn` 链式构造任务）。
    pub fn task(self: &Arc<Self>) -> TaskBuilder {
        TaskBuilder::new(self.clone())
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
    sire: Weak<Task>,
}

impl TeamBuilder {
    /// 接收已装载程序的 Space（owned；此后 Space 归团队）。
    pub fn new(space: Space) -> TeamBuilder {
        TeamBuilder {
            space,
            elftable: None,
            sire: Weak::new(),
        }
    }

    /// 绑定本团队程序的符号表（可选；装载后由调用方传入）。
    pub fn elftable(mut self, elftable: Option<Arc<ElfTable>>) -> TeamBuilder {
        self.elftable = elftable;
        self
    }

    /// 定生我者（spawn_team 子域才设；boot 顶级域 / 内核域默认空 Weak）。
    /// 构造期定型：sire 不可后改。
    pub fn sire(mut self, sire: Weak<Task>) -> TeamBuilder {
        self.sire = sire;
        self
    }

    /// 容器化：包 Arc<Space> + 建空簿记，返回团队句柄。
    pub fn spawn(self) -> Arc<Team> {
        let id = alloc_team_id();
        Arc::new(Team {
            space: Arc::new(self.space),
            tasks: SpinLock::new_level(Level::L3, Vec::new()),
            elftable: self.elftable,
            id,
            sire: self.sire,
            default_entry: OnceLock::new(),
        })
    }
}

/// 内核团队单例（拥有内核地址空间；内核任务挂此团队）。
pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();

/// 把内核地址空间封包进内核团队单例（恰好一次）。
pub(crate) fn init_kernel(space: Arc<Space>) -> &'static Arc<Team> {
    KERNEL_TEAM.get_or_init(|| {
        let id = alloc_team_id();
        Arc::new(Team {
            space,
            tasks: SpinLock::new_level(Level::L3, Vec::new()),
            elftable: None,
            id,
            sire: Weak::new(),
            default_entry: OnceLock::new(),
        })
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

// ── 运行期装载（spawn_team）────────────────────────────────────

/// 镜像拼装结果错误（parse / build / load 任一步失败）。
///
/// 三步失败坍缩成一个变体：内核原语只把「成 / 不成」透给用户态（TeamId vs usize::MAX），
/// 具体失败步由 `erra` 上下文（annotate 链）留痕，无需细分枚举在 ABI 上传。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitError {
    /// parser / SpaceBuilder / loader 任一步失败（不落，无脏域）。
    Load,
}

/// Spawnee → 内嵌 ELF 字节（编译期 match；文件系统出现后此函数弃用）。
/// 独立自由函数（非 `impl Spawnee`——`elf()` 不能是 ubi 类型的 inherent impl；
/// 且 `USER_*` env 只在 `kernel/build.rs` 定义，取字节只能落在内核侧）。
fn spawnee_elf(which: Spawnee) -> &'static [u8] {
    match which {
        Spawnee::Lisp => include_bytes!(env!("USER_LISP")),
        Spawnee::Shell => include_bytes!(env!("USER_SHELL")),
        Spawnee::Back => include_bytes!(env!("USER_BACK")),
        Spawnee::Narrow => include_bytes!(env!("USER_NARROW")),
    }
}

/// 装载镜像成独立域（新 Space+Team，不产 task），挂血缘，成功返回 TeamId。
///
/// 血缘：`child.sire`（构造期已定）← `sire`；`sire.adopt(child)`（heir 强持有，
/// 撑命 + spawn_task 授权凭证 + doom 遍历源）。
///
/// # Errors
/// - `Load` — 拼装任一步失败（不落，无脏域）。
pub(crate) fn spawn_team(which: Spawnee, sire: &Arc<Task>) -> Result<TeamId, UnitError> {
    let (team, entry) = super::assemble(spawnee_elf(which), Arc::downgrade(sire))?;
    // 域记住默认执行入口（装载 ELF 的 e_entry；spawn_task 的 entry=0 时用它）。
    team.set_default_entry(entry.as_usize());
    // 血缘：sire 强持有子域（heir）。
    sire.adopt(team.clone());
    Ok(team.id)
}
