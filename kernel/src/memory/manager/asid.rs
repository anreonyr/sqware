use fack::prelude::Error;
use sbi::ecall::SArgs;
use sbi::{self, fid};

use crate::hart;
use crate::lock::{Level, SpinLock};
use crate::memory::allocator::bitmap::BitmapAllocator;

use super::flush_asid;

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

    pub(crate) fn allocate() -> Self {
        let (asid, _) = ASID_ALLOCATOR
            .lock()
            .allocate(1)
            .expect("asid: 16-bit ASID space exhausted (65535 tasks)");
        Self(asid)
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

pub(crate) fn lease(hart: crate::hart::HartId) -> Option<Asid> {
    match hart::lease_load(hart) {
        VACANT => None,
        raw => Some(Asid::from_raw(raw)),
    }
}

pub fn shootdown(asid: Asid) -> Result<(), Deaf> {
    crate::memory::allocator::assert_allocation_allowed();
    // SAFETY: 页表已改完，刷后翻译即新映射
    unsafe { flush_asid(asid) };

    let me = hart::hart_id();
    let mut mask = 0usize;
    for hart in (0..hart::hart_count()).map(crate::hart::HartId::new) {
        if hart == me {
            continue;
        }
        if asid.is_kernel() || lease(hart) == Some(asid) {
            let (_, bit) = hart.bit();
            mask |= bit;
        }
    }

    let operation = if asid.is_kernel() { fid::Rfence::RemoteSfenceVma }
        else { fid::Rfence::RemoteSfenceVmaAsid };
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

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("remote sfence failed for asid {}", asid.get())]
pub struct Deaf {
    pub asid: Asid,
}

const _: () = assert!(ASID_BITS == 16);
