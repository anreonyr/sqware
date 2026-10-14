use alloc::sync::Arc;
use alloc::vec::Vec;
use core::mem::MaybeUninit;

#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PhysAddr, VirtAddr};
use crate::core::memory::flat::Frame;
use crate::core::memory::nest::{Access, NestError, RegionId};
use crate::core::work::unit::life::Life;
use crate::core::work::unit::space::{Memory, Space};
use crate::core::work::unit::team::{Team, kernel};
use crate::machine::layout::{HART_FRAME_BASE, IMAGE_BASE, TASK_STACK_SIZE};
use crate::machine::switcher::context::TrapContext;
use crate::machine::sync::{Level, SpinLock};

use super::ident::TaskMemory;
use super::ident::{TaskIdent, next_id};
use super::prepared::PreparedTask;
use super::state::TaskState;
use super::{MAX_ARGS, Task};

#[cfg(debug_assertions)]
static FAIL_PREPARATION: AtomicUsize = AtomicUsize::new(0);

#[cfg(debug_assertions)]
pub(crate) fn fail_preparation_at(stage: usize) {
    FAIL_PREPARATION.store(stage, Ordering::Relaxed);
}

#[cfg(debug_assertions)]
static FAIL_INVALIDATION: AtomicUsize = AtomicUsize::new(0);

#[cfg(debug_assertions)]
pub(crate) fn fail_invalidation_at(stage: usize) {
    assert!(crate::testing());
    FAIL_INVALIDATION.store(stage, Ordering::Relaxed);
}

fn invalidation_checkpoint(_stage: usize, _memory: &Memory) {
    #[cfg(debug_assertions)]
    if FAIL_INVALIDATION
        .compare_exchange(_stage, 0, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        crate::core::memory::nest::identity::fail_next_shootdown(_memory.asid());
    }
}

#[cfg(debug_assertions)]
static FAIL_ROLLBACK_FENCE: AtomicBool = AtomicBool::new(false);

#[cfg(debug_assertions)]
pub(crate) fn fail_rollback_fence() {
    assert!(crate::testing());
    FAIL_ROLLBACK_FENCE.store(true, Ordering::Relaxed);
}

#[inline]
fn preparation_checkpoint(_stage: usize) -> Result<(), NestError> {
    #[cfg(debug_assertions)]
    if FAIL_PREPARATION
        .compare_exchange(_stage, 0, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        return Err(NestError::OutOfMemory);
    }
    Ok(())
}

fn write_args(space: &Space, at: VirtAddr, args: &[usize]) {
    let mut done = 0usize;
    while done < args.len() {
        let va = at + done * size_of::<usize>();
        let pa = space
            .memory
            .physical_at(va)
            .expect("stack page materialized before args write");
        let page_rest = PAGE_SIZE - (va.as_usize() % PAGE_SIZE);
        let n = core::cmp::min((args.len() - done) * size_of::<usize>(), page_rest)
            / size_of::<usize>();
        let dst = pa.as_usize() as *mut usize;
        for i in 0..n {
            unsafe { core::ptr::write_volatile(dst.add(i), args[done + i]) };
        }
        done += n;
    }
}

/// 装配一个任务：起一个外壳、栈与参数，再交给 `PreparedTask` 等着公开。
pub struct TaskBuilder {
    team: Arc<Team>,
    entry: VirtAddr,
    args: Vec<usize>,
    stack: usize,
}

impl TaskBuilder {
    pub fn new(team: Arc<Team>) -> TaskBuilder {
        let entry = match team.default_entry() {
            0 => IMAGE_BASE,
            e => VirtAddr::from_raw(e),
        };
        TaskBuilder {
            team,
            entry,
            args: Vec::new(),
            stack: TASK_STACK_SIZE,
        }
    }

    pub fn args(mut self, args: Vec<usize>) -> TaskBuilder {
        debug_assert!(args.len() <= MAX_ARGS, "args 超过 MAX_ARGS");
        self.args = args;
        self
    }

    pub fn entry(mut self, entry: VirtAddr) -> TaskBuilder {
        self.entry = entry;
        self
    }

    pub fn stack(mut self, size: usize) -> TaskBuilder {
        self.stack = size;
        self
    }

    pub fn hold(self) -> Result<Arc<Task>, NestError> {
        if !self.team.ready() {
            return Err(NestError::WidenDenied);
        }
        self.prepare()?.publish(|| {})
    }

