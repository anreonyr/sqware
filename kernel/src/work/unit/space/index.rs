use alloc::boxed::Box;
use core::mem::MaybeUninit;

use super::map::Map;
use crate::memory::manager::addr::VirtAddr;

#[derive(Debug)]
pub(super) struct Index {
    root: Option<Box<Map>>,
    len: usize,
}

impl Index {
    pub(super) const fn new() -> Self {
        Self { root: None, len: 0 }
    }

    pub(super) fn len(&self) -> usize { self.len }

    pub(super) fn insert(&mut self, mut map: Box<Map>) {
        assert!(map.left.is_none() && map.right.is_none());
        debug_assert!(self.first_overlap(map.va.as_usize(), map.end()).is_none());
        update(&mut map);
        self.root = Some(insert(self.root.take(), map));
        self.len += 1;
    }

    pub(super) fn remove(&mut self, va: VirtAddr) -> Option<Box<Map>> {
        let (root, removed) = remove(self.root.take(), va);
        self.root = root;
        if removed.is_some() { self.len -= 1; }
        removed
    }

    pub(super) fn get(&self, va: VirtAddr) -> Option<&Map> {
        let mut node = self.root.as_deref();
        while let Some(map) = node {
            if va < map.va { node = map.left.as_deref(); }
            else if map.contains(va) { return Some(map); }
            else { node = map.right.as_deref(); }
        }
        None
    }

    pub(super) fn get_mut(&mut self, va: VirtAddr) -> Option<&mut Map> {
        let mut node = self.root.as_deref_mut();
        while let Some(map) = node {
            if va < map.va { node = map.left.as_deref_mut(); }
            else if map.contains(va) { return Some(map); }
            else { node = map.right.as_deref_mut(); }
        }
        None
    }

    pub(super) fn first_overlap(&self, lo: usize, hi: usize) -> Option<&Map> {
        if lo > hi { return None; }
        let mut node = self.root.as_deref();
        while let Some(map) = node {
            if map.max_end < lo { return None; }
            if map.left.as_ref().is_some_and(|left| left.max_end >= lo) {
                node = map.left.as_deref();
            } else if map.va.as_usize() > hi {
                return None;
            } else if map.end() >= lo {
                return Some(map);
            } else {
                node = map.right.as_deref();
            }
        }
        None
    }

    pub(super) fn iter(&self) -> Iter<'_> { self.overlapping(0, usize::MAX) }

    // Inclusive endpoints keep the final virtual page representable.
    pub(super) fn overlapping(&self, lo: usize, hi: usize) -> Iter<'_> {
        let mut iter = Iter { stack: [MaybeUninit::uninit(); DEPTH], depth: 0, lo, hi };
        if lo <= hi { iter.descend(self.root.as_deref()); }
        iter
    }

    pub(super) fn visit_mut(&mut self, lo: usize, hi: usize, mut visit: impl FnMut(&mut Map)) {
        fn walk(node: Option<&mut Map>, lo: usize, hi: usize, visit: &mut impl FnMut(&mut Map)) {
            let Some(map) = node else { return };
            if map.max_end < lo { return; }
            walk(map.left.as_deref_mut(), lo, hi, visit);
            if map.va.as_usize() > hi { return; }
            if map.end() >= lo { visit(map); }
            walk(map.right.as_deref_mut(), lo, hi, visit);
        }
        if lo <= hi { walk(self.root.as_deref_mut(), lo, hi, &mut visit); }
    }

    #[cfg(debug_assertions)]
    pub(super) fn audit(&self) {
        fn check(node: Option<&Map>) -> (u8, usize, usize) {
            let Some(map) = node else { return (0, 0, 0) };
            let (left_height, left_end, left_count) = check(map.left.as_deref());
            let (right_height, right_end, right_count) = check(map.right.as_deref());
            assert!(left_height.abs_diff(right_height) <= 1, "map AVL balance");
            assert_eq!(map.height, 1 + left_height.max(right_height));
            let max_end = map.end().max(left_end).max(right_end);
            assert_eq!(map.max_end, max_end);
            if let Some(left) = map.left.as_deref() {
                assert!(left.max_end < map.va.as_usize(), "map left overlap/order");
            }
            (map.height, max_end, 1 + left_count + right_count)
        }
        let (_, _, count) = check(self.root.as_deref());
        assert_eq!(self.len, count);
        let mut previous: Option<usize> = None;
        for map in self.iter() {
            assert!(previous.is_none_or(|end| end < map.va.as_usize()), "map overlap/order");
            previous = Some(map.end());
        }
    }
}

