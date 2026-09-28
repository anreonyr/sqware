use riscv::register::{satp, sstatus};

use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::work::unit::team::Team;

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Gprs(pub [usize; 32]);

#[allow(unused)]
impl Gprs {
    pub const RA: usize = 1;
    pub const SP: usize = 2;
    pub const GP: usize = 3;
    pub const TP: usize = 4;
    pub const S0: usize = 8;
    pub const A0: usize = 10;
    pub const A1: usize = 11;
    pub const A2: usize = 12;
    pub const A3: usize = 13;
    pub const A4: usize = 14;
    pub const A5: usize = 15;
    pub const A6: usize = 16;
    pub const A7: usize = 17;

    #[inline]
    pub fn x(&self, i: usize) -> usize {
        self.0[i]
    }

    #[inline]
    pub fn set_x(&mut self, i: usize, v: usize) {
        debug_assert!(i != 0, "x0 恒 0，不可写");
        self.0[i] = v;
    }
}

impl core::fmt::Debug for Gprs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "gpr{{")?;
        let mut any = false;
        for (i, v) in self.0.iter().enumerate().skip(1) {
            if *v != 0 {
                if any {
                    write!(f, " ")?;
                }
                write!(f, "x{i}={v:#x}")?;
                any = true;
            }
        }
        write!(f, "}}")
    }
}

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
        team: &Team,
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
        self.user_satp = satp::Satp::from_bits(
            (crate::memory::manager::mode::mode().into_usize() << 60)
                | (team.space.asid().get() << 44)
                | team.space.root(),
        );
        self.self_va = self_va;
        self.sepc = entry;
        self.gpr.set_x(Gprs::SP, stack_top.as_usize());
        self.gpr.set_x(Gprs::A0, args.0.as_usize());
        self.gpr.set_x(Gprs::A1, args.1);
        let mut ss = sstatus::Sstatus::from_bits(0);
        ss.set_spie(true);
        ss.set_spp(if team.space.kind().is_supervisor() {
            sstatus::SPP::Supervisor
        } else {
            sstatus::SPP::User
        });
        self.sstatus = ss;
    }
}

const _: () = {
    assert!(core::mem::offset_of!(TrapContext, kernel_satp) == 0x00);
    assert!(core::mem::offset_of!(TrapContext, kernel_sp) == 0x08);
    assert!(core::mem::offset_of!(TrapContext, trap_handler) == 0x10);
    assert!(core::mem::offset_of!(TrapContext, trap_stack_corrupt) == 0x18);
    assert!(core::mem::offset_of!(TrapContext, user_pa) == 0x20);
    assert!(core::mem::offset_of!(TrapContext, user_satp) == 0x28);
    assert!(core::mem::offset_of!(TrapContext, gpr) == 0x30);
    assert!(core::mem::offset_of!(TrapContext, sstatus) == 0x130);
    assert!(core::mem::offset_of!(TrapContext, sepc) == 0x138);
    assert!(core::mem::offset_of!(TrapContext, self_va) == 0x140);
};