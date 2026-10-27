use crate::DeviceRange;

#[test]
fn closed_interval_overlap_does_not_overflow_at_the_top_of_the_address_space() {
    let last = DeviceRange::new(u64::MAX, 1).unwrap();
    let first = DeviceRange::new(0, 1).unwrap();
    let below = DeviceRange::new(u64::MAX - 1, 2).unwrap();
    assert!(!last.overlaps(first));
    assert!(!first.overlaps(last));
    assert!(last.overlaps(last));
    assert!(last.overlaps(below));
    assert!(below.overlaps(last));
    assert!(DeviceRange::new(u64::MAX, 2).is_none());
    assert!(DeviceRange::new(1, 0).is_none());
}
