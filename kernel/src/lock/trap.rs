use riscv::register::sstatus;

pub(crate) struct TrapGuard {
    sie_was_enabled: bool,
}

impl TrapGuard {
    #[inline(always)]
    pub(crate) unsafe fn save() -> Self {
        unsafe {
            let was = sstatus::read().sie();
            if sstatus::read().sie() {
                sstatus::clear_sie();
            }
            TrapGuard {
                sie_was_enabled: was,
            }
        }
    }
}

impl Drop for TrapGuard {
    #[inline(always)]
    fn drop(&mut self) {
        if self.sie_was_enabled {
            // SAFETY: 恢复 SIE；与 save 同序
            unsafe {
                sstatus::set_sie();
            }
        }
    }
}
