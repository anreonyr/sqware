use alloc::boxed::Box;
use alloc::sync::Arc;

use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::NestError;

use super::super::view::{ActiveRange, View};

/// 关系索引节点：以关系最小区间起点为键的 AVL 节点，缓存高度与最大终点。
pub(in crate::core::memory::nest) struct IndexNode {
    pub(super) view: Arc<View>,
    pub(super) left: Option<Box<IndexNode>>,
    pub(super) right: Option<Box<IndexNode>>,
    pub(super) start: VirtAddr,
    pub(super) height: u8,
    pub(super) max_end: usize,
}

impl IndexNode {
    /// 以关系当前区间建节点。
    pub(in crate::core::memory::nest) fn new(view: Arc<View>) -> Result<Self, NestError> {
        let (start, max_end) = Self::span(&view.state.lock().active)?;
        Ok(Self::cached(view, start, max_end))
    }

    /// 以给定区间建节点。
    pub(in crate::core::memory::nest) fn with_ranges(
        view: Arc<View>,
        active: &[ActiveRange],
    ) -> Result<Self, NestError> {
        let (start, max_end) = Self::span(active)?;
        Ok(Self::cached(view, start, max_end))
    }

    /// 键取最小区间起点，缓存终点取最大区间终点；空集合没有键。
    fn span(active: &[ActiveRange]) -> Result<(VirtAddr, usize), NestError> {
        let start = active
            .iter()
            .map(|range| range.start)
            .min()
            .ok_or(NestError::NoRegion)?;
        let max_end = active
            .iter()
            .map(|range| range.end())
            .max()
            .ok_or(NestError::NoRegion)?;
        Ok((start, max_end))
    }

    fn cached(view: Arc<View>, start: VirtAddr, max_end: usize) -> Self {
        Self {
            view,
            left: None,
            right: None,
            start,
            height: 1,
            max_end,
        }
    }

    /// 空子树高度为 0。
    pub(super) fn height(node: &Option<Box<Self>>) -> u8 {
        node.as_ref().map_or(0, |node| node.height)
    }

    /// 重算高度与最大终点：自身活动区间与左右子树。
    pub(super) fn update(&mut self) {
        self.height = 1 + Self::height(&self.left).max(Self::height(&self.right));
        {
            let state = self.view.state.lock();
            if let Some(start) = state.active.iter().map(|range| range.start).min() {
                self.start = start;
            }
            self.max_end = state
                .active
                .iter()
                .map(|range| range.end())
                .max()
                .unwrap_or(0);
        }
        if let Some(left) = self.left.as_ref() {
            self.max_end = self.max_end.max(left.max_end);
        }
        if let Some(right) = self.right.as_ref() {
            self.max_end = self.max_end.max(right.max_end);
        }
    }

    fn rotate_left(mut node: Box<Self>) -> Box<Self> {
        let mut right = node.right.take().expect("AVL right child");
        node.right = right.left.take();
        node.update();
        right.left = Some(node);
        right.update();
        right
    }

    fn rotate_right(mut node: Box<Self>) -> Box<Self> {
        let mut left = node.left.take().expect("AVL left child");
        node.left = left.right.take();
        node.update();
        left.right = Some(node);
        left.update();
        left
    }

    /// 恢复 AVL 平衡并刷新缓存。
    pub(super) fn balance(mut node: Box<Self>) -> Box<Self> {
        node.update();
        if Self::height(&node.left) > Self::height(&node.right) + 1 {
            if Self::height(&node.left.as_ref().unwrap().right)
                > Self::height(&node.left.as_ref().unwrap().left)
            {
                node.left = Some(Self::rotate_left(node.left.take().unwrap()));
            }
            Self::rotate_right(node)
        } else if Self::height(&node.right) > Self::height(&node.left) + 1 {
            if Self::height(&node.right.as_ref().unwrap().left)
                > Self::height(&node.right.as_ref().unwrap().right)
            {
                node.right = Some(Self::rotate_right(node.right.take().unwrap()));
            }
            Self::rotate_left(node)
        } else {
            node
        }
    }

    /// 摘下最小键节点，返回 (剩余子树, 该节点)。
    pub(super) fn take_min(mut node: Box<Self>) -> (Option<Box<Self>>, Box<Self>) {
        let Some(left) = node.left.take() else {
            let right = node.right.take();
            return (right, node);
        };
        let (left, min) = Self::take_min(left);
        node.left = left;
        (Some(Self::balance(node)), min)
    }

    /// 摘除本节点，空位由右子树最小节点顶替。
    pub(super) fn detach(mut node: Box<Self>) -> Option<Box<Self>> {
        let left = node.left.take();
        let right = node.right.take();
        match (left, right) {
            (None, right) => right,
            (left, None) => left,
            (left, Some(right)) => {
                let (right, mut next) = Self::take_min(right);
                next.left = left;
                next.right = right;
                Some(Self::balance(next))
            }
        }
    }
}
