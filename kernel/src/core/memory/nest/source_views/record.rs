//! 关联里的一条：这段来源窗口被这个关系压着。
//!
//! 一条一次分配、随关系退场就释放：整张关联没有全局容量高水位。

use alloc::sync::Weak;

use super::super::View;

pub(super) struct Record {
    pub(super) source_id: usize,
    pub(super) view: Weak<View>,
    pub(super) next: Option<alloc::boxed::Box<Record>>,
}
