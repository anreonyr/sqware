//! The fake device on the other end of the queue's register window.
//!
//! `queue.rs` addresses MMIO as `base + offset` through `regs::read32/write32/...`, so
//! `base` here is a real buffer: the register file. Everything the queue programs -
//! the two base registers, the two tails, the two CSRs, the fault counter - lands in
//! this file, and everything the queue reads back comes out of it. The device side
//! owns the rest: it takes commands off the ring, writes the completion record each
//! `IOFENCE.C` asks for, and answers the queue CSRs.
//!
//! The device runs on [`clock`], the same call `queue.rs` makes to bound its waits:
//! a wait loop is exactly when a device would get to run, so the tick and the clock
//! are one thing here. What the tick cannot do is change a register the driver is
//! about to read back in the same call - see `tests.rs` for that boundary.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use crate::iommu::pages::{PAGE_SIZE, Pages};
use crate::iommu::regs;

/// The pages the device reaches: the queue's three (`queue.rs` gets them by index)
/// and the register file the queue programs.
pub const COMMANDS: usize = 0;
pub const FAULTS: usize = 1;
pub const FENCE: usize = 2;
pub const WINDOW: usize = 3;

/// Rings of the two queues the device runs, in entries.
const COMMAND_ENTRIES: u32 = 2 << 7;
const COMMAND_BYTES: usize = 16;
const FAULT_ENTRIES: u32 = 2 << 6;
const FAULT_BYTES: usize = 32;
/// `IOFENCE.C`: opcode 2 with `AV` (bit 10) asks for a completion record whose value
/// is carried in `dword0[63:32]`.
const IOFENCE_C: u64 = 2;
const NOTIFY: u64 = 1 << 10;

/// One command the device took off the command ring.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Command {
    /// The ring index it sat at, counted the way the queue counts its tail.
    pub index: u32,
    pub dword0: u64,
    pub dword1: u64,
}

impl Command {
    pub fn opcode(self) -> u64 {
        self.dword0 & 0x7F
    }

    /// The completion value the device must write (`dword0[63:32]`).
    pub fn completion(self) -> u32 {
        (self.dword0 >> 32) as u32
    }

    /// Where the command's ring entry is, in bytes from the start of the page.
    pub fn ring_slot(self) -> usize {
        ring_entry(self.index)
    }
}

/// Where ring index `index` sits inside the command ring.
fn ring_entry(index: u32) -> usize {
    (index % COMMAND_ENTRIES) as usize * COMMAND_BYTES
}

struct State {
    /// Where the device has consumed up to (the queue reads this back as `CQH`).
    head: u32,
    /// A held device leaves submitted commands on the ring: no completion arrives.
    held: bool,
    consumed: Vec<Command>,
    /// Commands whose completion address was outside the fence page, so the device
    /// wrote nothing (they show up as waits that never end).
    refused: usize,
}

pub struct Device {
    pages: Pages,
    state: RefCell<State>,
}

impl Device {
    /// Take the window and make this the device the fake clock ticks.
    pub fn new() -> Rc<Self> {
        ON_CLOCK.with(|hook| *hook.borrow_mut() = None);
        let device = Rc::new(Self {
            pages: Pages::take(4),
            state: RefCell::new(State {
                head: 0,
                held: false,
                consumed: Vec::new(),
                refused: 0,
            }),
        });
        CURRENT.with(|current| *current.borrow_mut() = Some(device.clone()));
        device
    }

    /// The register window the queue is pointed at.
    pub fn base(&self) -> usize {
        self.pages.physical(WINDOW)
    }

    pub fn pages(&self) -> &Pages {
        &self.pages
    }

    pub fn fence_page(&self) -> usize {
        self.pages.physical(FENCE)
    }

    /// Where the device has consumed up to.
    pub fn head(&self) -> u32 {
        self.state.borrow().head
    }

    /// The queue's tail, straight out of the register file (the doorbell it rang).
    pub fn tail(&self) -> u32 {
        regs::read32(self.base(), regs::CQT)
    }

    /// Every command the device has taken off the ring, in order.
    pub fn consumed(&self) -> Vec<Command> {
        self.state.borrow().consumed.clone()
    }

    /// How many completions the device refused to write (bad address in the command).
    pub fn refused(&self) -> usize {
        self.state.borrow().refused
    }

    /// Fetch the command sitting at ring index `index`, the way the device fetches it.
    pub fn command(&self, index: u32) -> Command {
        let at = self.pages.at(COMMANDS, ring_entry(index)) as *const u64;
        // SAFETY: the command page is a real buffer of this device, and an entry lies
        // inside it; the queue wrote both doublewords before ringing the doorbell.
        let (dword0, dword1) = unsafe {
            (
                core::ptr::read_volatile(at),
                core::ptr::read_volatile(at.add(1)),
            )
        };
        Command {
            index,
            dword0,
            dword1,
        }
    }

