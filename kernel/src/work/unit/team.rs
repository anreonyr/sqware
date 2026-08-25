// 团队（进程容器）— 持有地址空间 + 成员簿记。
//
// 生命周期：最后一个线程退出 → Arc<Team> 归零 → 团队回收；内核团队为
// 'static 单例，唯一拥有内核地址空间，永不回收。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

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
    pub fn spawn(self) -> Arc<Team> {
        Arc::new(Team {
            space: Arc::new(self.space),
            tasks: SpinLock::new_level(Level::L3, Vec::new()),
            elftable: self.elftable,
        })
    }
}

/// 内核团队单例（拥有内核地址空间；内核任务挂此团队）。
pub(crate) static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();

/// 把内核地址空间封包进内核团队单例（恰好一次）。
pub(crate) fn init_kernel(space: Arc<Space>) -> &'static Arc<Team> {
    KERNEL_TEAM.get_or_init(|| {
        Arc::new(Team {
            space,
            tasks: SpinLock::new_level(Level::L3, Vec::new()),
            elftable: crate::work::unit::elftable::kernel_table().map(Arc::new),
        })
    })
}

/// 内核团队访问器（宽容形）：未注入 → None，调用方自行降级。
pub fn kernel() -> Option<&'static Arc<Team>> {
    KERNEL_TEAM.get()
}