// An AVL tree with at most usize::MAX nodes has height below twice the word width.
const DEPTH: usize = 2 * usize::BITS as usize;

pub(super) struct Iter<'a> {
    stack: [MaybeUninit<&'a Map>; DEPTH],
    depth: usize,
    lo: usize,
    hi: usize,
}

impl<'a> Iter<'a> {
    fn descend(&mut self, mut node: Option<&'a Map>) {
        while let Some(map) = node {
            if map.max_end < self.lo { break; }
            if map.va.as_usize() <= self.hi {
                assert!(self.depth < DEPTH, "map index depth");
                self.stack[self.depth].write(map);
                self.depth += 1;
            }
            node = map.left.as_deref();
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Map;

    fn next(&mut self) -> Option<Self::Item> {
        while self.depth != 0 {
            self.depth -= 1;
            // SAFETY: descend initializes every entry below depth; popping lowers depth first.
            let map = unsafe { self.stack[self.depth].assume_init_read() };
            self.descend(map.right.as_deref());
            if map.end() >= self.lo { return Some(map); }
        }
        None
    }
}

fn height(node: &Option<Box<Map>>) -> u8 { node.as_ref().map_or(0, |map| map.height) }

fn update(map: &mut Map) {
    map.height = 1 + height(&map.left).max(height(&map.right));
    map.max_end = map.end()
        .max(map.left.as_ref().map_or(0, |child| child.max_end))
        .max(map.right.as_ref().map_or(0, |child| child.max_end));
}

fn rotate_left(mut map: Box<Map>) -> Box<Map> {
    let mut right = map.right.take().expect("AVL right child");
    map.right = right.left.take();
    update(&mut map);
    right.left = Some(map);
    update(&mut right);
    right
}

fn rotate_right(mut map: Box<Map>) -> Box<Map> {
    let mut left = map.left.take().expect("AVL left child");
    map.left = left.right.take();
    update(&mut map);
    left.right = Some(map);
    update(&mut left);
    left
}

fn balance(mut map: Box<Map>) -> Box<Map> {
    update(&mut map);
    if height(&map.left) > height(&map.right) + 1 {
        let left = map.left.as_ref().expect("AVL left child");
        if height(&left.right) > height(&left.left) {
            map.left = Some(rotate_left(map.left.take().expect("AVL left child")));
        }
        rotate_right(map)
    } else if height(&map.right) > height(&map.left) + 1 {
        let right = map.right.as_ref().expect("AVL right child");
        if height(&right.left) > height(&right.right) {
            map.right = Some(rotate_right(map.right.take().expect("AVL right child")));
        }
        rotate_left(map)
    } else { map }
}

fn insert(root: Option<Box<Map>>, map: Box<Map>) -> Box<Map> {
    let Some(mut root) = root else { return map };
    if map.va < root.va { root.left = Some(insert(root.left.take(), map)); }
    else {
        assert!(map.va > root.va, "duplicate map start");
        root.right = Some(insert(root.right.take(), map));
    }
    balance(root)
}

fn take_min(mut map: Box<Map>) -> (Option<Box<Map>>, Box<Map>) {
    let Some(left) = map.left.take() else {
        let right = map.right.take();
        update(&mut map);
        return (right, map);
    };
    let (left, minimum) = take_min(left);
    map.left = left;
    (Some(balance(map)), minimum)
}

fn remove(root: Option<Box<Map>>, va: VirtAddr) -> (Option<Box<Map>>, Option<Box<Map>>) {
    let Some(mut root) = root else { return (None, None) };
    if va < root.va {
        let (left, removed) = remove(root.left.take(), va);
        root.left = left;
        (Some(balance(root)), removed)
    } else if va > root.va {
        let (right, removed) = remove(root.right.take(), va);
        root.right = right;
        (Some(balance(root)), removed)
    } else {
        let left = root.left.take();
        let next = if let Some(right) = root.right.take() {
            let (right, mut next) = take_min(right);
            next.left = left;
            next.right = right;
            Some(balance(next))
        } else { left };
        update(&mut root);
        (next, Some(root))
    }
}


#[cfg(debug_assertions)]
pub(crate) fn accept() {
    use crate::memory::PAGE_SIZE;
    use crate::memory::manager::entry::PteFlags;
    use super::map::Pending;

    assert!(core::mem::size_of::<Map>() <= 128, "map allocation class");
    const COUNT: usize = 128;
    const BASE: usize = 0x4000_0000;
    const STRIDE: usize = 8 * PAGE_SIZE;
    let mut index = Index::new();
    let mut lengths = [0usize; COUNT];
    let mut random = 0x9e37_79b9_7f4a_7c15u64;
    for step in 0..2048 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let slot = if step < COUNT { step } else if step < 2 * COUNT {
            2 * COUNT - step - 1
        } else { random as usize % COUNT };
        let va = VirtAddr::wrap(BASE + slot * STRIDE);
        let insert = lengths[slot] == 0;
        let size = (1 + ((random >> 16) as usize % 4)) * PAGE_SIZE;
        let map = insert.then(|| Box::try_new(Map::new(va, size, PteFlags::R, Some(Pending::Lazy)))
            .expect("index test node"));
        let guard = crate::memory::allocator::NoAllocation::enter();
        if let Some(map) = map {
            index.insert(map);
            lengths[slot] = size;
        } else {
            let map = index.remove(va).expect("index test removal");
            assert!(map.left.is_none() && map.right.is_none());
            lengths[slot] = 0;
        }
        index.audit();
        assert_eq!(index.len(), lengths.iter().filter(|&&size| size != 0).count());
        for offset in [0, PAGE_SIZE - 1, size - 1, size, STRIDE - 1] {
            let at = va.as_usize() + offset;
            let expected = lengths[slot] != 0 && offset < lengths[slot];
            assert_eq!(index.get(VirtAddr::wrap(at)).is_some(), expected);
        }
        let lo = BASE + (random as usize % (COUNT * STRIDE));
        let hi = lo + ((random >> 32) as usize % (32 * STRIDE));
        let expected_first = lengths.iter().enumerate().find_map(|(i, &length)| {
            let start = BASE + i * STRIDE;
            (length != 0 && start <= hi && start + length - 1 >= lo).then_some(start)
        });
        assert_eq!(index.first_overlap(lo, hi).map(|map| map.va.as_usize()), expected_first);
        assert!(index.first_overlap(hi + 1, lo).is_none());
        let mut actual = index.overlapping(lo, hi);
        for (i, &length) in lengths.iter().enumerate() {
            let start = BASE + i * STRIDE;
            if length != 0 && start <= hi && start + length - 1 >= lo {
                assert_eq!(actual.next().expect("index range member").va.as_usize(), start);
            }
        }
        assert!(actual.next().is_none());
        assert!(index.overlapping(hi + 1, lo).next().is_none());
        drop(guard);
    }
    let top = crate::layout::TRAMPOLINE;
    let map = Box::try_new(Map::new(top, PAGE_SIZE, PteFlags::R, Some(Pending::Lazy)))
        .expect("index top page");
    let guard = crate::memory::allocator::NoAllocation::enter();
    index.insert(map);
    assert_eq!(index.first_overlap(usize::MAX, usize::MAX).expect("top first overlap").va, top);
    assert_eq!(index.get(VirtAddr::wrap(usize::MAX)).expect("top byte").va, top);
    assert_eq!(index.overlapping(usize::MAX, usize::MAX).next().expect("top range").va, top);
    index.visit_mut(top.as_usize(), usize::MAX, |map| map.flags |= PteFlags::W);
    assert!(index.get(top).expect("top mutable visit").flags.contains(PteFlags::W));
    index.remove(top).expect("top removal");
    for i in (0..COUNT).rev() {
        let removed = index.remove(VirtAddr::wrap(BASE + i * STRIDE));
        assert_eq!(removed.is_some(), lengths[i] != 0);
        index.audit();
    }
    assert_eq!(index.len(), 0);
    drop(guard);
}
