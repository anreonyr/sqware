//! 一具寄存架：一枚页上 N 格，满了按策略丢。
//!
//! 与 `communication::hand` 的关系：那边是**一手**（一枚孔上一条报，交完即走，满了只答
//! `Busy`）；这边把"同样的字节过边界"换成一枚**页**上的有界环（寄存），满了**按 `Mode`
//! 丢一头**并把丢掉的数记在册。两档共用 `crate::wire::message::Message` 那四样
//! （`Buf` / `EMPTY` / `store` / `fetch`），但**不共用类型**：失败域不同（`Full` 对 `Unbound`）。
//!
//! # 三条不变量
//! 1. **发送端永不挂起**：`send` 只有"放进去了"与"按策略丢了"两种下场 ⇒ 写者不会被慢读者
//!    堵住（对照 `hand::Sender::reclaim` 那条无期等）。
//! 2. **架是通知，不是账**：丢了几个由 `lost`（读者跳过）＋ `dropped`（写者按策略丢）两个数
//!    说出来；权威状态仍在拥有方那边（树那一侧是 `tile` / `token` / `list`）。
//! 3. **一页一端**：`Rack` 只开**一处**映射，读写两端各从它取一份；同一端不该再交出去
//!    第二份（页是多映射的，这条是用法纪律，类型保证不了）。
//!
//! # 页上的环
//! ```text
//! 0   .. 64            头：write / read / dropped / lost（四个 u64 计数）
//! 64  .. SLOT_SIZE·CAP 格：每格 { seq: u64, len: u32, pad, data: [u8; SLOT] }
//! ```
//! **一条格自己的序号**（不是裸长度）一次消掉三件事：写者先写载荷、再 `Release` 落 `seq`、
//! 最后放行 `write` ⇒ 读者**读不到半格**；读者见 `seq > r + 1` 即知中间那几格被丢了，
//! 自行跳号并记 `lost`——于是**写者不必去动读游标**（两个游标各只有一个写者，
//! 只有 `Acquire` / `Release`，无锁、无新内核对象）。
//!
//! # 与 `std::sync::mpsc` 的对应
//! `Rack` ↔ `sync_channel(n)`（那对句柄的所有者）；`Writer` ↔ `SyncSender`（但**不阻塞**、
//! 也没有 `Disconnected`——断了的下场是"写不进去"，说法是 `Full` 与 `lost` 计数）；
//! `Reader` ↔ `Receiver`（多一枚铃：`wait` 等"有事"，醒来还是要读环）。

pub mod reader;
pub mod writer;

use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, Ordering};

use env::{PieToken, PieResult};
use runtime::PAGE_SIZE;
use runtime::core::res::dock::{Dock, View};
use runtime::env::mail::{NolePie, PolePie};

use crate::wire::message::Message;

pub use self::reader::{Reader, RecvFail};
pub use self::writer::{SendFail, Writer};

/// 页就是架：一枚页一具架（`Pole::unseal` 要求页对齐，`PAGE_SIZE` 正好）。
pub const SIZE: usize = PAGE_SIZE;
/// 一页放得下几格。**取 8（2 的幂）**：下标用与取模省一条除法，且一页的格数本来就该小。
pub const CAP: usize = 8;
/// 头：四个 u64 计数，整 64 字节（故第一格也从 64 起）。
const HEAD: usize = 64;
/// 一格的整宽（`seq` 8 ＋ `len` 4 ＋ 补齐 ⇒ 与 64 对齐的整宽）。
const SLOT_SIZE: usize = (SIZE - HEAD) / CAP;
/// 一格能装多少字节（`M::MAX` 的上限，编译期由 [`exact`] 那一句把关）。
const SLOT: usize = SLOT_SIZE - 64;
/// 格下标那一枚掩码（CAP 是 2 的幂）。
const MASK: usize = CAP - 1;

/// 满了丢哪一头。默认 [`Mode::Oldest`]：最新那一格永远保得住（"树变了"比"从前变过"值钱）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// 丢最旧那一格（读者靠格序号跳过去，记 `lost`）。
    Oldest,
    /// 丢正要落的这一格（最新那一格进不来，记 `dropped`）。
    Newest,
}

