//! 一次私有分配的完整坐标：哪一个 Domain 的哪一段 IOVA。

use super::domain_id::DomainId;
use super::hold::DeviceRange;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DomainRange {
    pub domain: DomainId,
    pub range: DeviceRange,
}
