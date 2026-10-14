use alloc::sync::Arc;
use alloc::vec::Vec;
use core::mem::MaybeUninit;

use crate::core::memory::nest::NestError;
use crate::core::work::pend::Pend;
use crate::core::work::record::trace::{self, EventKind, RoomEvent};
use crate::core::work::room::scheduler;
use crate::core::work::unit::copies::Origin;
use crate::core::work::unit::life::Life;
use crate::core::work::unit::tally;
use crate::core::work::unit::team::{MAX_MEMBERS, Member};
use crate::machine::sync::SpinLock;

use super::{Task, TaskIdent, TaskState, TaskWeak};

/// 已装配好、尚未公开的任务：身份、生命周期线、状态机，与那块等着写入的外壳内存。
pub(crate) struct PreparedTask {
    ident: Arc<TaskIdent>,
    life: Arc<Life>,
    state: Arc<SpinLock<TaskState>>,
    slot: Option<Arc<MaybeUninit<Task>>>,
}

impl PreparedTask {
    pub(crate) fn new(
        ident: Arc<TaskIdent>,
        life: Arc<Life>,
        state: Arc<SpinLock<TaskState>>,
        slot: Arc<MaybeUninit<Task>>,
    ) -> PreparedTask {
        PreparedTask {
            ident,
            life,
            state,
            slot: Some(slot),
        }
    }

    /// 把这一份预备公开成团队里的一个成员：先按预留容量落座，再写上外壳、挂上
    /// 团队表与暂扣表。`commit` 是公开那一刻要回调的动作。
    pub(crate) fn publish(&mut self, commit: impl FnOnce()) -> Result<Arc<Task>, NestError> {
        let team = self.ident.team.clone();
        let receipts = team.receipts()?;
        let member = Arc::try_new(Member {
            id: self.ident.id,
            task: TaskWeak::empty(),
            node: Arc::downgrade(&team.life),
            state: self.state.clone(),
            life: self.life.clone(),
            receipts: SpinLock::new(receipts),
        })
        .map_err(|_| NestError::OutOfMemory)?;
        scheduler::roster::reserve_publication().map_err(|_| NestError::OutOfMemory)?;
        {
            let mut tasks = team.tasks.members.lock();
            if tasks.len() >= MAX_MEMBERS {
                return Err(NestError::OutOfMemory);
            }
            let mut held = team.tasks.held.lock();
            tasks.try_reserve(1).map_err(|_| NestError::OutOfMemory)?;
            let held_extra = tasks.len() + 1 - held.len();
            held.try_reserve(held_extra)
                .map_err(|_| NestError::OutOfMemory)?;
        }
        let _commit = crate::core::work::commit();
        if team.life.closed() {
            return Err(NestError::NoRegion);
        }
        let mut tasks = team.tasks.members.lock();
        let mut held = team.tasks.held.lock();
        if tasks.len() >= MAX_MEMBERS
            || tasks.len() == tasks.capacity()
            || held.capacity() < tasks.len() + 1
        {
            return Err(NestError::OutOfMemory);
        }
        let (task, ()) = scheduler::roster::publish(|| {
            commit();
            assert!(team.ready(), "publish: constructing team");
            let mut slot = self.slot.take().expect("prepare slot");
            Arc::get_mut(&mut slot)
                .expect("unique prepare slot")
                .write(Task {
                    ident: self.ident.clone(),
                    life: self.life.clone(),
                    state: self.state.clone(),
                    pend: Pend::new(),
                    heir: SpinLock::new(Vec::new()),
                });
            let task = unsafe { slot.assume_init() };
            let mut member = member;
            Arc::get_mut(&mut member).expect("private member").task =
                TaskWeak::stored(Arc::downgrade(&task), Origin::TeamTasks);
            tasks.push(member);
            held.push(task.clone());
            tally::push();
            (task, ())
        })
        .map_err(|_| NestError::OutOfMemory)?;

        drop(held);
        drop(tasks);

        trace::note(EventKind::Room(RoomEvent::Spawn {
            tid: task.ident.id.get(),
        }));
        Ok(task)
    }
}
