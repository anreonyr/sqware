//! 清理的单一重试纪律：临时阻抗时让步后原样重试，其他结果原样交回。

use crate::kernel::CapabilityKernel;

/// Complete cleanup despite temporary contention, yielding between attempts.
pub fn cleanup<K: CapabilityKernel>(
    mut run: impl FnMut() -> Result<(), K::PieError>,
) -> Result<(), K::PieError> {
    loop {
        match run() {
            Err(error) if K::is_busy(&error) => K::starve(),
            result => return result,
        }
    }
}
