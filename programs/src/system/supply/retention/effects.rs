use env::{AcquireSource, FlatReference, PieToken};

use super::Failure;

pub trait MemoryEffects {
    fn acquire(
        &mut self,
        source: AcquireSource,
        retention: PieToken,
    ) -> Result<FlatReference, Failure>;

    fn release(&mut self, reference: FlatReference) -> Result<(), Failure>;
}
