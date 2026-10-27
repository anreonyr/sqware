use super::{DeviceRange, Failure};

/// Installed only by this driver's own composition, never by a service request.
/// Methods are bounded progress steps; Pending retains all references for retry.
pub trait Platform {
    fn range(&self) -> DeviceRange;

    /// Revoke translations, invalidate device caches and drain all external readers/writers,
    /// including the platform's own page-table and queue engines.
    fn synchronize(&mut self) -> Result<(), Failure>;

    /// Independently stop this entire hardware scope and confirm all external access ended.
    /// CPU capability revocation, task exit and interrupt masking are not this confirmation.
    fn isolate(&mut self) -> Result<(), Failure>;
}
