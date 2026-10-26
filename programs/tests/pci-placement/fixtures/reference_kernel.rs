use execution_impl::memory::ReferenceKernel;

/// PCI 唯一的引用内核适配：只持有来源，不实现映射。
pub struct FixtureKernel;

impl ReferenceKernel for FixtureKernel {
    fn acquire(
        source: env::AcquireSource,
        retention: env::PieToken,
    ) -> env::MemoryResult<env::FlatReference> {
        crate::state::acquire(source, retention)
    }

    fn release(reference: env::FlatReference) -> env::MemoryResult<()> {
        crate::state::release_reference(reference)
    }
}
