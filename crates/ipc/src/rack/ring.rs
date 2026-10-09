//! 页上那一具环 —— **缓冲那一层**：页布局与两只游标的正文。
//!
//! 只借一枚 [`View`]（页首地址）：**不发系统调用、不分配、不认识"丢哪一头"这一档策略**
//! （那是写端的规矩，见 [`super::writer`]），也不认识铃（见 [`super::bell`]）。
//!
//! # 页上的环
//! ```text
//! 0   .. 64            头：write / read / dropped / lost（四个 u64 计数）
//! 64  .. SLOT_SIZE·CAP 格：每格 { seq: u64, len: u32, pad, data: [u8; SLOT] }
//! ```
//! **一条格自己的序号**（不是裸长度）一次消掉三件事：写者先写载荷、再 `Release` 落 `seq`、
//! 最后放行 `write` ⇒ 读者**读不到半格**；读者见 `seq > next` 即知中间那几格被丢了，
//! 自行跳号（跳了多少记在读端本地，见 [`super::reader`]）——于是**写者不必去动读游标**。
//!
//! # 两只游标各只有一个写者（这一条是这一层的全部纪律）
//! - `write`：**写端**写（[`push`] 最后一步，`Release`）。
//! - `read`：**读端**写（[`pop`] 每前进一号就落回来，`Release`）——它说"这一号我取走了"，
//!   故**只能在拷完载荷之后落**：落早了写端就可能把我们正在读的那一格顶掉。
//! 判满因此是 `write - read >= CAP`，两边都只有 `Acquire` / `Release`，无锁、无新内核对象。
//!
//! # 三个数各说各的
//! `lost` = **写端顶掉未读格**的条数（唯一写者是写端）；`dropped` = `Mode::Newest` 下没进架的
//! 条数；读端的"我跳过了几条"是它**本地**的账（见 [`super::reader::Reader::skipped`]），
//! 不往页里写——同一件事只记一次。

use core::sync::atomic::{AtomicU64, Ordering};

use env::PAGE_SIZE;
use ::resource::dock::View;

use super::Mode;
use wire::Message;

/// 架那一段：**连续四页**（`Pole::unseal` 只要求页对齐）。
///
/// **为什么不是一页**：这一具架是**跨域共享**的（手递出去的是"某一格在哪"，取回来时那一格
/// 可能已被后来的改动顶掉）。一页只放得下 7 格，而装配期一次改动挨着一次 ⇒ 慢一点的订户
/// 取回来时内容已是**别条路**的（实测：`got=0`，一条都认不出来）。四页给到 16 格，
/// 把"手上的那几格还活着"这个窗口从 7 次改动拉到 16 次——它仍是**一具**（与订户数无关）。
pub const SIZE: usize = 4 * PAGE_SIZE;
/// 这一段放得下几格（**2 的幂**：下标用与取模省一条除法）。**16 是这一段的上限**：
/// `HEAD ＋ CAP × SLOT_SIZE ≤ SIZE`，而 `SLOT_SIZE` 必须是 64 的整数倍（见下）。
/// （不必恰好装满：`HEAD ＋ 16 × 512 = 8256 ≤ 16384`。）
pub const CAP: usize = 16;
/// 头：四个 u64 计数（`write` / `read` / `dropped` / `lost`），**整 64 字节**——
/// 两件事一起说：它在自己的对齐上，且**第一格也从 64 起**（槽要 64 对齐）。
const HEAD: usize = 64;
/// **一格自己那一段**（`seq` 8 ＋ `len` 4 ＋ 补齐 ＋ `data`）＝ 64 的整数倍 ⇒ 每一格都落在
/// 自己的对齐上（槽里只放字节，对齐不外露）。
const SLOT_SIZE: usize = 512;
/// 一格能装多少字节（`M::MAX` 的上限，编译期由 [`exact`] 那一句把关）。
pub(crate) const SLOT: usize = SLOT_SIZE - 64;
/// 格下标那一枚掩码（CAP 是 2 的幂）。
const MASK: usize = CAP - 1;

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
pub(crate) struct Ring {
    write: AtomicU64,
    read: AtomicU64,
    dropped: AtomicU64,
    lost: AtomicU64,
    slots: [Slot; CAP],
}

/// 编译期：一页装得下头与 CAP 格（**不必恰好装满**），且 `Message::MAX` 塞得进一格。
pub(crate) const fn exact<M: Message>() {
    let _ = assert!(
        HEAD + SLOT_SIZE * CAP <= SIZE,
        "头 ＋ CAP 格超出一页（改 CAP / SLOT_SIZE 时要么减格、要么改页）"
    );
    let _ = assert!(SLOT_SIZE % 64 == 0, "一格要与 64 对齐");
    let _ = assert!(HEAD % 64 == 0, "头要与 64 对齐（第一格才在边界上）");
    let _ = assert!(M::MAX <= SLOT, "这一族的报比一格还长");
}

/// `View` 那一段就是页首：转成 `&Ring`（页由内核按页对齐造，`View::base` 是页首）。
///
/// SAFETY：映射活到 `Dock` 收起为止，调用方（两端）都持着同一枚页的映射；四个计数与每格
/// 的 `seq` 在整机范围内是 `AtomicU64`（页是共享内存，故两边看的是同一批帧）。
pub(crate) fn ring(view: View) -> &'static Ring {
    // SAFETY: 见上。
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
fn slot(ring: &Ring, i: usize) -> &Slot {
    &ring.slots[i & MASK]
}

/// 一格的载荷那一段。
fn payload(slot: &Slot) -> &[u8] {
    let n = slot.len as usize;
    slot.data.get(..n.min(SLOT)).unwrap_or(&[])
}

