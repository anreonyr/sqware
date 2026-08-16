// 线程模型类型（阶段 A 延续）：Team（进程容器）→ Task（线程）两层。
//
//   Team   — 唯一 Space + 成员簿记（SpinLock<Vec<Weak<Task>>>，弱引用，无强环）
//   Task   — 可调度单元：Arc<Team> + 调度状态 + 自己的 trap 帧句柄（TrapFrame { va, pa }）
//
// 引用图无环：Task → Arc<Team>（强）、调度器 → Arc<Task>（强，running/starved/
// blocked/reaped 恰好其一）、Team → Weak<Task>（弱，簿记不参与生命周期）。
// Team 由它的线程持有：spawn 返回的 Arc<Team> 只是构造期句柄，spawn 完线程即
// drop；最后一个线程退出 → Arc<Team> 归零 → Team/Space（ASID + 全部帧）自动回收。
//
// 状态是单一枚举 + 载荷：Running{ticks_left}（运行预算）/ Blocked{reason}（阻塞
// 原因）/ Starved（就绪）/ Reaped（僵尸）。阻塞原因与运行预算作为状态变体数据
// 放在任务上，外部队列（blocked / reaped）退化为纯 Arc<Task> 索引。
//
// 状态字段是普通字段（无原子）：所有状态变更都发生在「锁内 take/pop 出唯一
// Arc<Task> → Arc::get_mut 拿 &mut」路径上（见 scheduler::task_mut）——互斥由
// 锁 + 所有权保证，编译器强制；Task: Sync 仅意味着 &Task 可跨核只读共享。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use crate::lock::SpinLock;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::space::Space;

/// 任务状态：任务现在在哪 +（Running/Blocked 时）该状态特有的数据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// 正在执行（恒为某 hart 的 running，不在任何队列）：预算随 run 递减。
    /// 不变量：预算恒 ≥ 1（耗尽即转 Starved，不落盘 Running{0}）。
    Running { ticks_left: u32 },
    /// 已阻塞（在 blocked 容器中；不在任何就绪队列，不可被 steal）：原因在载荷。
    Blocked { reason: BlockReason },
    /// 已饥饿（预算耗尽，在 starved 容器等补给；被选中时重置满额预算）。
    Starved,
    /// 已收割（僵尸，在 reaped 容器等延迟回收；不在任何队列，任何核可回收）。
    Reaped,
}

/// Blocked 的载荷：阻塞原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// 睡眠：wake_at 到期由 unpark 唤醒。
    Park { wake_at: usize },
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
    /// 状态（含载荷）。普通字段：只有经 scheduler::task_mut 的 &mut 能改。
    pub(crate) state: TaskState,
    pub(crate) team: Arc<Team>,
    pub(crate) trap: TrapFrame,
}

impl Task {
    /// 状态变换（状态机不变量）：非法变换直接 panic。
    ///
    /// 合法变换：
    ///   Starved → Running（调度器选上 / steal 迁移后运行）
    ///   Running → Starved（预算耗尽轮转 / 主动让出）
    ///   Running → Blocked(原因)（阻塞：如睡眠）
    ///   Blocked(_) → Starved（唤醒：回到就绪容器）
    ///   Running → Reaped（退出：标记收割，延迟回收）
    pub(crate) fn transform(&mut self, next: TaskState) {
        let legal = matches!(
            (self.state, next),
            (TaskState::Starved, TaskState::Running { .. })
                | (TaskState::Running { .. }, TaskState::Starved)
                | (TaskState::Running { .. }, TaskState::Blocked { .. })
                | (TaskState::Blocked { .. }, TaskState::Starved)
                | (TaskState::Running { .. }, TaskState::Reaped)
        );
        assert!(
            legal,
            "illegal task state transform: {:?} -> {:?}",
            self.state, next
        );
        self.state = next;
    }

    pub(crate) fn state(&self) -> TaskState {
        self.state
    }

    /// 续跑：预算递减（Running → Running 仅载荷更新，不经状态机变换表）。
    pub(crate) fn dec_ticks_left(&mut self) {
        match self.state {
            TaskState::Running { ticks_left } => {
                debug_assert!(ticks_left >= 1, "Running 预算恒 ≥ 1");
                self.state = TaskState::Running {
                    ticks_left: ticks_left - 1,
                };
            }
            _ => unreachable!("dec_ticks_left 只对 Running 任务调用"),
        }
    }

    /// 阻塞唤醒时间（Blocked(Park) 才有值；unpark 从 blocked 队首读它）。
    pub(crate) fn wake_at(&self) -> Option<usize> {
        match self.state {
            TaskState::Blocked {
                reason: BlockReason::Park { wake_at },
            } => Some(wake_at),
            _ => None,
        }
    }
}

/// 团队（进程）— 共享地址空间的线程容器。
///
/// tasks 为成员簿记（弱引用，无强环——线程由各 hart 的 running/starved/blocked/
/// reaped 容器强持有），多核阶段用于团队视角的负载判断；生命周期仍由引用计数
/// 决定（最后一个线程退出 → Arc<Team> 归零 → 团队回收）。
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
}