/// 一格：自己的序号 ＋ 载荷长度 ＋ 载荷。
/// **`data` 是字节**：`Message::store` / `fetch` 本来就按字节走，故对齐不外露。
#[repr(C, align(64))]
struct Slot {
    seq: AtomicU64,
    len: u32,
    _pad: u32,
    data: [u8; SLOT],
}

/// 页上那个环（`#[repr(C, align(64))]` ＋ 页首 4096 对齐 ⇒ 四个计数与每格都在自己的对齐上）。
#[repr(C, align(64))]
struct Ring {
    write: AtomicU64,
    read: AtomicU64,
    dropped: AtomicU64,
    lost: AtomicU64,
    slots: [Slot; CAP],
}

/// 编译期：一页正好装下头与 CAP 格，且 `Message::MAX` 塞得进一格。
const fn exact<M: Message>() {
    let _ = assert!(
        SIZE == HEAD + SLOT_SIZE * CAP,
        "页容与格数对不上（改 CAP / SLOT_SIZE 时要让这两边相等）"
    );
    let _ = assert!(SLOT_SIZE % 64 == 0, "一格要与 64 对齐");
    let _ = assert!(M::MAX <= SLOT, "这一族的报比一格还长");
}

/// **一具架**：一枚页 ＋ 一枚铃。
///
/// 页是架本体（多映射：读写两端映同一段物理帧），铃是"有事"（写者每批响一次，读者等它）。
/// 两端由这一具架取（`writer()` / `reader()`），或由 [`Rack::ship`] 交出的两枚号在对端重建
/// （`Writer::from_token` / `Reader::from_token`，与 `Sender::from_token` 同形）。
pub struct Rack<M: Message> {
    /// 页（借映进本域，与 `Dock` 同一条手：`open` 返视图、`shut` 撤图）。
    dock: Dock,
    /// 铃：写者响、读者等。
    bell: NolePie,
    /// 满了丢哪一头（写那一侧的规矩，读端不必知道）。
    mode: Mode,
    _m: PhantomData<M>,
}

impl<M: Message> Rack<M> {
    /// 开一具架：**一页清过零的共享内存 ＋ 一枚铃**，`mode` 定满了丢哪一头。
    ///
    /// # Errors
    /// 页或铃解封不出来（`PieFail`）——起手那一步没材料，调用方按"这一档用不了"处置。
    pub fn open(mode: Mode) -> PieResult<Self> {
        exact::<M>();
        let pie = PolePie::unseal(SIZE)?;
        let dock = Dock::open(pie)?;
        // 页是内核清零的（`UnsealPole` 的正文），故四个计数与每格 `seq` 都从 0 起。
        Ok(Self {
            dock,
            bell: NolePie::unseal()?,
            mode,
            _m: PhantomData,
        })
    }

    /// 写端（与 std 的 `SyncSender` 同位）。
    pub fn writer(&self) -> Writer<M> {
        Writer::of(self.dock.view(), self.mode, self.bell.token())
    }

    /// 读端（与 std 的 `Receiver` 同位）。
    pub fn reader(&self) -> Reader<M> {
        Reader::of(self.dock.view(), self.bell.token())
    }

    /// **交出去的那两枚号**：`(页, 铃)`。对端拿它重建自己的那一端。
    pub fn ship(&self) -> (PieToken, PieToken) {
        (self.dock.pie_token(), self.bell.token())
    }
}

/// `View` 那一段就是页首：转成 `&Ring`（页由内核按页对齐造，`View::base` 是页首）。
///
/// SAFETY：映射活到 `Dock` 收起为止，调用方（两端）都持着同一枚页的映射；四个计数与每格
/// 的 `seq` 在整机范围内是 `AtomicU64`（页是共享内存，故两边看的是同一批帧）。
pub(crate) fn ring(view: View) -> &'static Ring {
    // SAFETY：见上。
    unsafe { &*(view.base() as *const Ring) }
}

