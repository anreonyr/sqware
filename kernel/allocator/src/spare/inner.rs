use core::ptr::NonNull;

use super::HEADER;
use super::blk::Blk;

pub(super) struct SpareInner {
    pub(super) base: usize,
    pub(super) edge: usize,
    head: Option<NonNull<Blk>>,
}

// SAFETY: 裸指针只在持有 Spare 锁时读写。
unsafe impl Send for SpareInner {}

impl SpareInner {
    pub(super) fn new(base: usize, edge: usize) -> Self {
        let head = unsafe { Blk::write(base, edge - base) };
        Self {
            base,
            edge,
            head: Some(head),
        }
    }

    pub(super) fn owns(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.edge
    }

    fn unlink(&mut self, blk: NonNull<Blk>) {
        unsafe {
            let l = blk.read().link;
            match l.prev {
                Some(p) => (*p.as_ptr()).next = l.next,
                None => self.head = l.next.map(|n| n.cast()),
            }
            if let Some(n) = l.next {
                (*n.as_ptr()).prev = l.prev;
            }
        }
    }

    pub(super) fn pull(&mut self, need: usize) -> Option<NonNull<Blk>> {
        let mut cur = self.head;
        while let Some(blk) = cur {
            if unsafe { blk.read() }.payload() >= need {
                self.split(blk, need);
                return Some(blk);
            }
            cur = unsafe { blk.read() }.next();
        }
        None
    }

    fn split(&mut self, blk: NonNull<Blk>, need: usize) {
        let start = blk.as_ptr() as usize;
        let size = unsafe { blk.read() }.size;
        let left = size - HEADER - need;
        if left >= HEADER {
            let rest = unsafe { Blk::write(start + HEADER + need, left) };
            unsafe {
                let l = blk.read().link;
                (*rest.as_ptr()).link = l;
                let (p, n) = ((*rest.as_ptr()).link.prev, (*rest.as_ptr()).link.next);
                match p {
                    Some(p) => (*p.as_ptr()).next = Some(rest.cast()),
                    None => self.head = Some(rest),
                }
                if let Some(n) = n {
                    (*n.as_ptr()).prev = Some(rest.cast());
                }
                (*blk.as_ptr()).size = HEADER + need;
            }
        } else {
            self.unlink(blk);
        }
    }

    pub(super) fn push(&mut self, mut blk: NonNull<Blk>) {
        let addr = blk.as_ptr() as usize;
        let mut prev: Option<NonNull<Blk>> = None;
        let mut cur = self.head;
        while let Some(n) = cur {
            if n.as_ptr() as usize > addr {
                break;
            }
            prev = Some(n);
            cur = unsafe { n.read() }.next();
        }
        unsafe {
            (*blk.as_ptr()).link.prev = prev.map(|p| p.cast());
            (*blk.as_ptr()).link.next = cur.map(|n| n.cast());
            match prev {
                Some(p) => (*p.as_ptr()).link.next = Some(blk.cast()),
                None => self.head = Some(blk),
            }
            if let Some(n) = cur {
                (*n.as_ptr()).link.prev = Some(blk.cast());
            }
        }
        if let Some(p) = prev {
            let pend = (p.as_ptr() as usize) + unsafe { p.read() }.size;
            if pend == addr {
                unsafe {
                    (*p.as_ptr()).size += blk.read().size;
                }
                self.unlink(blk);
                blk = p;
            }
        }
        if let Some(n) = unsafe { blk.read() }.next() {
            let bend = (blk.as_ptr() as usize) + unsafe { blk.read() }.size;
            if bend == n.as_ptr() as usize {
                unsafe {
                    (*blk.as_ptr()).size += n.read().size;
                }
                self.unlink(n);
            }
        }
    }
}
