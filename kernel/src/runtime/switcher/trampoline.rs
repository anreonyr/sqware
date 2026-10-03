use core::arch::global_asm;

use crate::layout::TRAMPOLINE;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::asid::{self, Asid};
use crate::runtime::switcher::context::TrapContext;

global_asm!(
    ".section .trampoline, \"ax\"",
    ".align 12",
    ".globl __trampoline_start",
    "__trampoline_start:",
    ".globl __alltraps",
    "__alltraps:",
    "    csrrw sp, sscratch, sp",
    "    sd    x5, 0x58(sp)",
    "    csrr  t0, satp",
    "    srli  t0, t0, 44",
    "    slli  t0, t0, 48",
    "    bnez  t0, __task_trap",
    "    j     __core_trap",
    "__task_trap:",
    "    sd    x1,  0x38(sp)",
    "    csrr  t0, sscratch",
    "    sd    t0,  0x40(sp)",
    "    sd    x3,  0x48(sp)",
    "    sd    x4,  0x50(sp)",
    "    sd    x6,  0x60(sp)",
    "    sd    x7,  0x68(sp)",
    "    sd    x8,  0x70(sp)",
    "    sd    x9,  0x78(sp)",
    "    sd    x10, 0x80(sp)",
    "    sd    x11, 0x88(sp)",
    "    sd    x12, 0x90(sp)",
    "    sd    x13, 0x98(sp)",
    "    sd    x14, 0xa0(sp)",
    "    sd    x15, 0xa8(sp)",
    "    sd    x16, 0xb0(sp)",
    "    sd    x17, 0xb8(sp)",
    "    sd    x18, 0xc0(sp)",
    "    sd    x19, 0xc8(sp)",
    "    sd    x20, 0xd0(sp)",
    "    sd    x21, 0xd8(sp)",
    "    sd    x22, 0xe0(sp)",
    "    sd    x23, 0xe8(sp)",
    "    sd    x24, 0xf0(sp)",
    "    sd    x25, 0xf8(sp)",
    "    sd    x26, 0x100(sp)",
    "    sd    x27, 0x108(sp)",
    "    sd    x28, 0x110(sp)",
    "    sd    x29, 0x118(sp)",
    "    sd    x30, 0x120(sp)",
    "    sd    x31, 0x128(sp)",
    "    csrr  t0, sstatus",
    "    sd    t0,  0x130(sp)",
    "    csrr  t0, sepc",
    "    sd    t0,  0x138(sp)",
    "    ld    t0,  0x00(sp)",
    "    ld    t1,  0x08(sp)",
    "    ld    t2,  0x10(sp)",
    "    ld    a0,  0x20(sp)",
    "    csrw  satp, t0",
    "    sfence.vma",
    "    mv    sp, t1",
    "    jalr  t2",
    "    j     __restore",
    "__core_trap:",
    "    sd    x1,  0x38(sp)",
    "    sd    x3,  0x48(sp)",
    "    sd    x4,  0x50(sp)",
    "    sd    x6,  0x60(sp)",
    "    sd    x7,  0x68(sp)",
    "    sd    x8,  0x70(sp)",
    "    sd    x9,  0x78(sp)",
    "    sd    x10, 0x80(sp)",
    "    sd    x11, 0x88(sp)",
    "    sd    x12, 0x90(sp)",
    "    sd    x13, 0x98(sp)",
    "    sd    x14, 0xa0(sp)",
    "    sd    x15, 0xa8(sp)",
    "    sd    x16, 0xb0(sp)",
    "    sd    x17, 0xb8(sp)",
    "    sd    x18, 0xc0(sp)",
    "    sd    x19, 0xc8(sp)",
    "    sd    x20, 0xd0(sp)",
    "    sd    x21, 0xd8(sp)",
    "    sd    x22, 0xe0(sp)",
    "    sd    x23, 0xe8(sp)",
    "    sd    x24, 0xf0(sp)",
    "    sd    x25, 0xf8(sp)",
    "    sd    x26, 0x100(sp)",
    "    sd    x27, 0x108(sp)",
    "    sd    x28, 0x110(sp)",
    "    sd    x29, 0x118(sp)",
    "    sd    x30, 0x120(sp)",
    "    sd    x31, 0x128(sp)",
    "    csrr  t0, sscratch",
    "    sd    t0,  0x40(sp)",
    "    csrr  t0, sstatus",
    "    sd    t0,  0x130(sp)",
    "    csrr  t0, sepc",
    "    sd    t0,  0x138(sp)",
    "    ld    t2,  0x10(sp)",
    "    ld    a0,  0x20(sp)",
    "    ld    sp,  0x08(sp)",
    "    jalr  t2",
    "    j     __restore",
    ".globl __restore",
    "__restore:",
    "    mv    sp, a0",
    "    ld    t0, 0x130(sp)",
    "    andi  t0, t0, -3",
    "    csrw  sstatus, t0",
    "    ld    t0, 0x138(sp)",
    "    csrw  sepc, t0",
    "    ld    t0,  0x28(sp)",
    "    srli  t1,  t0, 44",
    "    slli  t1,  t1, 48",
    "    bnez  t1, 1f",
    "    ld    t0, 0x08(tp)",
    "    csrw  sscratch, t0",
    "    j     2f",
    "1:",
    "    ld    t0,  0x140(sp)",
    "    csrw  sscratch, t0",
    "2:",
    "    ld    x1,  0x38(sp)",
    "    ld    x3,  0x48(sp)",
    "    ld    x4,  0x50(sp)",
    "    ld    x7,  0x68(sp)",
    "    ld    x8,  0x70(sp)",
    "    ld    x9,  0x78(sp)",
    "    ld    x10, 0x80(sp)",
    "    ld    x11, 0x88(sp)",
    "    ld    x12, 0x90(sp)",
    "    ld    x13, 0x98(sp)",
    "    ld    x14, 0xa0(sp)",
    "    ld    x15, 0xa8(sp)",
    "    ld    x16, 0xb0(sp)",
    "    ld    x17, 0xb8(sp)",
    "    ld    x18, 0xc0(sp)",
    "    ld    x19, 0xc8(sp)",
    "    ld    x20, 0xd0(sp)",
    "    ld    x21, 0xd8(sp)",
    "    ld    x22, 0xe0(sp)",
    "    ld    x23, 0xe8(sp)",
    "    ld    x24, 0xf0(sp)",
    "    ld    x25, 0xf8(sp)",
    "    ld    x26, 0x100(sp)",
    "    ld    x27, 0x108(sp)",
    "    ld    x28, 0x110(sp)",
    "    ld    x29, 0x118(sp)",
    "    ld    x30, 0x120(sp)",
    "    ld    x31, 0x128(sp)",
    "    ld    t0,  0x28(sp)",
    "    ld    t1,  0x140(sp)",
    "    csrw  satp, t0",
    "    sfence.vma",
    "    mv    x5,  t1",
    "    ld    x6,  0x60(x5)",
    "    ld    x2,  0x40(x5)",
    "    ld    x5,  0x58(x5)",
    "    fence.i",
    "    sret",
    ".globl __trampoline_end",
    "__trampoline_end:",
);

