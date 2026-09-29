use core::cell::UnsafeCell;

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;

use super::OnceLock;
use crate::hart;
use crate::platform::machine;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Level {
    Scheduler = 1,
    Space = 2,
    L3 = 4,
    Asid = 5,
    Frame = 6,
    Block = 7,
    Tally = 9,
    Spare = 10,
}

#[cfg(debug_assertions)]
pub(crate) fn report(what: &'static str, lock: usize, caller: usize) -> ! {
    crate::memory::allocator::portal::switch(crate::memory::allocator::portal::Backend::Spare);
    let held = held().expect("depend: report outside collected set");
    let mut msg = format!("[depend] {what}: {lock:#x} ({lock:#x})",);
    if caller != 0 {
        msg.push_str(&format!("\n  caller: {:#x} ({caller:#x})", caller,));
    }
    if held.len == 0 {
        msg.push_str("\n  held: (none)");
    } else {
        let max = held.max_level();
        msg.push_str("\n  held:");
        for slot in &held.slots[..held.len] {
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
const MAX_HELD: usize = 8;

#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
struct Held {
    addr: usize,
    level: Option<Level>,
    caller: usize,
}

#[cfg(debug_assertions)]
struct HeldSet {
    len: usize,
    slots: [Held; MAX_HELD],
}

#[cfg(debug_assertions)]
impl HeldSet {
    const fn new() -> HeldSet {
        HeldSet {
            len: 0,
            slots: [Held {
                addr: 0,
                level: None,
                caller: 0,
            }; MAX_HELD],
        }
    }

    fn max_level(&self) -> Option<Level> {
        self.slots[..self.len].iter().filter_map(|h| h.level).max()
    }

    fn contains(&self, addr: usize) -> bool {
        self.slots[..self.len].iter().any(|h| h.addr == addr)
    }

    fn push(&mut self, addr: usize, level: Option<Level>, caller: usize) -> Result<(), ()> {
        if self.len >= MAX_HELD {
            return Err(());
        }
        self.slots[self.len] = Held {
            addr,
            level,
            caller,
        };
        self.len += 1;
        Ok(())
    }

    fn remove(&mut self, addr: usize) -> Result<(), ()> {
        for i in 0..self.len {
            if self.slots[i].addr == addr {
                self.slots.copy_within(i + 1..self.len, i);
                self.len -= 1;
                return Ok(());
            }
        }
        Err(())
    }
}

#[cfg(debug_assertions)]
struct HeldCell(UnsafeCell<HeldSet>);

// SAFETY: 每核只写自己那份
#[cfg(debug_assertions)]
unsafe impl Send for HeldCell {}
#[cfg(debug_assertions)]
unsafe impl Sync for HeldCell {}

#[cfg(debug_assertions)]
static POOL: OnceLock<&'static [HeldCell]> = OnceLock::new();

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepInitError {
    OutOfMemory,
    AlreadyInit,
}

#[cfg(debug_assertions)]
pub(crate) fn init(hart_count: usize) -> Result<(), DepInitError> {
    if POOL.get().is_some() {
        return Err(DepInitError::AlreadyInit);
    }
    let n = hart_count.clamp(1, crate::layout::MAX_HART_SLOTS);
    let cells: Vec<HeldCell> = (0..n)
        .map(|_| HeldCell(UnsafeCell::new(HeldSet::new())))
        .collect();
    let pool: &'static [HeldCell] = Box::leak(cells.into_boxed_slice());
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
        // SAFETY: 每核 + SIE 关
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
