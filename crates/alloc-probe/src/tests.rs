//! 分配器自己的用例 —— **完全不涉及内核对象生命周期**。
//!
//! 判据分三层：
//!   ① 分配器的护栏（`checker`）自己会在违例处炸（双 free / 放没在手的帧 / 链成环…）；
//!   ② 本文件用**影子账**独立复核每一次交付（同一个帧不得发两次、区间不得重叠）；
//!   ③ 收尾对账：把借出去的都还回去之后，分配器的**自有簿记**应回到起点
//!      （这一条正是"分配器自己有没有漏"的判据，与"谁没 drop"无关）。

use core::alloc::{Allocator, Layout};
use std::collections::HashMap;

use crate::machine;
use crate::memory::allocator::{block, bump, frame};
use crate::memory::PAGE_SIZE;

/// 用例收尾：smartalloc 档把孤儿缓冲打出来（其它档 no-op）。
fn done() {
    crate::dump_orphans();
}

/// **架空内核分配器的那条路**：这一段内存完全由 smartalloc 提供与结算，
/// 内核对它一无所知 —— 于是"漏没漏"由 smartalloc 答。
#[cfg(feature = "smartalloc")]
#[test]
fn smartalloc_owns_this_path() {
    // 借了又还：不该留下任何孤儿缓冲。
    // SAFETY: 同一 bytes、只归还一次。
    unsafe {
        let p = crate::smart::alloc(64);
        assert!(!p.is_null());
        crate::smart::free(p, 64);
    }
    done();
}

/// 一块冒充"物理内存"的宿主缓冲：16 MiB、页对齐。
struct Arena {
    v: Vec<u8>,
    base: usize,
    size: usize,
}

impl Arena {
    fn new(pages: usize) -> Arena {
        let size = pages * PAGE_SIZE;
        // 多要一页，取其中页对齐的起点；保证 base 与 base+size 都在缓冲内。
        let mut v: Vec<u8> = vec![0; size + PAGE_SIZE];
        let raw = v.as_mut_ptr() as usize;
        let base = (raw + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        Arena { v, base, size }
    }
}

/// 影子账：帧分配器交出来的每一页记一次，重复即分配器缺陷。
#[derive(Default, Debug)]
struct Shadow {
    live: HashMap<usize, usize>, // 页地址 → 记账次数
    handed: usize,
}

impl Shadow {
    fn take(&mut self, addr: usize, len: usize) {
        for p in (addr..addr + len).step_by(PAGE_SIZE) {
            let n = self.live.entry(p).or_insert(0);
            *n += 1;
            assert_eq!(*n, 1, "同一帧被交付了两次：{p:#x}（分配器缺陷，不是泄漏）");
        }
        self.handed += len / PAGE_SIZE;
    }
    fn give(&mut self, addr: usize, len: usize) {
        for p in (addr..addr + len).step_by(PAGE_SIZE) {
            let n = self.live.get_mut(&p).expect("释放了从未交付的帧（分配器缺陷）");
            *n -= 1;
            if *n == 0 {
                self.live.remove(&p);
            }
        }
    }
    fn live_pages(&self) -> usize {
        self.live.len()
    }
}

/// 全局只装一次台（`bump`/`frame`/`block` 都是进程内单例）。
fn boot() -> &'static Arena {
    use std::sync::OnceLock;
    static ARENA: OnceLock<Arena> = OnceLock::new();
    static INIT: OnceLock<()> = OnceLock::new();
    let a = ARENA.get_or_init(|| Arena::new(4096));
    INIT.get_or_init(|| {
        machine::configure(a.base, a.size, 4);
        // 顺序照内核 `hybrid::init()`：bump（元数据）→ block（池 + 簿记表）→ frame。
        // **block 必须在 frame 之前**（block.rs 的 `init` 文档：池的页要向 frame 借，
        // 而簿记表要覆盖 free 区；顺序反了帧链的链头就是毒化内存 —— 实测过）。
        bump::init().expect("bump::init");
        block::init().expect("block::init");
        frame::init().expect("frame::init");
    });
    a
}

