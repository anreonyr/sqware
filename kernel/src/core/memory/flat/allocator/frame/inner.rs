use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::flat::allocator::{InitError, Link, bump};

static STALE_RM: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

pub(super) struct FrameInner {
    freelist: Vec<Option<NonNull<Link>>>,
    pagemeta: Vec<Option<u8>>,
    pub(super) base: usize,
    pub(super) edge: usize,
}

impl FrameInner {
    pub(super) const fn new() -> Self {
        Self {
            freelist: Vec::new(),
            pagemeta: Vec::new(),
            base: 0,
            edge: 0,
        }
    }

    pub(super) fn init(&mut self) -> Result<(), InitError> {
        self.edge = bump::boundary();
        let prov_base = bump::frontier().next_multiple_of(PAGE_SIZE);
        let max_frame = self.edge.saturating_sub(prov_base) / PAGE_SIZE;
        if max_frame == 0 {
            return Err(InitError::NoFreeFrames);
        }
        let max_power = max_frame.ilog2() as usize + 1;
        self.freelist
            .try_reserve(max_power)
            .map_err(|_| InitError::OutOfMemory)?;
        self.freelist.resize_with(max_power, || None);
        self.pagemeta
            .try_reserve(max_frame)
            .map_err(|_| InitError::OutOfMemory)?;
        self.pagemeta.resize_with(max_frame, || None);
        crate::core::memory::flat::allocator::statistics::install_frame_kinds(max_frame)?;

        self.base = bump::frontier().next_multiple_of(PAGE_SIZE);
        let max_frame = self.edge.saturating_sub(self.base) / PAGE_SIZE;
        if max_frame == 0 {
            return Err(InitError::NoFreeFrames);
        }
        let max_power = max_frame.ilog2() as usize + 1;
        self.freelist.resize_with(max_power, || None);
        self.pagemeta.resize_with(max_frame, || None);

        let mut index = 0usize;
        let mut remaining = max_frame;
        let holes = Self::holes(self.base, self.edge, max_frame);
        while remaining > 0 {
            if let Some(end) = hole_containing(&holes, index) {
                let skip = end - index;
                index += skip;
                remaining -= skip;
                continue;
            }
            let limit = holes
                .iter()
                .flatten()
                .map(|&(start, _)| start)
                .filter(|&start| start > index)
                .min()
                .unwrap_or(max_frame);
            let mut power = (index.trailing_zeros() as usize)
                .min(remaining.ilog2() as usize)
                .min(max_power - 1);
            while index + (1 << power) > limit {
                power -= 1;
            }
            unsafe {
                self.push_link(index, power);
            }
            index += 1 << power;
            remaining -= 1 << power;
        }
        Ok(())
    }

    pub(super) fn frame_index(&self, addr: usize) -> usize {
        (addr - self.base) / PAGE_SIZE
    }

    /// 最低非空伙伴桶的幂。只读：不改自由链、不分配、不归还。
    #[cfg(all(debug_assertions, feature = "mega-leaf"))]
    pub(super) fn lowest_free_power(&self) -> Option<usize> {
        self.freelist.iter().position(Option::is_some)
    }

    pub(super) fn frame_addr(&self, index: usize) -> usize {
        self.base + index * PAGE_SIZE
    }

    fn holes(
        base: usize,
        edge: usize,
        max_frame: usize,
    ) -> [Option<(usize, usize)>; crate::machine::board::MAX_RESERVED] {
        let mut out = [None; crate::machine::board::MAX_RESERVED];
        let mut n = 0;
        for r in crate::machine::board::info().reserved.iter().flatten() {
            let (hs, he) = (r.base, r.base + r.size);
            if he <= base || hs >= edge {
                continue;
            }
            let start = (hs.max(base) - base) / PAGE_SIZE;
            let end = (he.min(edge) - base).div_ceil(PAGE_SIZE);
            out[n] = Some((start.min(max_frame), end.min(max_frame)));
            n += 1;
        }
        out[..n].sort_unstable();
        out
    }

    fn buddy_index(index: usize, power: usize) -> usize {
        index ^ (1 << power)
    }

