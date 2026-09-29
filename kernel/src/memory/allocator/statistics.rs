use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::boxed::Box;
#[cfg(debug_assertions)]
use alloc::vec::Vec;

use crate::lock::OnceLock;

#[cfg(debug_assertions)]
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Plain = 0,
    Trap,
    Lazy,
    Heap,
    Stack,
    Image,
    Pole,
    Table,
    TrapStack,
    HartFrame,
    Spare,
    Prime,
    Probe,
    Task,
    Team,
    Space,
}

#[cfg(debug_assertions)]
impl Kind {
    pub(crate) const COUNT: usize = 16;

    pub(crate) const ALL: [Kind; Kind::COUNT] = [
        Kind::Plain,
        Kind::Trap,
        Kind::Lazy,
        Kind::Heap,
        Kind::Stack,
        Kind::Image,
        Kind::Pole,
        Kind::Table,
        Kind::TrapStack,
        Kind::HartFrame,
        Kind::Spare,
        Kind::Prime,
        Kind::Probe,
        Kind::Task,
        Kind::Team,
        Kind::Space,
    ];

    pub(crate) const fn ix(self) -> usize {
        self as usize
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Kind::Plain => "plain",
            Kind::Trap => "trap",
            Kind::Lazy => "lazy",
            Kind::Heap => "heap",
            Kind::Stack => "stack",
            Kind::Image => "image",
            Kind::Pole => "pole",
            Kind::Table => "table",
            Kind::TrapStack => "trap-stack",
            Kind::HartFrame => "hart-frame",
            Kind::Spare => "spare",
            Kind::Prime => "prime",
            Kind::Probe => "probe",
            Kind::Task => "task",
            Kind::Team => "team",
            Kind::Space => "space",
        }
    }
}

#[cfg(debug_assertions)]
struct MarkCell(core::cell::Cell<Kind>);

// SAFETY: 每核只碰自己那一格
#[cfg(debug_assertions)]
unsafe impl Sync for MarkCell {}

#[cfg(debug_assertions)]
static MARK: [MarkCell; crate::layout::MAX_HART_SLOTS] =
    [const { MarkCell(core::cell::Cell::new(Kind::Plain)) }; crate::layout::MAX_HART_SLOTS];

#[cfg(debug_assertions)]
fn mark_slot() -> &'static MarkCell {
    &MARK[crate::hart::hart_id()
        .get()
        .min(crate::layout::MAX_HART_SLOTS - 1)]
}

#[cfg(debug_assertions)]
#[must_use = "标注靠守卫的生存期起作用：立刻丢掉等于没标"]
pub(crate) fn mark(kind: Kind) -> Mark {
    Mark {
        prev: mark_slot().0.replace(kind),
    }
}

#[cfg(debug_assertions)]
pub(crate) fn current() -> Kind {
    mark_slot().0.get()
}

#[cfg(debug_assertions)]
pub(crate) struct Mark {
    prev: Kind,
}

#[cfg(debug_assertions)]
impl Drop for Mark {
    fn drop(&mut self) {
        mark_slot().0.set(self.prev);
    }
}

#[cfg(debug_assertions)]
#[macro_export]
macro_rules! tag {
    ($kind:ident, $v:expr) => {{
        let _m = $crate::memory::allocator::statistics::mark(
            $crate::memory::allocator::statistics::Kind::$kind,
        );
        $v
    }};
}

#[cfg(not(debug_assertions))]
#[macro_export]
macro_rules! tag {
    ($kind:ident, $v:expr) => {
        $v
    };
}

#[cfg(debug_assertions)]
struct FrameKinds {
    ptr: *mut Kind,
    len: usize,
}

// SAFETY: 句柄装配一次后不变；所指内存只经 record_* 在帧分配器锁内读写
#[cfg(debug_assertions)]
unsafe impl Send for FrameKinds {}
#[cfg(debug_assertions)]
unsafe impl Sync for FrameKinds {}

#[cfg(debug_assertions)]
static FRAME_KINDS: OnceLock<FrameKinds> = OnceLock::new();

