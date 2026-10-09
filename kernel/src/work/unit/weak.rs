use alloc::sync::Weak;
use core::ops::Deref;
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use super::task::Task;

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
}

#[cfg(debug_assertions)]
impl Site {
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
#[derive(Clone, Copy)]
struct SlotMeta(usize);

#[cfg(debug_assertions)]
const SITE_BITS: u32 = 8;

#[cfg(debug_assertions)]
const _: () = assert!(NSITE <= 1 << SITE_BITS);

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

#[repr(C)]
pub(crate) struct TaskWeak {
    w: Weak<Task>,
    id: usize,
    site: Site,
}

impl TaskWeak {
    pub(crate) fn stored(w: Weak<Task>, site: Site) -> TaskWeak {
        #[cfg(debug_assertions)]
        let id = if site.counted() { record(site) } else { 0 };
        #[cfg(not(debug_assertions))]
        let id = 0;
        TaskWeak { w, id, site }
    }

    pub(crate) fn copy_at(&self, site: Site) -> TaskWeak {
        TaskWeak::stored(self.w.clone(), site)
    }

    pub(crate) fn empty() -> TaskWeak {
        TaskWeak {
            w: Weak::new(),
            id: 0,
            site: Site::Empty,
        }
    }
}

impl Deref for TaskWeak {
    type Target = Weak<Task>;
    fn deref(&self) -> &Weak<Task> {
        &self.w
    }
}

impl Drop for TaskWeak {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if self.id != 0 {
            for i in 0..SLOTS {
                if SLOT_ID[i].load(Relaxed) == self.id {
                    SLOT_ID[i].store(0, Relaxed);
                    SLOT_META[i].store(SlotMeta::EMPTY.word(), Relaxed);
                    return;
                }
            }
        }
    }
}

#[cfg(debug_assertions)]
fn record(site: Site) -> usize {
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
