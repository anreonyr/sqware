use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

#[cfg(debug_assertions)]
use crate::core::memory::PAGE_SIZE;
#[cfg(debug_assertions)]
use crate::core::memory::manager::entry::PteFlags;
#[cfg(debug_assertions)]
use crate::core::memory::manager::mode;
#[cfg(debug_assertions)]
use crate::core::memory::manager::table::TableNode;

#[cfg(debug_assertions)]
use super::Tree;
use super::map::Map;

mod node;

pub(super) use node::IndexNode;

/// 区间索引：AVL 根与 Map 计数；键是 Map 最小区间起点，节点缓存子树高度与最大
/// 区间终点。
pub(super) struct Index {
    root: Option<Box<IndexNode>>,
    len: usize,
}

impl Index {
    pub(super) const fn new() -> Self {
        Self { root: None, len: 0 }
    }

    /// 存量 Map 数。
    pub(super) fn len(&self) -> usize {
        self.len
    }

    /// 接入一个已建好的节点。
    pub(super) fn insert(&mut self, node: Box<IndexNode>) {
        self.root = Some(insert(self.root.take(), node));
        self.len += 1;
    }

    /// 按 Map 身份摘除节点；摘到才减计数。
    pub(super) fn remove(&mut self, map: &Arc<Map>) -> bool {
        let (root, removed) = remove_map(self.root.take(), map);
        self.root = root;
        if removed {
            self.len -= 1;
        }
        removed
    }

    /// 覆盖 `address` 的 Map。
    pub(super) fn find(&self, address: usize) -> Option<Arc<Map>> {
        find_map(self.root.as_deref(), address)
    }

    /// 与 [lo, hi] 相交的第一个 Map。
    pub(super) fn first_overlap(&self, lo: usize, hi: usize) -> Option<Arc<Map>> {
        first_overlap(self.root.as_deref(), lo, hi)
    }

    /// 中序收集全部 Map。
    pub(super) fn collect(&self, maps: &mut Vec<Arc<Map>>) {
        collect_maps(self.root.as_deref(), maps)
    }

    /// Map 区间变化后重算其节点缓存。
    pub(super) fn refresh(&mut self, map: &Arc<Map>) -> bool {
        refresh(&mut self.root, map)
    }

    /// 撕销全部索引。
    pub(super) fn clear(&mut self) {
        self.root = None;
        self.len = 0;
    }

    /// 索引自身的不变量：节点数等于计数，键、高度与缓存都要对。
    #[cfg(debug_assertions)]
    pub(super) fn audit(&self) {
        let (_, _, count) = audit_index(self.root.as_deref(), None, None);
        assert_eq!(count, self.len, "tree index length");
    }

    /// 索引内的 Map 与树、硬件页表一致。
    #[cfg(debug_assertions)]
    pub(super) fn audit_maps(&self, tree: &Tree, root: &TableNode, geo: mode::Geo) {
        audit_maps(self.root.as_deref(), tree, root, geo);
    }
}

/// 按最小区间起点接入节点。
fn insert(root: Option<Box<IndexNode>>, node: Box<IndexNode>) -> Box<IndexNode> {
    let Some(mut root) = root else { return node };
    if node.start < root.start {
        root.left = Some(insert(root.left.take(), node));
    } else {
        root.right = Some(insert(root.right.take(), node));
    }
    IndexNode::balance(root)
}

/// 按 Map 身份摘除节点，返回 (新根, 是否摘到)。
fn remove_map(root: Option<Box<IndexNode>>, map: &Arc<Map>) -> (Option<Box<IndexNode>>, bool) {
    let Some(mut node) = root else {
        return (None, false);
    };
    if Arc::ptr_eq(&node.map, map) {
        return (IndexNode::detach(node), true);
    }
    let (left, found) = remove_map(node.left.take(), map);
    node.left = left;
    if found {
        return (Some(IndexNode::balance(node)), true);
    }
    let (right, found) = remove_map(node.right.take(), map);
    node.right = right;
    if found {
        (Some(IndexNode::balance(node)), true)
    } else {
        (Some(node), false)
    }
}

