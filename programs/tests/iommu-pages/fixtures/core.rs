use crate::{PAGE_SIZE, layout, regs, state};

pub fn base() -> usize {
    state::STATE.with_borrow(|state| state.mmio.as_ptr() as usize)
}

pub fn hold(held: bool) {
    state::STATE.with_borrow_mut(|state| state.held_queue = held);
}

pub fn completion(slot: usize) -> u32 {
    let address = state::STATE.with_borrow(|state| state.view_address + layout::FENCE * PAGE_SIZE + slot * 4);
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

pub fn complete(slot: usize, value: u32) {
    let address = state::STATE.with_borrow(|state| state.view_address + layout::FENCE * PAGE_SIZE + slot * 4);
    unsafe { core::ptr::write_volatile(address as *mut u32, value) };
}

pub fn consume(count: u32) {
    let base = base();
    let (address, bytes) = state::STATE.with_borrow(|state| (state.view_address, state.view_bytes));
    let mut head = regs::read32(base, regs::CQH);
    let tail = regs::read32(base, regs::CQT);
    let commands = ((regs::read64(base, regs::CQB) & regs::PPN_FIELD) >> regs::PPN_SHIFT) as usize * PAGE_SIZE;
    let command_offset = commands.checked_sub(PAGE_SIZE).unwrap();
    assert!(command_offset + PAGE_SIZE <= bytes);
    for _ in 0..count {
        if head == tail { break; }
        let command = (address + command_offset + (head % 256) as usize * 16) as *const u64;
        let word = unsafe { core::ptr::read_volatile(command) };
        if word & 0x7f == 2 && word & (1 << 10) != 0 {
            let physical = unsafe { core::ptr::read_volatile(command.add(1)) } << 2;
            let offset = (physical as usize).checked_sub(PAGE_SIZE).unwrap();
            assert!(offset + 4 <= bytes && offset % 4 == 0);
            unsafe { core::ptr::write_volatile((address + offset) as *mut u32, (word >> 32) as u32) };
        }
        head = head.wrapping_add(1);
        regs::write32(base, regs::CQH, head);
    }
}

pub fn clock() -> u64 {
    let (time, address, held, blocked) = state::STATE.with_borrow_mut(|state| {
        state.clock += 1_000_000;
        (state.clock, state.view_address, state.held_queue, state.blocked_queue)
    });
    if address == 0 { return time; }
    let base = base();
    for csr in [regs::CQCSR, regs::FQCSR] {
        let seen = regs::read32(base, csr);
        if seen & regs::QUEUE_ENABLE != 0 && blocked != Some(csr) {
            regs::write32(base, csr, (seen | regs::QUEUE_ON) & !regs::QUEUE_MEM_FAULT);
        }
    }
    if !held && regs::read32(base, regs::CQCSR) & regs::QUEUE_ON != 0 { consume(256); }
    time
}
