#![allow(dead_code, unused_imports)]
#![feature(allocator_api)]
extern crate alloc;
#[path = "fixture/exit.rs"]
pub mod exit_fixture;

#[path = "../../src/core/memory/address/count.rs"]
mod count;
#[path = "../../src/core/memory/address/index.rs"]
mod index;
#[path = "../../src/core/memory/address/offset.rs"]
mod offset;
#[path = "../../src/machine/sync/serial.rs"]
mod serial;
#[path = "fixture/flat.rs"]
pub(crate) mod flat_fixture;
#[path = "fixture/space.rs"]
pub(crate) mod space_fixture;
#[path = "fixture/pole.rs"]
mod pole_fixture;

mod core {
    pub(crate) use crate::{memory, work};
}
mod machine {
    pub(crate) use crate::lock as sync;
}

mod lock {
    pub(crate) use crate::serial::next_id;
    pub struct SpinLock<T>(std::sync::Mutex<T>);
    unsafe impl<T> Sync for SpinLock<T> {}
    impl<T> SpinLock<T> {
        pub const fn new(value: T) -> Self {
            Self(std::sync::Mutex::new(value))
        }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
        pub fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> {
            self.0.try_lock().ok()
        }
    }
    pub use std::sync::MutexGuard as SpinLockGuard;

    pub mod spin {
        pub use super::SpinLockGuard;
    }

    pub fn reserve_depend(_: usize) -> Result<(), ()> {
        let hook = HOOK.with(|slot| slot.borrow_mut().take());
        if let Some(hook) = hook { hook(); }
        Ok(())
    }
    thread_local! {
        static HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    }
    pub fn before_locking(hook: impl FnOnce() + 'static) {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }
}
mod memory {
    pub const PAGE_SIZE: usize = 4096;
    pub(crate) use crate::flat_fixture as flat;

    pub mod address {
        pub use crate::{count::PageCount, index::PageIndex, offset::PageOffset};
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct VirtAddr(usize);
        impl VirtAddr {
            pub const fn wrap(value: usize) -> Self {
                Self(value)
            }
        }
    }

    pub mod nest {
        pub struct View;
        impl View {
            pub fn initial_range(&self) -> (crate::core::memory::address::VirtAddr, usize) {
                (crate::core::memory::address::VirtAddr::wrap(0), 4096)
            }
        }
        bitflags::bitflags! {
            #[derive(Clone, Copy)]
            pub struct Access: usize { const READ = 1; const WRITE = 2; const EXECUTE = 4; }
        }
    }
}
mod work {
    pub use crate::grant;

    /// work 全域提交锁的宿主桩（内核那侧是 `crate::core::work::commit`）。
    pub fn commit() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap()
    }

    /// “这一步必须在提交段里做”的核内自检：宿主侧没有持有的抄本
    /// （这里的 `commit` 就是一把普通互斥锁），故只在核内校验。
    pub fn assert_held() {}

    pub mod room {
        pub mod scheduler {
            pub mod roster {
                use alloc::sync::{Arc, Weak};
                use crate::core::work::unit::task::Task;
                use env::TaskId;

                static ROSTER: std::sync::Mutex<Vec<Weak<Task>>> =
                    std::sync::Mutex::new(Vec::new());

                pub fn register(task: &Arc<Task>) {
                    let mut roster = ROSTER.lock().unwrap();
                    roster.retain(|known| known.strong_count() != 0);
                    roster.push(Arc::downgrade(task));
                }