    /// The address the command names for its completion record (`dword1` is set the way
    /// the queue sets it: address >> 2).
    pub fn completion_address(&self, command: Command) -> usize {
        (command.dword1 << 2) as usize
    }

    /// Where in the fence page the command's completion lands.
    pub fn slot(&self, command: Command) -> usize {
        self.completion_address(command) - self.fence_page()
    }

    /// Read the completion slot the way the device (or a late duplicate) left it.
    pub fn completion(&self, slot: usize) -> u32 {
        // SAFETY: `slot` is inside the fence page; the fixture's callers take it from
        // the command under discussion.
        unsafe { core::ptr::read_volatile(self.pages.at(FENCE, slot) as *const u32) }
    }

    /// A completion store into a slot of the fence page: the device answering, or an
    /// older completion arriving late.
    pub fn complete(&self, slot: usize, value: u32) {
        // SAFETY: as `completion`.
        unsafe { core::ptr::write_volatile(self.pages.at(FENCE, slot) as *mut u32, value) };
    }

    /// The device's answer to the driver's `ENABLE|MEM_FAULT` write: both queues are up.
    ///
    /// This is the handshake the driver asks for written into the register file after
    /// the fact - `tests.rs` says why it cannot be made to happen inside `enable`.
    pub fn rise(&self) {
        for csr in [regs::CQCSR, regs::FQCSR] {
            let seen = regs::read32(self.base(), csr);
            let done = seen | regs::QUEUE_ENABLE | regs::QUEUE_ON;
            regs::write32(
                self.base(),
                csr,
                done & !(regs::QUEUE_MEM_FAULT | regs::QUEUE_BUSY),
            );
        }
    }

    /// Whether the device takes commands off the ring. A held device is one that never
    /// answers, which is what leaves a fence outstanding.
    pub fn hold(&self, held: bool) {
        self.state.borrow_mut().held = held;
    }

    /// One record in the fault queue: the count the device has written, and the header
    /// word of the newest record. The queue reads back the count and the record the
    /// count points at, so this is a two-record ring exercise, not a single store.
    pub fn push_fault(&self, count: u32, header: u64) {
        assert!(count >= 1, "a fault ring counts from one");
        let slot = (count as usize - 1) % FAULT_ENTRIES as usize * FAULT_BYTES;
        let at = self.pages.at(FAULTS, slot) as *mut u64;
        // SAFETY: the record lies inside the fault page, 8-byte aligned.
        unsafe { core::ptr::write_volatile(at, header) };
        regs::write32(self.base(), regs::FQT, count);
    }

    /// One step of the device: take the commands the queue rang in for.
    pub fn tick(&self) {
        let mut state = self.state.borrow_mut();
        if state.held {
            return;
        }
        let tail = regs::read32(self.base(), regs::CQT);
        while state.head != tail {
            let command = self.command(state.head);
            if command.opcode() == IOFENCE_C && command.dword0 & NOTIFY != 0 {
                let at = self.completion_address(command);
                if at >= self.fence_page() && at + 4 <= self.fence_page() + PAGE_SIZE {
                    // SAFETY: the address was checked to be inside the fence page, and
                    // it is the driver's own completion record.
                    unsafe {
                        core::ptr::write_volatile(at as *mut u32, command.completion());
                    }
                } else {
                    state.refused += 1;
                }
            }
            state.head = state.head.wrapping_add(1);
            regs::write32(self.base(), regs::CQH, state.head);
            state.consumed.push(command);
        }
    }
}

thread_local! {
    static CLOCK: Cell<u64> = const { Cell::new(0) };
    static STEP: Cell<u64> = const { Cell::new(1_000_000) };
    static CURRENT: RefCell<Option<Rc<Device>>> = const { RefCell::new(None) };
    static ON_CLOCK: RefCell<Option<Box<dyn FnMut(u64)>>> = const { RefCell::new(None) };
}

/// The fake clock: one millisecond per read, and one device step per read.
///
/// The queue's waits are all "poll until the deadline", so a fake clock that stands
/// still would spin forever; a fake clock that steps per read bounds every wait by the
/// number of reads it makes. One read = one millisecond means a two-second deadline is
/// two thousand polls, which is nothing, and it keeps the wait deterministic.
pub fn clock() -> u64 {
    let now = CLOCK.with(|clock| {
        let now = clock.get() + STEP.with(|step| step.get());
        clock.set(now);
        now
    });
    ON_CLOCK.with_borrow_mut(|hook| {
        if let Some(hook) = hook.as_mut() { hook(now); }
    });
    if let Some(device) = CURRENT.with(|current| current.borrow().clone()) {
        device.tick();
    }
    now
}

/// The fake time, for cases that assert a wait really ran out its deadline.
pub fn now() -> u64 {
    CLOCK.with(|clock| clock.get())
}

pub fn on_clock(hook: impl FnMut(u64) + 'static) {
    ON_CLOCK.with(|current| *current.borrow_mut() = Some(Box::new(hook)));
}
