use alloc::sync::Arc;
use core::fmt;

use env::ExitCause as TaskExitCause;

use crate::core::work::room::messenger::{Ticket, WaitCall, WakeKey};
use crate::core::work::unit::task::Task;
use crate::machine::hart::HartId;

/// 任务状态机的轴：任一时刻只在一根轴上。`next` 是入链载荷，只挂在就绪 / 等待 /
/// 躯壳三种轴上（饥饿链、等待链、躯壳链共用同一个槽）。
pub enum TaskAxis {
    OnHart {
        hart: HartId,
        ticks_left: u32,
    },
    Ready {
        next: Option<Arc<Task>>,
    },
    Waiting {
        key: WakeKey,
        ticket: Ticket,
        next: Option<Arc<Task>>,
        wait: Option<WaitCall>,
    },
    Held,
    Parked,
    Doomed {
        hart: Option<HartId>,
        cause: TaskExitCause,
        reason: usize,
    },
    Reaped {
        cause: TaskExitCause,
        reason: usize,
        next: Option<Arc<Task>>,
    },
}

impl TaskAxis {
    /// 入链载荷槽：链只穿就绪 / 等待 / 躯壳三种轴，其余轴没有槽。
    pub(crate) fn chain(&mut self) -> &mut Option<Arc<Task>> {
        match self {
            TaskAxis::Ready { next }
            | TaskAxis::Waiting { next, .. }
            | TaskAxis::Reaped { next, .. } => next,
            other => unreachable!("链只穿就绪/等待/躯壳任务，实为 {other:?}"),
        }
    }

    /// 入链载荷是否为空：没有槽的轴恒为空。
    pub(crate) fn unlinked(&self) -> bool {
        match self {
            TaskAxis::Ready { next }
            | TaskAxis::Waiting { next, .. }
            | TaskAxis::Reaped { next, .. } => next.is_none(),
            _ => true,
        }
    }
}

impl fmt::Debug for TaskAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskAxis::OnHart { hart, ticks_left } => write!(
                f,
                "OnHart {{ hart: {}, ticks_left: {ticks_left} }}",
                hart.get()
            ),
            TaskAxis::Ready { next } => write!(f, "Ready {{ next: {} }}", next.is_some()),
            TaskAxis::Waiting {
                key, ticket, next, ..
            } => write!(
                f,
                "Waiting {{ key: {key:?}, ticket: {ticket:?}, next: {} }}",
                next.is_some()
            ),
            TaskAxis::Held => write!(f, "Held"),
            TaskAxis::Parked => write!(f, "Parked"),
            TaskAxis::Doomed {
                hart,
                cause,
                reason,
            } => write!(
                f,
                "Doomed {{ hart: {}, cause: {cause:?}, reason: {reason:#x} }}",
                hart.map_or(0, HartId::get)
            ),
            TaskAxis::Reaped { cause, reason, .. } => {
                write!(f, "Reaped {{ cause: {cause:?}, reason: {reason:#x} }}")
            }
        }
    }
}