/// 一格（`i` 是格下标）。**返的是一枚独占借用**：`push` 要写载荷与 `len` 那两格，
/// 而同一时刻只有写者一个（一页一端）。读者只借不写，仍走 [`slot`] 的 `&` 那一形。
fn slot_mut(ring: &'static Ring, i: usize) -> &'static mut Slot {
    // SAFETY：页是共享内存，整机只有一个写者（`Rack` 那一侧的"一页一端"是用法纪律）；
    // 写者只写自己那一格（`w & MASK`），且写之前先把游标读出来定下这一格。
    unsafe { &mut *(core::ptr::addr_of!(ring.slots[i & MASK]) as *mut Slot) }
}

/// 一格（`i` 是格下标）。
pub(crate) fn slot(ring: &Ring, i: usize) -> &Slot {
    &ring.slots[i & MASK]
}

/// 一格的载荷那一段。
pub(crate) fn payload(slot: &Slot) -> &[u8] {
    let n = slot.len as usize;
    slot.data.get(..n.min(SLOT)).unwrap_or(&[])
}

/// 写者那一侧：读一眼读者走到哪了（诊断用：不写、不改）。
pub(crate) fn read_cursor(ring: &Ring) -> u64 {
    ring.read.load(Ordering::Acquire)
}

/// 写者那一侧：让 `write` 那一格可见（`Release` ⇒ 载荷与 `seq` 先于游标）。
pub(crate) fn publish(ring: &Ring, seq: u64) {
    ring.write.store(seq, Ordering::Release);
}

/// **落一格**（写者那一侧的全部正文；`Writer::send` 编完帧就叫它）。
///
/// `Ok` = 进去了；`Err(())` = `Mode::Newest` 下满了、这一格被丢（写者只记 `dropped`）。
/// **顺序是契约**：载荷 → `len` → `seq`(Release) → `write`(Release)。
pub(crate) fn push(ring: &'static Ring, mode: Mode, bytes: &[u8]) -> Result<(), ()> {
    let w = ring.write.load(Ordering::Relaxed);
    let r = ring.read.load(Ordering::Acquire);
    if w.wrapping_sub(r) >= CAP as u64 {
        match mode {
            // 丢最旧那一格：**只记数、不动读游标**——读者见格序号跳过去。
            Mode::Oldest => {
                ring.lost.fetch_add(1, Ordering::Relaxed);
            }
            Mode::Newest => {
                ring.dropped.fetch_add(1, Ordering::Relaxed);
                return Err(());
            }
        }
    }
    let at = slot_mut(ring, w as usize);
    let Some(dst) = at.data.get_mut(..bytes.len()) else {
        // 调用方已按 `SLOT` 判过长度，故到不了这里。
        return Err(());
    };
    dst.copy_from_slice(bytes);
    at.len = bytes.len() as u32;
    at.seq.store(w.wrapping_add(1), Ordering::Release);
    publish(ring, w.wrapping_add(1));
    Ok(())
}

/// **读一格**（读者那一侧的全部正文；`Reader::try_recv` 取到字节后解帧）。
///
/// `next` = 我该读的那一号（绝对序号）。返：
/// - `Ok(Some(n))` = 读到 `n` 字节，`*next` 已前进；
/// - `Ok(None)` = 这一刻没有新的（看都没看那一格）；
/// - `Err(n)` = 读到 `n` 字节但解不动（照样前进，不留死消息堵队）；
/// - `Err(0)` = 这一格被别的读者拿走了（不该发生：一页一端）。
///
/// 格序号比 `next` 大 ⇒ 中间那几格被写者覆盖丢了，跳过并记 `lost`。
pub(crate) fn pop(ring: &Ring, next: &mut u64, out: &mut [u8]) -> Result<Option<usize>, usize> {
    let w = ring.write.load(Ordering::Acquire);
    if *next > w {
        return Ok(None);
    }
    for _ in 0..CAP {
        let at = slot(ring, *next as usize);
        let seq = at.seq.load(Ordering::Acquire);
        if seq == *next {
            let src = payload(at);
            let n = src.len();
            if out.len() < n {
                *next = next.wrapping_add(1);
                return Err(n);
            }
            out[..n].copy_from_slice(src);
            *next = next.wrapping_add(1);
            return Ok(Some(n));
        }
        if seq > *next {
            ring.lost.fetch_add(seq - *next, Ordering::Relaxed);
            *next = seq;
            continue;
        }
        // `seq < next`：这一格还是更旧的内容，没有新的。
        return Ok(None);
    }
    Ok(None)
}

// 本 crate 的 `test = false`（`crates/protocol/Cargo.toml` 头注）：目标 `riscv64gc-unknown-none-elf`
// 上编不出 libtest，故环那一套**在这里没有用例**——它由 `programs` 那一档的探针在真机上量
// （`probe-watch`：收到 / 过滤 / 丢最旧三个签名的读数）。
