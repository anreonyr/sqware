use sbi::ecall::SArgs;
use sbi::{self, fid};

use crate::core::memory::allocator::bitmap::BitmapAllocator;
use crate::machine::hart;
use crate::machine::sync::{Level, SpinLock};

use super::MapError;

mod error;

pub use error::Deaf;

pub(crate) const ASID_BITS: u32 = 16;
const VACANT: usize = 1 << ASID_BITS;

pub(crate) const fn vacant() -> usize {
    VACANT
}

static ASID_ALLOCATOR: SpinLock<BitmapAllocator> =
    SpinLock::new_level(Level::Asid, BitmapAllocator::new(1, 65536, 1));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Asid(usize);

impl Asid {
    pub(crate) const fn kernel() -> Self {
        Self(0)
    }

    pub(crate) const fn from_raw(raw: usize) -> Self {
        Self(raw & 0xFFFF)
    }

    pub(crate) fn try_allocate() -> Option<Self> {
        ASID_ALLOCATOR
            .lock()
            .allocate(1)
            .ok()
            .map(|(asid, _)| Self(asid))
    }

    pub(crate) fn release_unpublished(asid: Self) {
        debug_assert!(!asid.is_kernel());
        ASID_ALLOCATOR
            .lock()
            .deallocate(asid.get(), 1)
            .expect("asid: unpublished allocation must be owned");
    }

    pub fn get(self) -> usize {
        self.0
    }

    pub fn is_kernel(self) -> bool {
        self.0 == 0
    }
}

pub fn deallocate(asid: Asid) -> Result<(), Deaf> {
    shootdown(asid)?;
    ASID_ALLOCATOR
        .lock()
        .deallocate(asid.get(), 1)
        .expect("asid: double-free or never-allocated");
    Ok(())
}

pub fn occupy(asid: Asid) {
    hart::lease_store(asid.get());
}

pub fn vacate() {
    hart::lease_store(VACANT);
}

pub(crate) fn lease(hart: crate::machine::hart::HartId) -> Option<Asid> {
    match hart::lease_load(hart) {
        VACANT => None,
        raw => Some(Asid::from_raw(raw)),
    }
}

#[cfg(debug_assertions)]
static FAIL_NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(VACANT);

#[cfg(debug_assertions)]
pub(crate) fn fail_next_shootdown(asid: Asid) {
    assert!(crate::testing(), "shootdown injection requires a test run");
    FAIL_NEXT
        .compare_exchange(
            VACANT,
            asid.get(),
            core::sync::atomic::Ordering::Relaxed,
            core::sync::atomic::Ordering::Relaxed,
        )
        .expect("shootdown injection already armed");
}

pub fn shootdown(asid: Asid) -> Result<(), Deaf> {
    crate::core::memory::allocator::assert_allocation_allowed();
    unsafe { flush(asid) };

    #[cfg(debug_assertions)]
    if FAIL_NEXT
        .compare_exchange(
            asid.get(),
            VACANT,
            core::sync::atomic::Ordering::Relaxed,
            core::sync::atomic::Ordering::Relaxed,
        )
        .is_ok()
    {
        return Err(Deaf { asid });
    }

    let me = hart::hart_id();
    let mut mask = 0usize;
    for hart in (0..hart::hart_count()).map(crate::machine::hart::HartId::new) {
        if hart == me {
            continue;
        }
        if (asid.is_kernel() && crate::machine::hart::online(hart)) || lease(hart) == Some(asid) {
            let (_, bit) = hart.bit();
            mask |= bit;
        }
    }

    if mask == 0 {
        return Ok(());
    }

    let operation = if asid.is_kernel() {
        fid::Rfence::RemoteSfenceVma
    } else {
        fid::Rfence::RemoteSfenceVmaAsid
    };
    let r = sbi::RfenceCall::new(operation)
        .args(SArgs {
            a0: mask,
            a1: 0,
            a2: 0,
            a3: 0,
            a4: asid.get(),
            ..Default::default()
        })
        .call();
    r.map(|_| ()).map_err(|_| Deaf { asid })
}

const _: () = assert!(ASID_BITS == 16);

/// 本 hart 直接失效该 ASID 的 TLB（shootdown 的本地一半）。
#[inline(always)]
pub unsafe fn flush(asid: Asid) {
    unsafe {
        if asid.is_kernel() {
            core::arch::asm!("sfence.vma zero, zero");
        } else {
            core::arch::asm!("sfence.vma zero, {}", in(reg) asid.get());
        }
    }
}

/// 让已写下的指令字节对所有在线地址空间的 hart 可见。
pub fn sync_instructions() -> Result<(), MapError> {
    unsafe { core::arch::asm!("fence rw, rw", options(nostack)) };
    hart::request_instruction_sync();
    unsafe { core::arch::asm!("fence rw, rw", "fence.i", options(nostack)) };
    let me = hart::hart_id();
    let mut mask = 0usize;
    for h in (0..hart::hart_count()).map(hart::HartId::new) {
        if h != me && lease(h).is_some() {
            mask |= h.bit().1;
        }
    }
    sbi::RfenceCall::new(sbi::fid::Rfence::RemoteFenceI)
        .args(SArgs {
            a0: mask,
            a1: 0,
            ..Default::default()
        })
        .call()
        .map(|_| ())
        .map_err(|_| MapError::NoRegion)
}