/// 覆盖 `address` 的 Map（先左子树，再看本节点，再右子树）。
fn find_map(root: Option<&IndexNode>, address: usize) -> Option<Arc<Map>> {
    let node = root?;
    if node
        .left
        .as_ref()
        .is_some_and(|left| left.max_end >= address)
    {
        if let Some(map) = find_map(node.left.as_deref(), address) {
            return Some(map);
        }
    }
    if node.map.overlaps(address, address) {
        return Some(node.map.clone());
    }
    (node.start.as_usize() <= address)
        .then(|| find_map(node.right.as_deref(), address))
        .flatten()
}

/// 与 [lo, hi] 相交的第一个 Map。
fn first_overlap(root: Option<&IndexNode>, lo: usize, hi: usize) -> Option<Arc<Map>> {
    let node = root?;
    if node.max_end < lo {
        return None;
    }
    if node.left.as_ref().is_some_and(|left| left.max_end >= lo) {
        if let Some(map) = first_overlap(node.left.as_deref(), lo, hi) {
            return Some(map);
        }
    }
    if node.map.overlaps(lo, hi) {
        return Some(node.map.clone());
    }
    if node.start.as_usize() <= hi {
        first_overlap(node.right.as_deref(), lo, hi)
    } else {
        None
    }
}

fn collect_maps(node: Option<&IndexNode>, maps: &mut Vec<Arc<Map>>) {
    let Some(node) = node else { return };
    collect_maps(node.left.as_deref(), maps);
    maps.push(node.map.clone());
    collect_maps(node.right.as_deref(), maps);
}

fn refresh(root: &mut Option<Box<IndexNode>>, map: &Arc<Map>) -> bool {
    let Some(node) = root.as_deref_mut() else {
        return false;
    };
    if Arc::ptr_eq(&node.map, map) {
        node.update();
        return true;
    }
    if refresh(&mut node.left, map) || refresh(&mut node.right, map) {
        node.update();
        true
    } else {
        false
    }
}

#[cfg(debug_assertions)]
fn audit_index(
    node: Option<&IndexNode>,
    lower: Option<usize>,
    upper: Option<usize>,
) -> (usize, usize, usize) {
    let Some(node) = node else { return (0, 0, 0) };
    let key = node.start.as_usize();
    assert!(lower.is_none_or(|lo| lo < key), "AVL key below lower bound");
    assert!(upper.is_none_or(|hi| key < hi), "AVL key above upper bound");
    let (left_height, left_end, left_count) = audit_index(node.left.as_deref(), lower, Some(key));
    let (right_height, right_end, right_count) =
        audit_index(node.right.as_deref(), Some(key), upper);
    let map_state = node.map.state.lock();
    assert!(
        !map_state.ranges.is_empty(),
        "indexed map has no active ranges"
    );
    assert!(
        map_state
            .ranges
            .windows(2)
            .all(|pair| pair[0].end() < pair[1].start.as_usize()),
        "map ranges overlap or are unordered"
    );
    for range in &map_state.ranges {
        assert!(
            !overlaps_other(node, &node.map, range.start.as_usize(), range.end()),
            "distinct Maps overlap in the Tree index"
        );
    }
    assert_eq!(
        key,
        map_state
            .ranges
            .iter()
            .map(|r| r.start.as_usize())
            .min()
            .unwrap(),
        "AVL key differs from active Map range"
    );
    let local_end = map_state.ranges.iter().map(|r| r.end()).max().unwrap();
    let expected_height = 1 + left_height.max(right_height);
    assert_eq!(node.height as usize, expected_height, "AVL height cache");
    assert!(left_height.abs_diff(right_height) <= 1, "AVL balance");
    let expected_end = local_end.max(left_end).max(right_end);
    assert_eq!(node.max_end, expected_end, "AVL max_end cache");
    (expected_height, expected_end, left_count + right_count + 1)
}