                pub fn muster(id: TaskId) -> Option<Weak<Task>> {
                    ROSTER.lock().unwrap().iter().find_map(|known| {
                        known.upgrade()
                            .filter(|task| task.ident.id == id)
                            .map(|_| known.clone())
                    })
                }
            }
        }
        pub mod messenger {
            use alloc::sync::Weak;
            use env::TaskId;
            pub enum WakeKey {
                Pies { task: TaskId },
                Inspect { task: TaskId, token: usize },
            }
            pub fn signal(_: WakeKey) -> Result<(), ()> {
                Ok(())
            }
            pub fn wake(_: WakeKey, _: &Weak<()>) -> Result<(), ()> {
                Ok(())
            }
        }
    }
    pub mod mail {
        use alloc::sync::Arc;
        use ::core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use env::TaskId;
        pub struct OperationState {
            busy: AtomicBool,
            reserved: AtomicUsize,
            busy_attempts: AtomicUsize,
        }
        impl OperationState {
            pub fn new() -> Arc<Self> {
                Arc::new(Self {
                    busy: AtomicBool::new(false),
                    reserved: AtomicUsize::new(0),
                    busy_attempts: AtomicUsize::new(0),
                })
            }
            pub fn operation(self: &Arc<Self>) -> Option<Operation> {
                if self
                    .busy_attempts
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok()
                {
                    return None;
                }
                self.busy
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .ok()
                    .map(|_| Operation(self.clone()))
            }
            pub fn reserved(&self) -> usize {
                self.reserved.load(Ordering::Relaxed)
            }
            pub fn stage_for(&self, team: usize) {
                self.reserved.store(team, Ordering::Relaxed);
            }
            pub fn busy_for(&self, attempts: usize) {
                self.busy_attempts.store(attempts, Ordering::Relaxed);
            }
            pub fn busy_remaining(&self) -> usize {
                self.busy_attempts.load(Ordering::Relaxed)
            }
        }
        pub struct Operation(Arc<OperationState>);
        impl Drop for Operation {
            fn drop(&mut self) {
                self.0.busy.store(false, Ordering::Release);
            }
        }
        macro_rules! meta {
            ($name:ident) => {
                pub struct $name {
                    owner: TaskId,
                    live: AtomicBool,
                    control: Arc<OperationState>,
                }
                impl $name {
                    pub fn new(owner: TaskId) -> Arc<Self> {
                        Arc::new(Self {
                            owner,
                            live: AtomicBool::new(true),
                            control: OperationState::new(),
                        })
                    }
                    pub fn owner(&self) -> TaskId {
                        self.owner
                    }
                    pub fn alive(&self) -> bool {
                        self.live.load(Ordering::Acquire)
                    }
                    pub fn reserved(&self) -> usize { self.control.reserved() }
                    pub fn has_cells(&self) -> bool {
                        false
                    }
                    pub fn seal(&self) {
                        self.live.store(false, Ordering::Release);
                    }
                }
            };
        }
        meta!(HoleMeta);
        meta!(NoleMeta);
        meta!(ToleMeta);
        pub use crate::pole_fixture::PoleMeta;
        #[derive(Clone)]
        pub enum Mail {
            Hole(Arc<HoleMeta>),
            Pole(Arc<PoleMeta>),
            Nole(Arc<NoleMeta>),
            Tole(Arc<ToleMeta>),
        }
        impl Mail {
            pub fn kind(&self) -> env::PieKind {
                match self {
                    Mail::Hole(_) => env::PieKind::Hole,
                    Mail::Pole(_) => env::PieKind::Pole,
                    Mail::Nole(_) => env::PieKind::Nole,
                    Mail::Tole(_) => env::PieKind::Tole,
                }
            }
            pub fn owner(&self) -> TaskId {
                match self {
                    Mail::Hole(meta) => meta.owner(),
                    Mail::Pole(meta) => meta.owner(),
                    Mail::Nole(meta) => meta.owner(),
                    Mail::Tole(meta) => meta.owner(),
                }
            }
            pub fn alive(&self) -> bool {
                match self {
                    Mail::Hole(meta) => meta.alive(),
                    Mail::Pole(meta) => meta.alive(),
                    Mail::Nole(meta) => meta.alive(),
                    Mail::Tole(meta) => meta.alive(),
                }
            }
            pub fn seal(&self) {
                match self {
                    Mail::Hole(meta) => meta.seal(),
                    Mail::Pole(meta) => meta.seal(),
                    Mail::Nole(meta) => meta.seal(),
                    Mail::Tole(meta) => meta.seal(),
                }
            }
            pub fn hole(&self) -> Option<Arc<HoleMeta>> {
                match self {
                    Mail::Hole(meta) => Some(meta.clone()),
                    _ => None,
                }
            }
            pub fn pole(&self) -> Option<Arc<PoleMeta>> {
                match self {
                    Mail::Pole(meta) => Some(meta.clone()),
                    _ => None,
                }
            }
            pub fn nole(&self) -> Option<Arc<NoleMeta>> {
                match self {
                    Mail::Nole(meta) => Some(meta.clone()),
                    _ => None,
                }
            }
            pub fn tole(&self) -> Option<Arc<ToleMeta>> {
                match self {
                    Mail::Tole(meta) => Some(meta.clone()),
                    _ => None,
                }
            }
            pub fn same(&self, other: &Mail) -> bool {
                match (self, other) {
                    (Mail::Hole(a), Mail::Hole(b)) => Arc::ptr_eq(a, b),
                    (Mail::Pole(a), Mail::Pole(b)) => Arc::ptr_eq(a, b),
                    (Mail::Nole(a), Mail::Nole(b)) => Arc::ptr_eq(a, b),
                    (Mail::Tole(a), Mail::Tole(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                }
            }
            pub fn new_grant(
                &self,
                permission: env::Permission,
                token: env::PieToken,
                window: Option<(
                    crate::core::memory::address::PageOffset,
                    crate::core::memory::address::PageCount,
                )>,
            ) -> Result<Arc<crate::core::work::grant::Grant>, env::PieFail> {
                let grant = crate::core::work::grant::Grant::try_new_window(permission, window)?;
                if let Mail::Pole(meta) = self {
                    meta.register_grant(token, grant.clone())?;
                }
                Ok(grant)
            }
            pub fn release_grant(&self, token: env::PieToken) {
                if let Mail::Pole(meta) = self {
                    meta.release_grant(token);
                }
            }
        }
        pub mod hole {
            pub use super::HoleMeta;
            pub fn seal(meta: &HoleMeta) {
                meta.seal();
            }
        }
        pub mod nole {
            pub use super::NoleMeta;
            pub fn seal(meta: &NoleMeta) {
                meta.seal();
            }
        }
        pub mod tole {
            pub use super::ToleMeta;
            pub fn seal(meta: &ToleMeta) {
                meta.seal();
            }
        }
        pub mod pole {
            pub use super::PoleMeta;
            pub struct PoleOperation(super::Operation);
            impl PoleOperation {
                pub fn acquire(meta: &alloc::sync::Arc<PoleMeta>) -> Option<Self> {
                    meta.control.operation().map(Self)
                }
            }
            pub fn seal(meta: &PoleMeta) {
                meta.seal();
            }
            pub fn shut(_: &PoleMeta, _: env::PieToken) -> Result<(), env::PieFail> {
                Ok(())
            }
            pub fn narrow(
                _: &PoleMeta,
                _: env::PieToken,
                _: crate::core::memory::nest::Access,
            ) -> Result<(), env::PieFail> {
                Ok(())
            }
        }
    }
    pub mod unit {
        pub mod team {
            pub use crate::exit_fixture as exit;
        }
        pub(crate) use crate::space_fixture as space;
        pub mod task {
            use crate::pend::Pend;
            use crate::machine::sync::SpinLock;
            use alloc::sync::{Arc, Weak};
            use ::core::sync::atomic::{AtomicUsize, Ordering};
            use env::TaskId;
            static NEXT: AtomicUsize = AtomicUsize::new(1);
            pub struct TaskIdent {
                pub id: TaskId,
            }
            pub struct TaskState {
                axis: TaskAxis,
                paused: bool,
            }
            pub enum TaskAxis {
                OnHart { hart: usize, ticks_left: u32 },
                Ready { next: Option<Arc<Task>> },
                Waiting { key: usize, ticket: usize, next: Option<Arc<Task>> },
                Held,
                Parked,
                Doomed { hart: Option<usize>, cause: env::ExitCause, reason: usize },
                Reaped { cause: env::ExitCause, reason: usize, next: Option<Arc<Task>> },
            }
            impl TaskState {
                pub fn held() -> TaskState {
                    TaskState { axis: TaskAxis::Held, paused: false }
                }
                pub fn doom(&mut self, hart: Option<usize>, cause: env::ExitCause, reason: usize) {
                    self.axis = TaskAxis::Doomed { hart, cause, reason };
                    self.paused = false;
                }
            }
            pub struct Task {
                pub ident: TaskIdent,
                pub state: SpinLock<TaskState>,
                pub(crate) pend: Pend,
                life: Arc<()>,
            }
            unsafe impl Send for Task {}
            pub struct Team;
            impl Team {
                pub fn operation(&self) -> Option<()> {
                    Some(())
                }
            }
            impl Task {
                pub fn new() -> Arc<Self> {
                    let task = Arc::new(Self {
                        ident: TaskIdent {
                            id: TaskId::new(NEXT.fetch_add(1, Ordering::Relaxed)),
                        },
                        state: SpinLock::new(TaskState::held()),
                        pend: Pend::new(),
                        life: Arc::new(()),
                    });
                    crate::core::work::room::scheduler::roster::register(&task);
                    task
                }
                pub fn dead(&self) -> bool {
                    matches!(
                        self.state.lock().axis,
                        TaskAxis::Doomed { .. } | TaskAxis::Reaped { .. }
                    )
                }
                pub fn life(&self) -> Weak<()> {
                    Arc::downgrade(&self.life)
                }
                pub fn close_controls(&self) {}
                pub fn clan(&self, _: env::TeamId) -> Option<Arc<Team>> {
                    None
                }
            }
        }
        pub use crate::pend;
    }
}

#[path = "../../src/core/work/grant.rs"]
pub mod grant;

#[path = "../../src/core/work/pend/mod.rs"]
pub mod pend;

pub use work::commit;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod source_tests;
