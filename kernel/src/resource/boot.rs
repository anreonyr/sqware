use env::ledger::entry::{Header, VERSION};
use env::wire::{Span, store_tail};
use env::{Entry, PieFail};

pub(crate) const BLOCK_BYTES: usize = crate::memory::PAGE_SIZE;
#[repr(C, align(4096))]
struct Block(core::cell::UnsafeCell<[u8; BLOCK_BYTES]>);
// SAFETY: written by the boot hart before the initial task is released; then read-only.
unsafe impl Sync for Block {}
static BLOCK: Block = Block(core::cell::UnsafeCell::new([0; BLOCK_BYTES]));

pub(crate) fn block() -> (usize, usize) {
    (core::ptr::addr_of!(BLOCK) as usize, BLOCK_BYTES)
}
pub(crate) fn size(count: usize) -> Result<usize, PieFail> {
    let len = count
        .checked_mul(env::ENTRY_LEN)
        .and_then(|n| n.checked_add(Header::LEN))
        .ok_or(PieFail::OoM)?;
    if len > BLOCK_BYTES {
        return Err(PieFail::OoM);
    }
    Ok(len)
}
pub(crate) fn write(entries: &[Entry]) -> Result<usize, PieFail> {
    let len = size(entries.len())?;
    let count = u32::try_from(entries.len()).map_err(|_| PieFail::OoM)?;
    // SAFETY: single boot writer, and no task has been released to read this mapping yet.
    let out = unsafe { &mut *BLOCK.0.get() };
    let at = Header {
        version: VERSION,
        count,
    }
    .store_at(out, 0)
    .ok_or(PieFail::Denied)?;
    let end = store_tail(out, at, entries).ok_or(PieFail::Denied)?;
    if end != len {
        return Err(PieFail::Denied);
    }
    Ok(len)
}
