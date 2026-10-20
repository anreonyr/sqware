use core::sync::atomic::Ordering;

use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
use crate::core::memory::nest::identity::Asid;
use crate::core::memory::nest::table::TABLE_LIVE;
use crate::core::work::room::{messenger, scheduler};

pub(super) struct Baseline {
    tables: usize,
    pages: usize,
    sites: usize,
    husks: usize,
}

impl Baseline {
    pub(super) fn new() -> Self {
        if crate::machine::hart::scheduler().is_null() {
            scheduler::init().expect("scheduler init");
        }
        let seed = Asid::try_allocate().unwrap();
        Asid::release_unpublished(seed);
        Self {
            tables: TABLE_LIVE.load(Ordering::Relaxed),
            pages: frame_occupied() - block_occupied(),
            sites: messenger::site_count(),
            husks: messenger::probe_bookkeeping().1,
        }
    }

    pub(super) fn expect(self) {
        assert_eq!(
            messenger::probe_bookkeeping().1,
            self.husks,
            "sub_pole: husk 表回基线"
        );
        assert_eq!(
            messenger::site_count(),
            self.sites,
            "sub_pole: 等待站点回基线"
        );
        assert_eq!(
            TABLE_LIVE.load(Ordering::Relaxed),
            self.tables,
            "sub_pole: 页表回基线"
        );
        assert_eq!(
            frame_occupied() - block_occupied(),
            self.pages,
            "sub_pole: 整页账回基线"
        );
    }
}
