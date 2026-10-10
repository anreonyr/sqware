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
        use crate::work::unit::space::Permit;
        use alloc::sync::Arc;
        use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use env::{Permission, TaskId};
        pub struct Backing {
            busy: AtomicBool,
            reserved: AtomicUsize,
        }
        impl Backing {
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
            pub fn try_permit(&self, permission: Permission) -> Result<Arc<Permit>, ()> {
                Ok(Arc::new(Permit::new(permission)))
            }
        }
        pub struct Operation(Arc<Backing>);
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
                    backing: Arc<Backing>,
                }
                impl $name {
                    pub fn new(owner: TaskId) -> Arc<Self> {
                        Arc::new(Self {
                            owner,
                            live: AtomicBool::new(true),
                            backing: Backing::new(),
                        })
                    }
                    pub fn owner(&self) -> TaskId {
                        self.owner
                    }
                    pub fn alive(&self) -> bool {
                        self.live.load(Ordering::Acquire)
                    }
                    pub fn backing(&self) -> &Arc<Backing> {
                        &self.backing
                    }
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
        pub mod space {
            use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
            use env::Permission;
            pub struct Permit {
                live: AtomicBool,
                permission: AtomicU32,
            }
            impl Permit {
                pub fn new(permission: Permission) -> Self {
                    Self {
                        live: AtomicBool::new(true),
                        permission: AtomicU32::new(permission.bits()),
                    }
                }
                pub fn permission(&self) -> Permission {
                    if self.live.load(Ordering::Acquire) {
                        Permission::from_bits_retain(self.permission.load(Ordering::Relaxed))
                    } else {
                        Permission::empty()
                    }
                }
                pub fn narrow(&self, permission: Permission) {
                    self.permission.store(permission.bits(), Ordering::Relaxed);
                }
                pub fn invalidate(&self) {
                    self.live.store(false, Ordering::Release);
                }
            }
            pub struct Space;
            impl Space {
                pub fn with_shootdown<T>(&self, run: impl FnOnce(&mut Self) -> T) -> Result<T, ()> {
                    Ok(run(&mut Self))
                }
                pub fn narrow_token(
                    &mut self,
                    _: env::PieToken,
                    _: crate::memory::manager::entry::PteFlags,
                ) -> Result<(), ()> {
                    Ok(())
                }
            }
        }
        pub mod task {
            use super::gate::AnyPie;
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
            pub struct Team {
                pub space: super::space::Space,
            }
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
                            pies: SpinLock::new(Vec::new()), heirs: SpinLock::new(Vec::new()) },
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
                pub fn heir(&self, _: env::TeamId) -> Option<Team> {
                    None
                }
            }
        }
        pub use crate::gate;
    }
}

#[path = "../../src/work/unit/gate/mod.rs"]
pub mod gate;

pub use work::unit::{space, commit};

#[cfg(test)]
mod tests;
