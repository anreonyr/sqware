use crate::domain::{FIRST_IOVA, IOVA_LIMIT, IOVA_STRIDE};
use crate::{PAGE_SIZE, PieToken, pages::Pages, table};

fn table_base(index: u64) -> usize {
    (FIRST_IOVA + index * IOVA_STRIDE) as usize
}

fn take() -> Pages {
    let mut pages = Pages::default();
    pages.take(PieToken::mint(7), 4).unwrap();
    pages
}

fn word(pages: &Pages, page: usize, slot: usize) -> u64 {
    let offset = slot * 8;
    u64::from(pages.get32(page, offset)) | (u64::from(pages.get32(page, offset + 4)) << 32)
}

#[test]
fn private_iova_windows_cross_root_paths_and_reach_the_last_sv39_window() {
    let mut pages = take();
    let physical = [0x4000_0000, 0x6000_0000, 0x7000_0000];
    for serial in [1, 63, 64, 511, 512, 126_975] {
        for page in 0..3 {
            pages.fill(page, 0);
        }
        let base = table_base(serial);
        table::map_physical(&pages, 0, 1, 2, base, &physical).unwrap();
        let root = (base >> 30) & 511;
        let middle = (base >> 21) & 511;
        for slot in 0..512 {
            assert_eq!(word(&pages, 0, slot), if slot == root {
                1 | ((pages.physical(1) as u64 >> 12) << 10)
            } else { 0 });
            assert_eq!(word(&pages, 1, slot), if slot == middle {
                1 | ((pages.physical(2) as u64 >> 12) << 10)
            } else { 0 });
            if slot < physical.len() {
                assert_eq!(word(&pages, 2, slot) >> 10, physical[slot] >> 12);
                assert_eq!(word(&pages, 2, slot) & 0xff, 0xd7);
            } else {
                assert_eq!(word(&pages, 2, slot), 0);
            }
        }
    }
    pages.close().unwrap();
}

#[test]
fn noncanonical_wrapping_empty_or_cross_leaf_windows_do_not_write_any_pte() {
    let mut pages = take();
    for page in 0..3 {
        pages.fill(page, 0xa5);
    }
    for (base, physical) in [
        (IOVA_LIMIT as usize, vec![0x4000_0000]),
        (usize::MAX & !(PAGE_SIZE - 1), vec![0x4000_0000; 2]),
        (0, vec![]),
        (1, vec![0x4000_0000]),
        (IOVA_STRIDE as usize - PAGE_SIZE, vec![0x4000_0000; 2]),
    ] {
        assert!(table::map_physical(&pages, 0, 1, 2, base, &physical).is_err());
    }
    assert!(table::map_pages(&pages, 0, 1, 2, 0, usize::MAX, 2).is_err());
    assert!(table::map_pages(&pages, 0, 1, 2, 0, 0, usize::MAX).is_err());
    for page in 0..3 {
        for slot in 0..512 {
            assert_eq!(word(&pages, page, slot), 0xa5a5_a5a5_a5a5_a5a5);
        }
    }
    pages.close().unwrap();
}
