use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use env::Permission;

pub(crate) struct Grant {
    permission: AtomicU32,
}

impl Grant {
    pub(crate) fn try_new(permission: Permission) -> Result<Arc<Self>, env::PieFail> {
        Arc::try_new(Self {
            permission: AtomicU32::new(permission.bits()),
        })
        .map_err(|_| env::PieFail::OoM)
    }

    pub(crate) fn permission(&self) -> Option<Permission> {
        let bits = self.permission.load(Ordering::Acquire);
        (bits != 0).then(|| Permission::from_bits_retain(bits))
    }

    pub(crate) fn narrow(&self, subset: Permission) -> Result<(), env::PieFail> {
        let subset_bits = subset.bits();
        let mut current = self.permission.load(Ordering::Acquire);
        loop {
            if current == 0 {
                return Err(env::PieFail::Dead);
            }
            let permission = Permission::from_bits_retain(current);
            if subset.is_empty()
                || !permission.contains(subset)
                || permission.contains(Permission::ONLY) && !subset.contains(Permission::ONLY)
            {
                return Err(env::PieFail::Denied);
            }
            match self.permission.compare_exchange_weak(
                current,
                current & subset_bits,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn invalidate(&self) {
        self.permission.store(0, Ordering::Release);
    }
}
