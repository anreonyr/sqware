use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use env::TaskId;

use env::PieToken;

use crate::lock::{Level, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::life::Life;
use crate::work::unit::space::{SegmentKind, Space, Span};

use env::{MailFail, PieFail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PoleId(pub usize);

fn alloc_id() -> PoleId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    PoleId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoleState {
    Live,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    Frames,
    Region,
}

/// 一枚页：一段页级安全内存（`payload` 说它那几帧是自有还是外来区）＋ **页上那一位"有事"**。
///
/// # 页上为什么有"有事"
/// 一具架（`protocol::communication::rack`）是"一枚页 ＋ 一枚铃"两件东西；把**铃并进页**
/// 之后，一具架就只剩**一枚 Pie** ——树上一格门牌挂得下的正是它。那一位与孔/铃上那位同义：
/// 写者落完一格响一下（`ring`），读者读干应一下（`hush`），等它走 `wait`。
///
/// **它不碰中断闸门**（与 [`crate::work::mail::nole`] 的铃不同）：页上那一位不是中断响的，
/// 故 `envcall` 那一侧 `Hush` 的 Pole 支照孔那一支写、**不调 `sie::set_sext`**。
///
/// **一位（bool）而不是一列位**：架的写者每落一格响一次，而读者用的是"读干 → 应铃 → 复探
/// → 等"四拍（见 `rack::reader`）——重复的响本就该并成一枚，多响的那几次答 `Busy`、写者当
/// "正好"忽略。落一列位会把 `hush` 变成"应几次"，与那四拍多出没有读者的一档。
pub struct PoleMeta {
    state: SpinLock<PoleState>,
    payload: Payload,
    base: NonNull<u8>,
    size: usize,
    mappings: SpinLock<Vec<(PieToken, Weak<Space>, Span)>>,
    owner: TaskId,
    /// 这一枚在本机里的号（组、唤醒键用它——`PieToken` 是**表内**的号，跨域没有意义）。
    id: PoleId,
    /// 等待者随资源一起醒（`messenger` 要它；[`Drop`] 与 [`seal`] 都会 `wipe`）。
    life: Arc<Life>,
    /// 页上那一位"有事"。
    ring: SpinLock<bool>,
}

// SAFETY: PoleMeta 经 Arc 跨任务共享；base 指向共享物理帧
unsafe impl Send for PoleMeta {}
unsafe impl Sync for PoleMeta {}

impl PoleMeta {
    pub(super) fn allocate(size: usize, owner: TaskId) -> Result<Arc<Self>, PieFail> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(PieFail::NotAligned);
        }
        let layout = core::alloc::Layout::from_size_align(size, PAGE_SIZE)
            .map_err(|_| PieFail::NotAligned)?;
        let ptr = crate::tag!(
            Pole,
            frame::allocator()
                .allocate(layout)
                .map_err(|_| PieFail::OoM)?
        );
        // SAFETY: 分配返回非空
        let base = unsafe { NonNull::new_unchecked(ptr.as_ptr().cast::<u8>()) };
        unsafe {
            core::ptr::write_bytes(base.as_ptr(), 0, size);
        }
        Ok(Arc::new(Self {
            state: SpinLock::new_level(Level::L3, PoleState::Live),
            payload: Payload::Frames,
            base,
            size,
            mappings: SpinLock::new(Vec::new()),
            owner,
            id: alloc_id(),
            life: Life::new(),
            ring: SpinLock::new_level(Level::L3, false),
        }))
    }

    pub(super) fn region(base: usize, reg: usize, owner: TaskId) -> Result<Arc<Self>, PieFail> {
        if reg == 0 {
            return Err(PieFail::NotAligned);
        }
        let end = base.checked_add(reg).ok_or(PieFail::NotAligned)?;
        let lo = base & !(PAGE_SIZE - 1);
        let hi = end
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(PieFail::NotAligned)?;
        let base = NonNull::new(lo as *mut u8).ok_or(PieFail::NotAligned)?;
        let size = hi - lo;
        Ok(Arc::new(Self {
            state: SpinLock::new_level(Level::L3, PoleState::Live),
            payload: Payload::Region,
            base,
            size,
            mappings: SpinLock::new(Vec::new()),
            owner,
            id: alloc_id(),
            life: Life::new(),
            ring: SpinLock::new_level(Level::L3, false),
        }))
    }

    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }

    pub(crate) fn id(&self) -> PoleId {
        self.id
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == PoleState::Live
    }

    /// 页上那一位此刻亮着吗。
    pub(crate) fn ready(&self) -> bool {
        *self.ring.lock()
    }

    fn open_into(
        &self,
        token: PieToken,
        space: &Arc<Space>,
        flags: PteFlags,
    ) -> Result<usize, PieFail> {
        {
            let m = self.mappings.lock();
            if let Some((_, _, span)) = m.iter().find(|(t, _, _)| *t == token) {
                return Ok(span.va.as_usize());
            }
        }
        let va = space
            .with_flush(|inner| {
                let va = inner.allocate(SegmentKind::Normal, self.size)?;
                if let Err(e) = inner.borrow(
                    va,
                    PhysAddr::from_raw(self.base.as_ptr() as usize),
                    self.size,
                    flags,
                ) {
                    inner.deallocate(SegmentKind::Normal, va.as_usize(), self.size);
                    return Err(e);
                }
                Ok::<_, MapError>(va)
            })
            .map_err(|_| PieFail::OoM)?;
        let mut maps = self.mappings.lock();
        if maps.try_reserve(1).is_err() {
            drop(maps);
            let _ = space.release(Span::new(SegmentKind::Normal, va, self.size, None));
            return Err(PieFail::OoM);
        }
        maps.push((
            token,
            Arc::downgrade(space),
            Span::new(SegmentKind::Normal, va, self.size, None),
        ));
        Ok(va.as_usize())
    }

    fn narrow_into(&self, token: PieToken, flags: PteFlags) -> Result<(), PieFail> {
        let target = {
            let m = self.mappings.lock();
            m.iter()
                .find(|(t, _, _)| *t == token)
                .and_then(|(_, w, s)| {
                    w.upgrade()
                        .map(|space| (space, s.va.as_usize(), s.size.get()))
                })
        };
        if let Some((space, va, size)) = target {
            space
                .protect(VirtAddr::from_raw(va), size, flags)
                .map_err(|_| PieFail::Denied)?;
        }
        Ok(())
    }

    fn shut_from(&self, token: PieToken) -> Result<(), PieFail> {
        let (space, span) = {
            let mut m = self.mappings.lock();
            let pos = m.iter().position(|(t, _, _)| *t == token);
            match pos {
                Some(i) => {
                    let (_, w, s) = m.remove(i);
                    match w.upgrade() {
                        Some(space) => (space, s),
                        None => return Ok(()),
                    }
                }
                None => return Ok(()),
            }
        };
        space.release(span).map_err(|_| PieFail::Denied)
    }
}