    pub(crate) fn prepare(self) -> Result<PreparedTask, NestError> {
        let stack_size = self
            .stack
            .max(1)
            .checked_next_multiple_of(PAGE_SIZE)
            .filter(|size| size.checked_add(crate::machine::layout::TASK_STACK_GUARD).is_some())
            .ok_or(NestError::NoRegion)?;
        let id = next_id()?;

        let mut memory = TaskMemory {
            space: self.team.space.clone(),
            stack: None,
            frame: None,
            guard: None,
        };
        let slot = self.team.space.memory.allocate_high(
            RegionId::Normal,
            stack_size + crate::machine::layout::TASK_STACK_GUARD,
        )?;
        let (guard, body) = self
            .team
            .space
            .memory
            .split_reservation(slot, crate::machine::layout::TASK_STACK_GUARD)?;
        memory.guard = Some(guard);
        let stack_body_top = body.start().as_usize() + body.size();
        let mut frames = Vec::new();
        if frames.try_reserve_exact(stack_size / PAGE_SIZE).is_err() {
            self.team.space.memory.release_reservation(body)?;
            return Err(NestError::OutOfMemory);
        }
        for _ in 0..stack_size / PAGE_SIZE {
            match Frame::allocate() {
                Ok(frame) => frames.push(frame),
                Err(error) => {
                    self.team.space.memory.release_reservation(body)?;
                    return Err(NestError::from(error));
                }
            }
        }
        let rw = Access::READ | Access::WRITE;
        invalidation_checkpoint(1, &self.team.space.memory);
        memory.stack = Some(self.team.space.memory.map_frames(body, frames, rw, rw)?);
        preparation_checkpoint(1)?;

        let mut frame_pages = Vec::new();
        frame_pages
            .try_reserve_exact(1)
            .map_err(|_| NestError::OutOfMemory)?;
        let frame = Frame::allocate()?;
        let frame_pa = PhysAddr::from_raw(frame.physical_address());
        frame_pages.push(frame);
        let reservation = self.team.space.memory.allocate_low(RegionId::Kernel, PAGE_SIZE)?;
        let frame_va = reservation.start();
        invalidation_checkpoint(2, &self.team.space.memory);
        memory.frame = Some(self.team.space.memory.map_frames(
            reservation,
            frame_pages,
            rw,
            rw,
        )?);
        preparation_checkpoint(2)?;

        let count = self.args.len();
        let args_at = stack_body_top - count * size_of::<usize>();
        if count > 0 {
            write_args(&self.team.space, VirtAddr::from_raw(args_at), &self.args);
        }
        let sp = VirtAddr::wrap(args_at & !0xF);

        let satp = self.team.space.memory.satp();
        let spp = self.team.space.kind().spp();
        let frame = unsafe { &mut *(frame_pa.as_usize() as *mut TrapContext) };
        unsafe {
            let ktc = kernel()
                .expect("kernel team not initialized")
                .space
                .memory
                .physical_at(HART_FRAME_BASE)
                .expect("kernel frame not mapped")
                .as_usize() as *const TrapContext;
            frame.init(
                &*ktc,
                satp,
                spp,
                self.entry,
                sp,
                (VirtAddr::wrap(args_at), count),
                frame_pa,
                frame_va,
            );
        }

        let alloc = crate::core::memory::flat::allocator::hybrid::allocator();
        let ident: Arc<TaskIdent> = unsafe {
            let ident = crate::tag!(
                Task,
                Arc::try_new_in(
                    TaskIdent::new(
                        id,
                        self.team.clone(),
                        frame_pa,
                        #[cfg(debug_assertions)]
                        frame_va,
                        memory,
                    ),
                    alloc,
                )
            )
            .map_err(|_| NestError::OutOfMemory)?;
            let (ptr, _alloc) = Arc::into_raw_with_allocator(ident);
            Arc::from_raw(ptr)
        };
        #[cfg(debug_assertions)]
        if FAIL_ROLLBACK_FENCE.swap(false, Ordering::Relaxed) {
            crate::core::memory::nest::identity::fail_next_shootdown(self.team.space.memory.asid());
        }
        preparation_checkpoint(3)?;
        let life = Life::try_new().map_err(|_| NestError::OutOfMemory)?;
        preparation_checkpoint(4)?;
        let state = Arc::try_new(SpinLock::new_level(Level::UnitState, TaskState::held()))
            .map_err(|_| NestError::OutOfMemory)?;
        let slot: Arc<MaybeUninit<Task>> = unsafe {
            let slot = crate::tag!(Task, Arc::<Task, _>::try_new_uninit_in(alloc))
                .map_err(|_| NestError::OutOfMemory)?;
            let (ptr, _) = Arc::into_raw_with_allocator(slot);
            Arc::from_raw(ptr)
        };
        Ok(PreparedTask::new(ident, life, state, slot))
    }
}
