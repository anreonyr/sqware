#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Site {
    Roster,
    Holder,
    TeamTasks,
    Sire,
    Muster,
    Snapshot,
    Empty,
}

#[cfg(debug_assertions)]
const ALL: [Site; NSITE] = [
    Site::Roster,
    Site::Holder,
    Site::TeamTasks,
    Site::Sire,
    Site::Muster,
    Site::Snapshot,
    Site::Empty,
];

#[cfg(debug_assertions)]
const NSITE: usize = 7;

#[cfg(debug_assertions)]
const _: () = {
    let mut i = 0;
    while i < NSITE {
        assert!(ALL[i] as usize == i);
        i += 1;
    }
};

#[cfg(debug_assertions)]
impl Site {
    fn ix(self) -> usize {
        self as usize
    }

    fn counted(self) -> bool {
        self != Site::Empty
    }

    fn name(self) -> &'static str {
        match self {
            Site::Roster => "名册",
            Site::Holder => "票根",
            Site::TeamTasks => "团队簿记",
            Site::Sire => "血亲",
            Site::Muster => "抄件·muster",
            Site::Snapshot => "抄件·快照",
            Site::Empty => "空弱引用",
        }
    }
}

#[cfg(debug_assertions)]
const SLOTS: usize = 48;

#[cfg(debug_assertions)]
static SLOT_ID: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

#[cfg(debug_assertions)]
static SLOT_META: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

#[cfg(debug_assertions)]
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

#[cfg(debug_assertions)]
const SITE_BITS: u32 = 8;

#[cfg(debug_assertions)]
const _: () = assert!(NSITE <= 1 << SITE_BITS);

#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
struct SlotMeta(usize);

#[cfg(debug_assertions)]
impl SlotMeta {
    const EMPTY: SlotMeta = SlotMeta(0);

    fn pack(site: Site, hart: crate::hart::HartId) -> SlotMeta {
        let m = SlotMeta(site.ix() | (hart.get() << SITE_BITS));
        debug_assert!(
            m.site() == site && m.hart() == hart,
            "槽位元数据往返：出身或出生核在打包里丢了"
        );
        m
    }

    fn site(self) -> Site {
        ALL[self.0 & ((1 << SITE_BITS) - 1)]
    }

    fn hart(self) -> crate::hart::HartId {
        crate::hart::HartId::new(self.0 >> SITE_BITS)
    }

    fn word(self) -> usize {
        self.0
    }

    fn from_word(w: usize) -> SlotMeta {
        SlotMeta(w)
    }
}

#[cfg(debug_assertions)]
pub(crate) fn register(site: Site) -> usize {
    if !site.counted() {
        return 0;
    }
    let id = NEXT_ID.fetch_add(1, Relaxed);
    let meta = SlotMeta::pack(site, crate::hart::hart_id());
    for i in 0..SLOTS {
        if SLOT_ID[i].compare_exchange(0, id, Relaxed, Relaxed).is_ok() {
            SLOT_META[i].store(meta.word(), Relaxed);
            return id;
        }
    }
    0
}

#[cfg(debug_assertions)]
pub(crate) fn release(id: usize) {
    if id == 0 {
        return;
    }
    for i in 0..SLOTS {
        if SLOT_ID[i].load(Relaxed) == id {
            SLOT_ID[i].store(0, Relaxed);
            SLOT_META[i].store(SlotMeta::EMPTY.word(), Relaxed);
            return;
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) fn check_block_heldout() {
    let me = crate::hart::hart_id();
    for i in 0..SLOTS {
        if SLOT_ID[i].load(Relaxed) == 0 {
            continue;
        }
        let meta = SlotMeta::from_word(SLOT_META[i].load(Relaxed));
        let site = meta.site();
        if matches!(site, Site::Muster | Site::Snapshot) && meta.hart() == me {
            panic!(
                "[weak] 挂起自检：本核栈上仍有抄件（出身：{}）—— 跨挂起的弱引用会让外壳 \
                 永远归还不掉（`strong 0 weak 1`）",
                site.name()
            );
        }
    }
}