#[test]
fn frame_alloc_free_round_trip() {
    let _a = boot();
    let f = frame::allocator();
    let mut shadow = Shadow::default();

    // 单帧、不同 order（4 KiB…128 KiB），记下每一次交付。
    let mut got: Vec<(usize, usize)> = Vec::new();
    for &bytes in &[PAGE_SIZE, 2 * PAGE_SIZE, 8 * PAGE_SIZE, 32 * PAGE_SIZE] {
        let layout = Layout::from_size_align(bytes, PAGE_SIZE).unwrap();
        let p = f.allocate(layout).expect("allocate").cast::<u8>().as_ptr() as usize;
        shadow.take(p, bytes);
        got.push((p, bytes));
    }
    // 逆序归还（逼出伙伴合并）。
    for (p, bytes) in got.into_iter().rev() {
        let layout = Layout::from_size_align(bytes, PAGE_SIZE).unwrap();
        // SAFETY: 这块正是上面 allocate 出来的同一段。
        unsafe { f.deallocate(core::ptr::NonNull::new(p as *mut u8).unwrap(), layout) };
        shadow.give(p, bytes);
    }
    assert_eq!(shadow.live_pages(), 0, "归还完毕仍有在册帧：{shadow:?}");
    done();
}

#[test]
fn frame_no_double_delivery_under_churn() {
    let _a = boot();
    let f = frame::allocator();
    let mut shadow = Shadow::default();

    // 混合尺寸的取/还（含中间释放），影子账会抓住"同一帧发两次"。
    let sizes = [PAGE_SIZE, 4 * PAGE_SIZE, 16 * PAGE_SIZE];
    let mut held: Vec<(usize, usize)> = Vec::new();
    for round in 0..64usize {
        let bytes = sizes[round % sizes.len()];
        let layout = Layout::from_size_align(bytes, PAGE_SIZE).unwrap();
        let p = f.allocate(layout).expect("allocate").cast::<u8>().as_ptr() as usize;
        shadow.take(p, bytes);
        held.push((p, bytes));
        if held.len() > 8 {
            let (q, b) = held.remove(0);
            let l = Layout::from_size_align(b, PAGE_SIZE).unwrap();
            // SAFETY: 同一次 allocate 的产物。
            unsafe { f.deallocate(core::ptr::NonNull::new(q as *mut u8).unwrap(), l) };
            shadow.give(q, b);
        }
    }
    for (p, b) in held {
        let l = Layout::from_size_align(b, PAGE_SIZE).unwrap();
        // SAFETY: 同上。
        unsafe { f.deallocate(core::ptr::NonNull::new(p as *mut u8).unwrap(), l) };
        shadow.give(p, b);
    }
    assert_eq!(shadow.live_pages(), 0);
    // 中间帧判据（内核那边随 `fence` 一起删了）；宿主侧由影子账兜住：
    // 交付过的帧若被当成块首再放一次，`Shadow::give` 会当场炸。
    assert_eq!(shadow.live_pages(), 0, "收尾仍有在册帧：{shadow:?}");
    done();
}

#[test]
fn block_pool_alloc_free_round_trip() {
    let _a = boot();
    let b = block::allocator();
    let mut live: Vec<(usize, usize)> = Vec::new();
    for &bytes in &[16usize, 64, 256, 1024, 2048, 300] {
        let layout = Layout::from_size_align(bytes, 8).unwrap();
        let p = b.allocate(layout).expect("block allocate").cast::<u8>().as_ptr() as usize;
        // 交付区间不得与任何在册块重叠（块池的"表在手 = 账在手"）。
        for &(q, n) in &live {
            assert!(
                p + bytes <= q || q + n <= p,
                "块池交付了重叠区间：{p:#x}+{bytes} 与 {q:#x}+{n}（分配器缺陷）"
            );
        }
        live.push((p, bytes));
    }
    for (p, bytes) in live.into_iter().rev() {
        let layout = Layout::from_size_align(bytes, 8).unwrap();
        // SAFETY: 上面同一次 allocate 的产物。
        unsafe { b.deallocate(core::ptr::NonNull::new(p as *mut u8).unwrap(), layout) };
    }
    done();
}