#[cfg(debug_assertions)]
pub(crate) fn install_frame_kinds(frames: usize) -> Result<(), super::InitError> {
    let mut v: Vec<Kind> = Vec::new();
    v.try_reserve(frames)
        .map_err(|_| super::InitError::OutOfMemory)?;
    v.resize(frames, Kind::Plain);
    let slice: &'static mut [Kind] = Box::leak(v.into_boxed_slice());
    let h = FrameKinds {
        ptr: slice.as_mut_ptr(),
        len: slice.len(),
    };
    FRAME_KINDS
        .set(h)
        .map_err(|_| super::InitError::AlreadyInitialized)
}

#[cfg(not(debug_assertions))]
pub(crate) fn install_frame_kinds(_frames: usize) -> Result<(), super::InitError> {
    Ok(())
}

#[cfg(debug_assertions)]
fn frame_kind_table() -> &'static mut [Kind] {
    let h = FRAME_KINDS
        .get()
        .expect("frame kinds not installed (frame::init 应先调 install_frame_kinds)");
    // SAFETY: 指针来自 Box::leak；独占性由调用方持帧锁保证
    unsafe { core::slice::from_raw_parts_mut(h.ptr, h.len) }
}

#[cfg(debug_assertions)]
fn note_frame_kind(index: usize, frames: usize, k: Kind) {
    frame_kind_table()[index..index + frames].fill(k);
}

#[cfg(debug_assertions)]
fn frame_kind_at(index: usize) -> Kind {
    frame_kind_table()[index]
}

#[cfg(debug_assertions)]
pub(crate) const BLOCK_KIND_SHIFT: usize = 7;

#[cfg(debug_assertions)]
struct BlockKinds {
    ptr: *mut Kind,
    len: usize,
    base: usize,
}

#[cfg(debug_assertions)]
unsafe impl Send for BlockKinds {}
#[cfg(debug_assertions)]
unsafe impl Sync for BlockKinds {}

#[cfg(debug_assertions)]
static BLOCK_KINDS: OnceLock<BlockKinds> = OnceLock::new();

#[cfg(debug_assertions)]
pub(crate) fn install_block_kinds(base: usize, len: usize) -> Result<(), super::InitError> {
    let slots = len >> BLOCK_KIND_SHIFT;
    let mut v: Vec<Kind> = Vec::new();
    v.try_reserve(slots)
        .map_err(|_| super::InitError::OutOfMemory)?;
    v.resize(slots, Kind::Plain);
    let slice: &'static mut [Kind] = Box::leak(v.into_boxed_slice());
    let h = BlockKinds {
        ptr: slice.as_mut_ptr(),
        len: slice.len(),
        base,
    };
    BLOCK_KINDS
        .set(h)
        .map_err(|_| super::InitError::AlreadyInitialized)
}

#[cfg(not(debug_assertions))]
pub(crate) fn install_block_kinds(_base: usize, _len: usize) -> Result<(), super::InitError> {
    Ok(())
}

#[cfg(debug_assertions)]
fn block_kind_table() -> (&'static mut [Kind], usize) {
    let h = BLOCK_KINDS
        .get()
        .expect("block kinds not installed (block::init 应先调 install_block_kinds)");
    let t = unsafe { core::slice::from_raw_parts_mut(h.ptr, h.len) };
    (t, h.base)
}

#[cfg(debug_assertions)]
fn note_block_kind(addr: usize, power: usize, k: Kind) {
    if power < BLOCK_KIND_SHIFT {
        return;
    }
    let (t, base) = block_kind_table();
    let first = (addr - base) >> BLOCK_KIND_SHIFT;
    t[first..first + (1 << (power - BLOCK_KIND_SHIFT))].fill(k);
}

#[cfg(debug_assertions)]
fn block_kind_at(addr: usize, power: usize) -> Kind {
    if power < BLOCK_KIND_SHIFT {
        return Kind::Plain;
    }
    let (t, base) = block_kind_table();
    t[(addr - base) >> BLOCK_KIND_SHIFT]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    AlreadyInitialized,
}

struct Stats {
    frame_occupied: AtomicUsize,
    block_occupied: AtomicUsize,
    spare_occupied: AtomicUsize,
    spare_total: AtomicUsize,
    #[cfg(debug_assertions)]
    kinds: [AtomicUsize; Kind::COUNT],
}

static STATS: OnceLock<&'static Stats> = OnceLock::new();

fn stats() -> &'static Stats {
    STATS.get().expect("statistics not initialized")
}

