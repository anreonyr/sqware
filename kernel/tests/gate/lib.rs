//! Host concurrency tests compile the production gate modules. Task scheduling,
//! resource mappings and notifications are replaced; QEMU tests cover those.
#![allow(dead_code, unused_imports)]
#![feature(allocator_api)]
extern crate alloc;

mod lock {
    pub struct SpinLock<T>(std::sync::Mutex<T>);
    // Matches the kernel lock: raw capability handles stay inside locked records.
    unsafe impl<T> Sync for SpinLock<T> {}
    impl<T> SpinLock<T> {
        pub fn new(value: T) -> Self {
            Self(std::sync::Mutex::new(value))
        }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
        pub fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> {
            self.0.try_lock().ok()
        }
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
    pub mod manager {
        pub struct Map;
        impl Map {
            pub fn initial_range(&self) -> (addr::VirtAddr, usize) { (addr::VirtAddr::wrap(0), 4096) }
        }
        pub mod addr {
            #[derive(Clone, Copy, PartialEq, Eq)]
            pub struct VirtAddr(usize);
            impl VirtAddr { pub const fn wrap(value: usize) -> Self { Self(value) } }
        }
        pub mod entry {
            bitflags::bitflags! {
                #[derive(Clone, Copy)]
                pub struct PteFlags: usize { const R = 1; const W = 2; }
            }
        }
    }
}
mod work {
    pub mod room {
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
        use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use env::TaskId;
        pub struct OperationState {
            busy: AtomicBool,
            reserved: AtomicUsize,
        }
        impl OperationState {
            pub fn new() -> Arc<Self> {
                Arc::new(Self {
                    busy: AtomicBool::new(false),
                    reserved: AtomicUsize::new(0),
                })
            }
            pub fn operation(self: &Arc<Self>) -> Option<Operation> {
                self.busy
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .ok()
                    .map(|_| Operation(self.clone()))
            }
            pub fn reserved(&self) -> usize {
                self.reserved.load(Ordering::Relaxed)
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
                    pub fn operation(self: &Arc<Self>) -> Option<Operation> { self.control.operation() }
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
        meta!(PoleMeta);
        meta!(ToleMeta);
        impl PoleMeta {
            pub fn withdraw(&self, _: env::PieToken) {}
            pub fn release_grant(&self, _: env::PieToken) {}
            pub fn register_grant(&self, _: env::PieToken, _: alloc::sync::Arc<crate::gate::Grant>) -> Result<(), env::PieFail> {
                Ok(())
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
            pub fn seal(meta: &PoleMeta) {
                meta.seal();
            }
            pub fn shut(_: &PoleMeta, _: env::PieToken) -> Result<(), env::PieFail> {
                Ok(())
            }
            pub fn narrow(
                _: &PoleMeta,
                _: env::PieToken,
                _: crate::memory::manager::entry::PteFlags,
            ) -> Result<(), env::PieFail> {
                Ok(())
            }
        }
    }
    pub mod unit {
        pub fn commit() -> std::sync::MutexGuard<'static, ()> {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            LOCK.lock().unwrap()
        }
        pub mod task {
            use super::gate::{AnyPie, CreationRegistry};
            use crate::lock::SpinLock;
            use alloc::sync::{Arc, Weak};
            use alloc::vec::Vec;
            use core::sync::atomic::{AtomicUsize, Ordering};
            use env::{PieToken, TaskId};
            static NEXT: AtomicUsize = AtomicUsize::new(1);
            pub struct TaskIdent {
                pub id: TaskId,
            }
            #[derive(Clone, Copy)]
            pub enum TaskTag {
                Held,
                Doomed,
                Reaped,
            }
            pub enum TaskState {
                Held,
                Doomed { hart: Option<usize>, cause: env::ExitCause, reason: usize },
                Reaped,
            }
            pub struct Gate {
                serial: SpinLock<()>,
                pub version: AtomicUsize,
                pub pies: SpinLock<Vec<AnyPie>>,
                pub heirs: SpinLock<Vec<(PieToken, Weak<Task>, PieToken)>>,
                pub creations: CreationRegistry,
            }
            impl Gate {
                pub fn lock(&self) -> std::sync::MutexGuard<'_, ()> { self.serial.lock() }
            }
            pub struct Task {
                pub ident: TaskIdent,
                pub state: SpinLock<TaskState>,
                pub gate: Gate,
                life: Arc<()>,
            }
            // Mirrors the production Task bound for its kernel-owned handles.
            unsafe impl Send for Task {}
            pub struct Team;
            impl Team {
                pub fn operation(&self) -> Option<()> {
                    Some(())
                }
            }
            impl Task {
                pub fn new() -> Arc<Self> {
                    Arc::new(Self {
                        ident: TaskIdent {
                            id: TaskId::new(NEXT.fetch_add(1, Ordering::Relaxed)),
                        },
                        state: SpinLock::new(TaskState::Held),
                        gate: Gate { serial: SpinLock::new(()), version: AtomicUsize::new(0),
                            pies: SpinLock::new(Vec::new()), heirs: SpinLock::new(Vec::new()),
                            creations: CreationRegistry::new() },
                        life: Arc::new(()),
                    })
                }
                pub fn tag(&self) -> TaskTag {
                    match &*self.state.lock() {
                        TaskState::Held => TaskTag::Held,
                        TaskState::Doomed { .. } => TaskTag::Doomed,
                        TaskState::Reaped => TaskTag::Reaped,
                    }
                }
                pub fn life(&self) -> Weak<()> {
                    Arc::downgrade(&self.life)
                }
                pub fn clan(&self, _: env::TeamId) -> Option<Arc<Team>> {
                    None
                }
            }
        }
        pub use crate::gate;
    }
}

#[path = "../../src/work/unit/gate/mod.rs"]
pub mod gate;

pub use work::unit::commit;

#[cfg(test)]
mod tests;