impl Drop for PoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = PoleState::Dead;
        // 睡在"页上那一位"上面的读者随资源一起醒（与 nole 同一条）。
        messenger::wipe(key(self));
        let mappings: Vec<(PieToken, Weak<Space>, Span)> =
            core::mem::take(&mut *self.mappings.lock());
        for (_, weak, span) in mappings {
            if let Some(space) = weak.upgrade() {
                let _ = space.release(span);
            }
        }
        let layout =
            core::alloc::Layout::from_size_align(self.size, PAGE_SIZE).expect("pole layout valid");
        if self.payload == Payload::Frames {
            unsafe {
                frame::allocator().deallocate(self.base, layout);
            }
        }
    }
}

pub(crate) fn open(
    meta: &PoleMeta,
    token: PieToken,
    space: &Arc<Space>,
    flags: PteFlags,
) -> Result<(usize, usize), PieFail> {
    if !meta.alive() {
        return Err(PieFail::Dead);
    }
    let va = meta.open_into(token, space, flags)?;
    let _ = space.protect(VirtAddr::from_raw(va), meta.size, flags);
    Ok((va, meta.size))
}

pub(crate) fn shut(meta: &PoleMeta, token: PieToken) -> Result<(), PieFail> {
    meta.shut_from(token)
}

pub(crate) fn narrow(meta: &PoleMeta, token: PieToken, flags: PteFlags) -> Result<(), PieFail> {
    if !meta.alive() {
        return Err(PieFail::Dead);
    }
    meta.narrow_into(token, flags)
}

/// 页上那一位的唤醒键（组、`messenger` 用它；号是**本机**的 `PoleId`，不是表内 `PieToken`）。
pub(crate) fn key(meta: &PoleMeta) -> WakeKey {
    WakeKey::Pole { id: meta.id.0 }
}

/// 响一下：立起"有事"并唤醒等的人。**已响 ⇒ `Busy`**（写者当"正好"，不是失败）。
pub(crate) fn ring(meta: &PoleMeta) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    {
        let mut ring = meta.ring.lock();
        if *ring {
            return Err(MailFail::Busy);
        }
        *ring = true;
    }
    let _ = messenger::wake(key(meta), &meta.life());
    Ok(())
}

/// 应一下：清掉"有事"。**没响 ⇒ `Busy`**（读端当"正好"）。
pub(crate) fn hush(meta: &PoleMeta) -> Result<(), MailFail> {
    let mut ring = meta.ring.lock();
    if !*ring {
        return Err(MailFail::Busy);
    }
    *ring = false;
    Ok(())
}

/// 等那一位亮（照 `nole::wait`：`true` = 当场就绪、未挂起）。
pub(crate) fn wait(meta: &PoleMeta, dur: Duration) -> Result<Handoff<bool>, MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    if meta.ready() {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    Ok(match messenger::wait(key(meta), meta.life(), dur)? {
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready()),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

pub(crate) fn seal(meta: &PoleMeta) {
    *meta.state.lock() = PoleState::Dead;
    // 封印即"再也不会有事"：等的人立刻醒，下一次 `wait` 答 `Dead`。
    messenger::wipe(key(meta));
}

pub(crate) fn meta(size: usize, owner: TaskId) -> Result<Arc<PoleMeta>, PieFail> {
    PoleMeta::allocate(size, owner)
}

pub(crate) fn region(base: usize, reg: usize, owner: TaskId) -> Result<Arc<PoleMeta>, PieFail> {
    PoleMeta::region(base, reg, owner)
}
