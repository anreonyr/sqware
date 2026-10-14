use riscv::register::{satp, sstatus};

use crate::core::memory::manager::addr::{PhysAddr, VirtAddr};

mod gprs;
pub use gprs::Gprs;

#[derive(Debug)]
#[repr(C)]
pub struct TrapContext {
    pub kernel_satp: satp::Satp,
    pub kernel_sp: VirtAddr,
    pub trap_handler: VirtAddr,
    pub trap_stack_corrupt: usize,
    pub user_pa: PhysAddr,
    pub user_satp: satp::Satp,
    pub gpr: Gprs,
    pub sstatus: sstatus::Sstatus,
    pub sepc: VirtAddr,
    pub self_va: VirtAddr,
}

impl TrapContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn init(
        &mut self,
        template: &TrapContext,
        user_satp: satp::Satp,
        spp: sstatus::SPP,
        entry: VirtAddr,
        stack_top: VirtAddr,
        args: (VirtAddr, usize),
        pa: PhysAddr,
        self_va: VirtAddr,
    ) {
        self.kernel_satp = template.kernel_satp;
        self.trap_handler = template.trap_handler;
        self.trap_stack_corrupt = template.trap_stack_corrupt;
        self.user_pa = pa;
        self.user_satp = user_satp;
        self.self_va = self_va;
        self.sepc = entry;
        self.gpr.set_x(Gprs::SP, stack_top.as_usize());
        self.gpr.set_x(Gprs::A0, args.0.as_usize());
        self.gpr.set_x(Gprs::A1, args.1);
        let mut ss = sstatus::Sstatus::from_bits(0);
        ss.set_spie(true);
        ss.set_spp(spp);
        self.sstatus = ss;
    }
}

const _: () = {
    assert!(::core::mem::offset_of!(TrapContext, kernel_satp) == 0x00);
    assert!(::core::mem::offset_of!(TrapContext, kernel_sp) == 0x08);
    assert!(::core::mem::offset_of!(TrapContext, trap_handler) == 0x10);
    assert!(::core::mem::offset_of!(TrapContext, trap_stack_corrupt) == 0x18);
    assert!(::core::mem::offset_of!(TrapContext, user_pa) == 0x20);
    assert!(::core::mem::offset_of!(TrapContext, user_satp) == 0x28);
    assert!(::core::mem::offset_of!(TrapContext, gpr) == 0x30);
    assert!(::core::mem::offset_of!(TrapContext, sstatus) == 0x130);
    assert!(::core::mem::offset_of!(TrapContext, sepc) == 0x138);
    assert!(::core::mem::offset_of!(TrapContext, self_va) == 0x140);
};