    fn clear_head(&mut self, index: usize) {
        self.pagemeta[index] = None;
    }

    unsafe fn pull_link(&mut self, power: usize) -> Option<usize> {
        unsafe {
            let head = self.freelist[power]?;

            let addr = head.addr().get();
            let index = self.frame_index(addr);
            if index >= self.pagemeta.len() {
                static BAD: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                let n = BAD.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                if n <= 8 {
                    crate::putln!(
                        "frame freelist corrupt: pop index={index:#x} len={} power={power} ({n})",
                        self.pagemeta.len()
                    );
                }
                return None;
            }

            let next = head.read().next;
            self.freelist[power] = next;
            if let Some(n) = next {
                (*n.as_ptr()).prev = None;
            }

            self.clear_head(index);
            Some(index)
        }
    }

    unsafe fn push_link(&mut self, index: usize, power: usize) {
        unsafe {
            let addr = NonNull::new_unchecked(self.frame_addr(index) as *mut Link);
            addr.write(Link::new(None, self.freelist[power]));

            if let Some(head) = self.freelist[power] {
                (*head.as_ptr()).prev = Some(addr);
            }

            self.freelist[power] = Some(addr);
            self.pagemeta[index] = Some(power as u8);
        }
    }

    unsafe fn remove_link(&mut self, index: usize, power: usize) {
        unsafe {
            let addr = self.frame_addr(index) as *mut Link;

            let prev = (*addr).prev;
            let next = (*addr).next;

            if let Some(p) = prev {
                (*p.as_ptr()).next = next;
            } else {
                let head = self.freelist[power].map(|h| h.as_ptr() as usize);
                if head != Some(addr as usize) {
                    let n = STALE_RM.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                    if n <= 8 {
                        crate::putln!(
                            "[stale-rm] remove_link idx={index} power={power} 自称桶头但桶头={:?} next={:?}（第 {n} 次）",
                            head.map(|h| (h - self.base) / PAGE_SIZE),
                            next.map(|x| (x.as_ptr() as usize - self.base) / PAGE_SIZE)
                        );
                    }
                }
                self.freelist[power] = next;
            }
            if let Some(n) = next {
                (*n.as_ptr()).prev = prev;
            }
        }
    }

    pub(super) unsafe fn split_block(&mut self, power: usize) -> Option<usize> {
        unsafe {
            let mut k = power;
            while k < self.freelist.len() && self.freelist[k].is_none() {
                k += 1;
            }
            if k >= self.freelist.len() {
                return None;
            }

            let index = self.pull_link(k)?;

            while k > power {
                k -= 1;
                let buddy = Self::buddy_index(index, k);
                self.push_link(buddy, k);
            }

            Some(index)
        }
    }

    pub(super) unsafe fn merge_block(&mut self, mut index: usize, mut power: usize) {
        unsafe {
            self.clear_head(index);
            while power < self.freelist.len() {
                let buddy = Self::buddy_index(index, power);

                if buddy >= self.pagemeta.len() {
                    break;
                }

                if !self.pagemeta[buddy].is_some_and(|p| p as usize == power) {
                    break;
                }
                if !self.in_freelist(buddy, power) {
                    break;
                }

                self.remove_link(buddy, power);
                self.clear_head(buddy);
                index = index.min(buddy);
                power += 1;
            }

            self.push_link(index, power);
            if !self.in_freelist(index, power) {
                static LOST: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                let n = LOST.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
                if n <= 8 {
                    crate::putln!(
                        "merge: pushed block NOT in chain idx={index} power={power} ({n})"
                    );
                }
            }
        }
    }

    fn in_freelist(&self, index: usize, power: usize) -> bool {
        let target = self.frame_addr(index);
        let mut cur = self.freelist[power];
        while let Some(node) = cur {
            if node.as_ptr() as usize == target {
                return true;
            }
            cur = unsafe { node.read() }.next;
        }
        false
    }
}

fn hole_containing(holes: &[Option<(usize, usize)>], index: usize) -> Option<usize> {
    holes
        .iter()
        .flatten()
        .find(|&&(start, end)| index >= start && index < end)
        .map(|&(_, end)| end)
}
