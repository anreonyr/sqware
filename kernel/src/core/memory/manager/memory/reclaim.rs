use super::mapping::Mapping;
use super::range_reclaim::RangeReclaim;

pub(super) enum Reclaim {
    Mapping(Mapping),
    Range(RangeReclaim),
}
