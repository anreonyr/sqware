use ::core::cell::UnsafeCell;

use alloc::boxed::Box;
use alloc::format;
#[cfg(debug_assertions)]
use alloc::vec::Vec;

use super::OnceLock;
use crate::machine::hart;

mod level;
#[cfg(debug_assertions)]
mod error;
#[cfg(debug_assertions)]
mod held;
#[cfg(debug_assertions)]
mod held_cell;
#[cfg(debug_assertions)]
mod held_set;

pub use level::Level;

#[cfg(debug_assertions)]
pub use error::DepInitError;

#[cfg(debug_assertions)]
use held_cell::HeldCell;
#[cfg(debug_assertions)]
use held_set::HeldSet;

#[cfg(debug_assertions)]
pub(crate) fn report(what: &'static str, lock: usize, caller: usize) -> ! {
    crate::core::memory::allocator::portal::switch(
        crate::core::memory::allocator::portal::Backend::Spare,
    );
    let held = held().expect("depend: report outside collected set");
    let mut msg = format!("[depend] {what}: {lock:#x} ({lock:#x})",);
    if caller != 0 {
        msg.push_str(&format!("\n  caller: {:#x} ({caller:#x})", caller,));
    }
    if held.slots.is_empty() {
        msg.push_str("\n  held: (none)");
    } else {
        let max = held.max_level();
        msg.push_str("\n  held:");
        for slot in held.slots.iter() {
            let lv = slot
                .level
                .map(|l| format!("{l:?}"))
                .unwrap_or_else(|| "exempt".into());
            msg.push_str(&format!(
                "\n    {:#x} {:#x} ({lv}){} acquired at {:#x} ({:#x})",
                slot.addr,
                slot.addr,
                if slot.level == max {
                    "  <-- max held"
                } else {
                    ""
                },
                slot.caller,
                slot.caller
            ));
        }
        msg.push_str("\n  rule: new level must exceed max(held); violation");
    }
    panic!("{msg}");
}

#[cfg(debug_assertions)]
static POOL: OnceLock<&'static [HeldCell]> = OnceLock::new();

#[cfg(debug_assertions)]
pub(crate) fn init(hart_count: usize) -> Result<(), DepInitError> {
    if POOL.get().is_some() {
        return Err(DepInitError::AlreadyInit);
    }
    let n = hart_count.clamp(1, crate::machine::layout::MAX_HART_SLOTS);
    let mut cells =
        Box::<[HeldCell]>::try_new_uninit_slice(n).map_err(|_| DepInitError::OutOfMemory)?;
    for index in 0..n {
        let set = match HeldSet::new() {
            Ok(set) => set,
            Err(error) => {
                for cell in &mut cells[..index] {
                    unsafe {
                        cell.assume_init_drop();
                    }
                }
                return Err(error);
            }
        };
        cells[index].write(HeldCell(UnsafeCell::new(set)));
    }
    let pool: &'static [HeldCell] = Box::leak(unsafe { cells.assume_init() });
    POOL.set(pool).map_err(|_| DepInitError::AlreadyInit)
}

#[cfg(debug_assertions)]
fn held() -> Option<&'static mut HeldSet> {
    let pool = POOL.get()?;
    let h = hart::hart_id();
    if h.get() >= pool.len() {
        panic!("[depend] hart {h} out of pool ({} slots)", pool.len());
    }
    Some({
        let this = &pool[h.get()];
        unsafe { &mut *this.0.get() }
    })
}

#[cfg(debug_assertions)]
pub(crate) fn check(addr: usize, level: Option<Level>, caller: usize) {
    let Some(held) = held() else {
        return;
    };
    if held.contains(addr) {
        report("recursive acquisition", addr, caller);
    }
    if let Some(lv) = level
        && held.max_level().is_some_and(|m| lv <= m)
    {
        report("lock-order level violation", addr, caller);
    }
}

#[cfg(debug_assertions)]
pub(crate) fn acquire(addr: usize, level: Option<Level>, caller: usize) {
    let Some(held) = held() else {
        return;
    };
    if held.push(addr, level, caller).is_err() {
        report("held set overflow", addr, caller);
    }
}

#[cfg(debug_assertions)]
pub(crate) fn release(addr: usize) {
    let Some(held) = held() else {
        return;
    };
    if held.remove(addr).is_err() {
        report("release of unheld lock", addr, 0);
    }
}

pub(crate) fn reserve(_additional: usize) -> Result<(), ()> {
    #[cfg(debug_assertions)]
    {
        let _trap = unsafe { super::trap::TrapGuard::save() };
        let Some(set) = held() else {
            return Ok(());
        };
        let wanted = set.slots.len().checked_add(_additional).ok_or(())?;
        if set.slots.capacity() >= wanted {
            return Ok(());
        }
        let mut slots = Vec::new();
        slots.try_reserve_exact(wanted).map_err(|_| ())?;
        let set = held().expect("depend initialized");
        slots.extend_from_slice(&set.slots);
        let old = ::core::mem::replace(&mut set.slots, slots);
        drop(old);
    }
    Ok(())
}
