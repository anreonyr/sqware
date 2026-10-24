use crate::{PAGE_SIZE, context::{DeviceContext, PSCID_LIMIT}};

#[test]
fn first_stage_encodes_an_independent_twenty_bit_pscid_with_reserved_bits_zero() {
    for pscid in [0, 1, 9, 17, PSCID_LIMIT - 1] {
        let context = DeviceContext::first_stage(2 * PAGE_SIZE, pscid).unwrap();
        let words = context.words();
        assert_eq!(words[0], 1);
        assert_eq!(words[1], 0);
        assert_eq!(words[2], u64::from(pscid) << 12);
        assert_eq!(words[3], (8 << 60) | 2);
        assert_eq!(&words[4..], &[0; 4]);
    }
}

#[test]
fn unsupported_pscids_or_root_addresses_are_rejected_before_encoding() {
    for pscid in [PSCID_LIMIT, u32::MAX] {
        assert!(DeviceContext::first_stage(PAGE_SIZE, pscid).is_none());
    }
    for root in [1, PAGE_SIZE + 1, 1usize << 56, usize::MAX & !(PAGE_SIZE - 1)] {
        assert!(DeviceContext::first_stage(root, 1).is_none());
    }
}

#[test]
fn every_context_owns_three_distinct_table_pages_outside_the_shared_engine_pages() {
    use crate::layout;
    let mut pages = std::collections::BTreeSet::from([
        layout::DDT, layout::COMMANDS, layout::FAULTS, layout::FENCE,
    ]);
    assert_eq!(pages.len(), 4);
    for requester in 0..crate::context::CAPACITY {
        let (root, middle, leaf) = layout::tables(requester as u16).unwrap();
        for page in [root, middle, leaf] {
            assert!(page < layout::PAGES);
            assert!(pages.insert(page), "context table shares page {page}");
        }
    }
    assert_eq!(pages.len(), layout::PAGES);
    assert_eq!(layout::PAGES, 196);
    assert!(layout::tables(crate::context::CAPACITY as u16).is_none());
}
