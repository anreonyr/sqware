//! 持有账要的那两步效果：把 `acquire` / `release` 落到本驱动自己引用表上的真实调用。

use env::{AcquireSource, FlatReference, PieToken};

use super::hold::{Failure, MemoryEffects};

/// 持有账的调用方作用域就是本驱动那一张引用表（没有别人的表，也没有影子账）。
pub struct References;

impl MemoryEffects for References {
    fn acquire(
        &mut self,
        source: AcquireSource,
        retention: PieToken,
    ) -> Result<FlatReference, Failure> {
        execution::memory::acquire(source, retention).map_err(|_| Failure::Denied)
    }

    fn release(&mut self, reference: FlatReference) -> Result<(), Failure> {
        execution::memory::release(reference).map_err(|_| Failure::Pending)
    }
}
