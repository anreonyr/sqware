// 团队（进程容器）— 持有地址空间 + 成员簿记。
//
// Team = Space 容器：地址空间（Arc 共享）+ 成员簿记（弱引用）。
//
// 生命周期：最后一个线程退出 → Arc<Team> 归零 → 团队回收；内核团队（kernel）
// 为 'static 单例，共享全局 KERNEL_SPACE，永不回收。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use crate::lock::{OnceLock, SpinLock};
use crate::memory::manager::space::{Space, kernel_space};

use super::task::{Task, TaskBuilder};

/// 团队（进程）— 共享地址空间的线程容器。
///
/// tasks 为成员簿记（弱引用，无强环——线程由各 hart 的 running/starved/blocked/
/// reaped 容器强持有），多核阶段用于团队视角的负载判断；生命周期仍由引用计数
/// 决定（最后一个线程退出 → Arc<Team> 归零 → 团队回收）。
///
/// space 为 Arc 共享：用户团队独占一份；内核团队（kernel）与全局 KERNEL_SPACE
/// 共享同一份（'static 引用，永不回收）。
///
/// 多核下 per-hart 调度锁不提供跨 hart 互斥，故 tasks 自带 SpinLock
/// （level 3）。**不变量：持本锁时绝不调用任何 space 方法**——push_task /
/// prune_tasks 是纯 Vec 操作，与 Space.inner（level 2）只顺序获取、永不嵌套。
pub struct Team {
    /// 地址空间（窗口簿记持有全部分配的页）。Arc 共享：用户团队独占；
    /// 内核团队与 KERNEL_SPACE 共享同一份。
    pub(crate) space: Arc<Space>,
    /// 成员簿记（弱引用条目；死条目在下次清理时摘除）。
    pub(crate) tasks: SpinLock<Vec<Weak<Task>>>,
}

impl Team {
    /// 成员入簿（scheduler::push 入队收尾调用；不调 space 方法）。
    pub(crate) fn push_task(&self, task: &Arc<Task>) {
        self.tasks.lock().push(Arc::downgrade(task));
    }

    /// 清理簿记：摘除已退出线程与全部死条目（弱引用无所有权，滞留仅占条目）。
    pub(crate) fn prune_tasks(&self, exited: &Arc<Task>) {
        self.tasks.lock().retain(|t| match t.upgrade() {
            // 已回收的死条目（strong_count == 0）与本线程条目一并摘除
            Some(a) => !Arc::ptr_eq(&a, exited),
            None => false,
        });
    }

    /// 本团队产出任务 builder（后续 `.name/.entry/.arg/.closure/.spawn` 链式构造任务）。
    pub fn task(self: &Arc<Self>) -> TaskBuilder {
        TaskBuilder::new(self.clone())
    }
}

/// 团队构建器：把已装载程序的地址空间容器化为团队（W2：只建容器，不含任务）。
///
/// 调用链：loader::load(&space, program) → TeamBuilder::new(space).spawn()。
/// 容器化不分配新资源（包 Arc<Space> + 建空簿记），故 spawn 无错误路径。
pub struct TeamBuilder {
    space: Space,
}

impl TeamBuilder {
    /// 接收已装载程序的 Space（owned；此后 Space 归团队）。
    pub fn new(space: Space) -> TeamBuilder {
        TeamBuilder { space }
    }

    /// 容器化：包 Arc<Space> + 建空簿记，返回团队句柄。
    pub fn spawn(self) -> Arc<Team> {
        Arc::new(Team {
            space: Arc::new(self.space),
            tasks: SpinLock::new(Vec::new()),
        })
    }
}

/// 内核团队单例：与全局 KERNEL_SPACE 共享同一份空间（'static 引用，永不回收）。
/// 内核任务（kthread 式）挂此团队：SPP=1 运行于 S 态。
pub fn kernel() -> &'static Arc<Team> {
    static KERNEL_TEAM: OnceLock<Arc<Team>> = OnceLock::new();
    KERNEL_TEAM.get_or_init(|| {
        let space = kernel_space()
            .as_ref()
            .expect("kernel space not initialized")
            .clone();
        Arc::new(Team {
            space,
            tasks: SpinLock::new(Vec::new()),
        })
    })
}
