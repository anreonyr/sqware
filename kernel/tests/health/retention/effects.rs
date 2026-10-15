use super::super::super::retention_core::{Failure, MemoryEffects};
use super::super::Caller;
use env::{AcquireSource, EnvCall, FlatReference, MemoryCall, PieToken};

pub(super) struct Effects<'a> {
    pub caller: &'a Caller,
}

impl MemoryEffects for Effects<'_> {
    fn acquire(
        &mut self,
        source: AcquireSource,
        retention: PieToken,
    ) -> Result<FlatReference, Failure> {
        let value = self
            .caller
            .call(EnvCall::Memory(MemoryCall::Acquire { source, retention }));
        if value <= 0 {
            return Err(Failure::Denied);
        }
        Ok(FlatReference::from_bytes(&(value as usize).to_le_bytes()).unwrap())
    }

    fn release(&mut self, reference: FlatReference) -> Result<(), Failure> {
        match self
            .caller
            .call(EnvCall::Memory(MemoryCall::Release { reference }))
        {
            0 => Ok(()),
            _ => Err(Failure::Pending),
        }
    }
}