pub fn init() -> Result<(), Error> {
    if STATS.get().is_some() {
        return Err(Error::AlreadyInitialized);
    }
    let s: &'static Stats = Box::leak(Box::new(Stats {
        frame_occupied: AtomicUsize::new(0),
        block_occupied: AtomicUsize::new(0),
        spare_occupied: AtomicUsize::new(0),
        spare_total: AtomicUsize::new(0),
        #[cfg(debug_assertions)]
        kinds: [const { AtomicUsize::new(0) }; Kind::COUNT],
    }));
    STATS.set(s).map_err(|_| Error::AlreadyInitialized)?;
    Ok(())
}

pub(crate) fn record_frame_take(index: usize, power: usize) {
    let s = stats();
    s.frame_occupied.fetch_add(1, Ordering::Relaxed);
    #[cfg(debug_assertions)]
    {
        let k = current();
        note_frame_kind(index, 1 << power, k);
        s.kinds[k.ix()].fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = (index, power);
}

pub(crate) fn record_frame_give(index: usize) {
    let s = stats();
    s.frame_occupied.fetch_sub(1, Ordering::Relaxed);
    #[cfg(debug_assertions)]
    {
        let k = frame_kind_at(index);
        s.kinds[k.ix()].fetch_sub(1, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = index;
}

pub(crate) fn record_block_take(addr: usize, power: usize) {
    #[cfg(debug_assertions)]
    {
        let k = if power < BLOCK_KIND_SHIFT {
            Kind::Plain
        } else {
            let k = current();
            note_block_kind(addr, power, k);
            k
        };
        stats().kinds[k.ix()].fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = (addr, power);
}

pub(crate) fn record_block_give(addr: usize, power: usize) {
    #[cfg(debug_assertions)]
    {
        let k = block_kind_at(addr, power);
        stats().kinds[k.ix()].fetch_sub(1, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = (addr, power);
}

pub(crate) fn record_pool_take() {
    stats().block_occupied.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_pool_give() {
    stats().block_occupied.fetch_sub(1, Ordering::Relaxed);
}

pub(crate) fn record_spare_take(bytes: usize) {
    stats().spare_occupied.fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn record_spare_give(bytes: usize) {
    stats().spare_occupied.fetch_sub(bytes, Ordering::Relaxed);
}

pub(crate) fn record_spare_total(total: usize) {
    records(|s| s.spare_total.store(total, Ordering::Relaxed));
}

#[cfg(debug_assertions)]
pub fn frame_occupied() -> usize {
    stats().frame_occupied.load(Ordering::Relaxed)
}

#[cfg(debug_assertions)]
pub fn block_occupied() -> usize {
    stats().block_occupied.load(Ordering::Relaxed)
}

#[cfg(debug_assertions)]
pub fn spare_occupied() -> usize {
    stats().spare_occupied.load(Ordering::Relaxed)
}

#[cfg(debug_assertions)]
pub fn spare_available() -> usize {
    let s = stats();
    s.spare_total
        .load(Ordering::Relaxed)
        .saturating_sub(s.spare_occupied.load(Ordering::Relaxed))
}

fn records(f: impl FnOnce(&Stats)) {
    if let Some(s) = STATS.get() {
        f(s);
    }
}

#[cfg(debug_assertions)]
pub(crate) fn kinds() -> Kinds {
    let s = stats();
    let mut out = [0usize; Kind::COUNT];
    for (i, c) in s.kinds.iter().enumerate() {
        out[i] = c.load(Ordering::Relaxed);
    }
    Kinds(out)
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
pub(crate) struct Kinds([usize; Kind::COUNT]);

#[cfg(debug_assertions)]
impl Kinds {
    pub(crate) fn get(&self, k: Kind) -> usize {
        self.0[k.ix()]
    }

    pub(crate) fn nonzero(&self) -> impl Iterator<Item = (Kind, usize)> + '_ {
        Kind::ALL
            .into_iter()
            .enumerate()
            .filter_map(move |(i, k)| (self.0[i] != 0).then_some((k, self.0[i])))
    }
}

#[cfg(debug_assertions)]
impl core::fmt::Display for Kinds {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut any = false;
        for (k, n) in self.nonzero() {
            if any {
                f.write_str(" ")?;
            }
            any = true;
            write!(f, "{}={n}", k.name())?;
        }
        if !any {
            f.write_str("(空)")?;
        }
        Ok(())
    }
}