/// **环里第 `seq` 格此刻的载荷字节**（号从 1 起，写端每落一格加一）。
///
/// 写者刚写完就把它**借出去**（交给孔上那只手：手只登记"这一段在哪"，取走那一刻内核复制
/// 一次）。**只在环没绕回这一格之前有效**：绕回来之后这里就是更新的内容——故载荷里带一格
/// 自己的号（`Event::seq`），读者自己辨"这一条我读过没有、中间丢了几条"。
pub(crate) fn payload_at(ring: &Ring, seq: u64) -> &[u8] {
    if seq == 0 {
        return &[];
    }
    payload(slot(ring, (seq - 1) as usize))
}

/// 写者那一侧：让 `write` 那一格可见（`Release` ⇒ 载荷与 `seq` 先于游标）。
fn publish(ring: &Ring, seq: u64) {
    ring.write.store(seq, Ordering::Release);
}

/// **读端那一只游标的起始号**：`read + 1`（0 ⇒ 1）。
///
/// 读端**从页里起**（不是从 1 起）：同一具架换一个读端（重启、重建）时不重放已经取走的消息，
/// 而"已经取走"这件事只有页里那一格说得出来。
pub(crate) fn cursor(ring: &Ring) -> u64 {
    ring.read.load(Ordering::Acquire).wrapping_add(1)
}

/// 架上**还有几格**（**诊断用**：与读者那一刻看到的未必同值）。
///
/// 口径是**物理格数**：`write - read` 是"写端认为还没被读走的枚数"，而 `Mode::Oldest` 顶掉的
/// 那几枚也算在里面（它们永远不会被读到）⇒ 取 `CAP` 封顶，才是这一页此刻压着的条数。
pub(crate) fn depth(ring: &Ring) -> u64 {
    let w = ring.write.load(Ordering::Acquire);
    let r = ring.read.load(Ordering::Acquire);
    w.wrapping_sub(r).min(CAP as u64)
}

/// 被写端顶掉的未读格数。
pub(crate) fn lost(ring: &Ring) -> u64 {
    ring.lost.load(Ordering::Relaxed)
}

/// `Mode::Newest` 下没进架的条数。
pub(crate) fn dropped(ring: &Ring) -> u64 {
    ring.dropped.load(Ordering::Relaxed)
}

/// **落一格**（写者那一侧的全部正文；[`super::writer::Writer::send`] 编完帧就叫它）。
///
/// `Ok` = 进去了；`Err(())` = `Mode::Newest` 下满了、这一格被丢（写者只记 `dropped`）。
/// **顺序是契约**：载荷 → `len` → `seq`(Release) → `write`(Release)。
///
/// 满了那两档**都先记数再落格**：`Oldest` 顶掉的是"读者还没取的那一格"（`w - read >= CAP` ⇒
/// 下标 `w` 与 `read` 同余），故如实记一枚 `lost`；`Newest` 不落格、记一枚 `dropped`。
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

/// **读一格**（读者那一侧的全部正文；[`super::reader::Reader::try_recv`] 取到字节后解帧）。
///
/// `next` = 我该读的那一号（绝对序号）。返：
/// - `Ok(Some(n))` = 读到 `n` 字节，`*next` 已前进（页里的 `read` 也落回）；
/// - `Ok(None)` = 这一刻没有新的（看都没看那一格）；
/// - `Err(n)` = 读到 `n` 字节但放不下（`out` 那一格比它短），**照样前进**（不留死消息堵队）；
/// - `Err(0)` = 这一格被别的读者拿走了（不该发生：一页一端）。
///
/// 格序号比 `next` 大 ⇒ 中间那几格被写端覆盖丢了：跳过去，把跳过的枚数累到 `skipped`
/// （**页里那枚 `lost` 由写端记**，这里不记第二遍）。
pub(crate) fn pop(
    ring: &Ring,
    next: &mut u64,
    skipped: &mut u64,
    out: &mut [u8],
) -> Result<Option<usize>, usize> {
    let w = ring.write.load(Ordering::Acquire);
    if *next > w {
        return Ok(None);
    }
    for _ in 0..CAP {
        // **格下标是"号 − 1"**：`push` 那一侧写的正是 `w & MASK`（`w` 是"已落几格"），
        // 而 `next` 是那一格的**号**（从 1 起）——两处必须同一套坐标。
        let at = slot(ring, (*next - 1) as usize);
        let seq = at.seq.load(Ordering::Acquire);
        if seq == *next {
            let src = payload(at);
            let n = src.len();
            if out.len() < n {
                advance(ring, next);
                return Err(n);
            }
            out[..n].copy_from_slice(src);
            advance(ring, next);
            return Ok(Some(n));
        }
        if seq > *next {
            *skipped = skipped.saturating_add(seq - *next);
            *next = seq;
            continue;
        }
        // `seq < next`：这一格还是更旧的内容，没有新的。
        return Ok(None);
    }
    Ok(None)
}

/// 取走这一号：**先把读游标落回页里**（落的是刚取走的那一号），再让 `next` 前进。
///
/// 顺序是契约：载荷已经拷完才调它（`read` 一落回，写端就有权顶掉这一格）。
/// 页里那一格说的是"**我取到第几号**"（`w - read` 因此就是架上的条数，见 [`depth`]），
/// 读者手里那一枚 [`super::reader::Reader`] 的 `next` 才是"下一号"。
/// 跳号那一路**不**落：那几格早已被顶掉，而当前这一号还没读——落早了就把正在读的那一格
/// 交给写端了。（跳过的枚数由调用方记在自己身上。）
fn advance(ring: &Ring, next: &mut u64) {
    ring.read.store(*next, Ordering::Release);
    *next = next.wrapping_add(1);
}
