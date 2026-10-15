use env::{AcquireSource, PieToken};

use super::DeviceRange;

pub struct Registration {
    pub range: DeviceRange,
    pub source: AcquireSource,
    pub retention: PieToken,
}