#[cfg(debug_assertions)]
fn overlaps_other(node: &IndexNode, map: &Arc<Map>, lo: usize, hi: usize) -> bool {
    if node.max_end < lo {
        return false;
    }
    if node
        .left
        .as_ref()
        .is_some_and(|left| left.max_end >= lo && overlaps_other(left, map, lo, hi))
    {
        return true;
    }
    if !Arc::ptr_eq(&node.map, map) && node.map.overlaps(lo, hi) {
        return true;
    }
    node.start.as_usize() <= hi
        && node
            .right
            .as_ref()
            .is_some_and(|right| overlaps_other(right, map, lo, hi))
}

#[cfg(debug_assertions)]
fn audit_maps(node: Option<&IndexNode>, tree: &Tree, root: &TableNode, geo: mode::Geo) {
    let Some(node) = node else { return };
    audit_maps(node.left.as_deref(), tree, root, geo);
    let map = &node.map;
    assert!(
        core::ptr::eq(map.tree.as_ptr(), tree as *const Tree),
        "indexed Map belongs to another Tree"
    );
    let map_state = map.state.lock();
    assert!(
        map_state.borrowed.is_none() || map_state.frames.is_empty(),
        "Map is both borrowed and Frame-backed"
    );
    assert!(
        map_state.borrowed.is_none() || !map_state.lazy,
        "borrowed Map is marked lazy"
    );
    assert!(
        map_state
            .frames
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0),
        "owned Frame indexes are not strictly ordered"
    );
    for (page, frame) in &map_state.frames {
        let offset = page
            .checked_mul(PAGE_SIZE)
            .expect("Frame page offset overflow");
        assert!(
            offset < map.initial_size.get(),
            "owned Frame exceeds initial Map extent"
        );
        let address = map.initial_start + offset;
        assert!(
            map_state.covers(address.as_usize(), address.as_usize()),
            "owned Frame lies outside live Map ranges"
        );
        let (physical, flags) = root
            .walk_ref_geometry(address, geo)
            .expect("owned Frame has no hardware PTE");
        assert_eq!(
            physical.as_usize(),
            frame.physical_address(),
            "owned Frame PTE physical address"
        );
        let expected = map_state
            .range_at(address.as_usize())
            .expect("Frame range exists")
            .current;
        assert_eq!(
            (flags
                & (PteFlags::V
                    | PteFlags::R
                    | PteFlags::W
                    | PteFlags::X
                    | PteFlags::U
                    | PteFlags::G))
                .bits(),
            (expected
                & (PteFlags::V
                    | PteFlags::R
                    | PteFlags::W
                    | PteFlags::X
                    | PteFlags::U
                    | PteFlags::G))
                .bits(),
            "Frame PTE permissions differ from Map state"
        );
    }
    if let Some(region) = &map_state.borrowed {
        assert!(
            map_state
                .borrowed_offset
                .checked_add(map.initial_size.get())
                .is_some_and(|end| end <= region.size.get()),
            "borrowed Map exceeds its reserved physical region"
        );
    }
    if !map_state.lazy || map_state.borrowed.is_some() {
        for range in &map_state.ranges {
            let mut offset = 0;
            while offset < range.size.get() {
                let address = range.start + offset;
                let span = root.leaf_span_geometry(address, geo);
                let step = match span {
                    Some(span) => {
                        assert!(
                            span <= range.size.get() - offset,
                            "hardware leaf crosses a live range boundary"
                        );
                        span
                    }
                    None if map_state.lazy => PAGE_SIZE,
                    None => panic!("eager live Map range has an unmapped page"),
                };
                offset += step;
            }
        }
    }
    drop(map_state);
    audit_maps(node.right.as_deref(), tree, root, geo);
}
