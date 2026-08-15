// 线程模型类型（阶段 A 延续）：Team（进程容器）→ Task（线程）两层。
//
//   Team   — 唯一 Space + 成员簿记（SpinLock<Vec<Weak<Task>>>，弱引用，无强环）
//   Task   — 可调度单元：Arc<Team> + 调度状态 + 自己的 trap 帧句柄（TrapFrame { va, pa }）
//
// 引用图无环：Task → Arc<Team>（强）、调度器 → Arc<Task>（强，current/ready）、
// Team → Weak<Task>（弱，簿记不参与生命周期）。Team 由它的线程持有：spawn 返回
// 的 Arc<Team> 只是构造期句柄，spawn 完线程即 drop；最后一个线程退出 → Arc<Team>
// 归零 → Team/Space（ASID + 全部帧）自动回收。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU8, Ordering};

use crate::lock::SpinLock;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::space::Space;

/// 任务状态（生命周期：就绪 ↔ 运行）。
///
/// 存为 AtomicU8 而非裸枚举字段：Task 经 Arc 共享，状态转换须经不可变
/// 引用完成——原子字段是满足 Sync 的最小载体。实际全部转换都在调度器锁内
/// 发生（单写者 + 锁内读取），原子性只是形式而非并发协议。
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// 就绪（在某 hart 的就绪队列中等待运行）。
    Ready = 0,
    /// 当前运行（恒为某 hart 的 current，不在任何队列）。
    Running = 1,
    /// 阻塞（在某个等待队列中——睡眠/信号量等；**不在**任何就绪队列，不可被 steal）。
    Blocked = 2,
}

/// trap 帧句柄 — 线程 trap 帧的薄引用。
///
/// 帧页由所属 Space 的 Frame 窗口子 Map **持有**（随线程退出回收），本句柄只
/// 携带 VA/PA 两个数：PA 供 restore 直接取帧，VA 供退出时按位归还窗口。
#[derive(Clone, Copy, Debug)]
pub struct TrapFrame {
    /// 帧在本空间中的虚拟地址（Frame 窗口分配，S-only）。
    pub(crate) va: VirtAddr,
    /// 帧物理地址（restore 的 a0）。
    pub(crate) pa: PhysAddr,
}

/// 线程 — 可调度单元：共享所属 Team 的地址空间，持有自己的 trap 帧。
///
/// 栈 / 堆 / 帧全部归 Team.space 的窗口簿记（Window 子 Map），Task 只持
/// trap 句柄与共享的 team 引用——无任何页所有权。
pub struct Task {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
    pub(crate) state: AtomicU8,
    pub(crate) team: Arc<Team>,
    pub(crate) trap: TrapFrame,
}

impl Task {
    /// 状态转移（状态机不变量）：非法转移直接 panic。
    ///
    /// 合法转移：
    ///   Ready → Running（调度器选上 / steal 迁移后运行）
    ///   Running → Ready（抢占 / 让出）
    ///   Running → Blocked（阻塞：进入等待队列，如睡眠）
    ///   Blocked → Ready（唤醒：回到就绪队列）
    pub(crate) fn transition(&self, next: TaskState) {
        let cur = self.state();
        let legal = matches!(
            (cur, next),
            (TaskState::Ready, TaskState::Running)
                | (TaskState::Running, TaskState::Ready)
                | (TaskState::Running, TaskState::Blocked)
                | (TaskState::Blocked, TaskState::Ready)
        );
        assert!(legal, "illegal task state transition: {cur:?} -> {next:?}");
        self.state.store(next as u8, Ordering::Relaxed);
    }

    pub(crate) fn state(&self) -> TaskState {
        match self.state.load(Ordering::Relaxed) {
            0 => TaskState::Ready,
            1 => TaskState::Running,
            2 => TaskState::Blocked,
            // 只会写入 TaskState 的合法编码，此处不可能到达
            _ => unreachable!("invalid task state: {}", self.state.load(Ordering::Relaxed)),
        }
    }
}

/// 团队（进程）— 共享地址空间的线程容器。
///
/// tasks 为成员簿记（弱引用，无强环——线程由各 hart 的 current/就绪队列强
/// 持有），多核阶段用于团队视角的负载判断；生命周期仍由引用计数决定（最后
/// 一个线程退出 → Arc<Team> 归零 → 团队回收）。
///
/// 多核下 per-hart 调度锁不再提供跨 hart 互斥，故 tasks 自带 SpinLock
/// （level 3）。**不变量：持本锁时绝不调用任何 space 方法**——push_task /
/// prune_tasks 是纯 Vec 操作，与 Space.inner（level 2）只顺序获取、永不嵌套
/// 持有（ABBA 防御，见 lock/mod.rs 层级注释）。
pub struct Team {
    /// 地址空间（窗口簿记持有全部分配的页）。
    pub(crate) space: Space,
    /// 成员簿记（弱引用条目；死条目在下次清理时摘除）。
    pub(crate) tasks: SpinLock<Vec<Weak<Task>>>,
}

impl Team {
    /// 成员入簿（调用方持 schedulers()[hart]——scheduler::enqueue 入队收尾；
    /// 不调 space 方法）。
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
}