unsafe extern "C" {
    pub static __trampoline_start: u8;
    pub static __alltraps: u8;
    pub static __trampoline_end: u8;
    pub static __restore: u8;
}

pub fn restore(frame_pa: usize) -> ! {
    // SAFETY: frame_pa 为有效帧物理地址
    asid::occupy(Asid::from_raw(unsafe {
        (*(frame_pa as *const TrapContext)).user_satp.asid()
    }));
    let link = core::ptr::addr_of!(__restore) as usize;
    let va = TRAMPOLINE.as_usize() + (link - core::ptr::addr_of!(__trampoline_start) as usize);
    unsafe {
        core::arch::asm!(
            "jalr ra, {addr}",
            addr = in(reg) va,
            in("a0") frame_pa,
            options(noreturn),
        );
    }
}

pub fn alltraps_va() -> usize {
    let start = core::ptr::addr_of!(__trampoline_start) as usize;
    let alltraps = core::ptr::addr_of!(__alltraps) as usize;
    TRAMPOLINE.as_usize() + (alltraps - start)
}

pub fn check_fits_page() {
    let tsize = (core::ptr::addr_of!(__trampoline_end) as usize)
        - (core::ptr::addr_of!(__trampoline_start) as usize);
    assert!(
        tsize <= PAGE_SIZE,
        "trampoline exceeds one page: {tsize:#x}"
    );
}
