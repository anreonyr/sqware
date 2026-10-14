use alloc::sync::Arc;

use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::View;

/// 一笔已落地的映射：线外兼容名，只持关系身份。
///
/// 地址与长度从关系自身读出，不再另存一份占用；线外的借出句柄都拿它，
/// 核心只认 `Arc<View>`。
pub struct Mapping(Arc<View>);

impl Mapping {
    pub(crate) fn new(view: Arc<View>) -> Self {
        Self(view)
    }

    pub fn view(&self) -> &Arc<View> {
        &self.0
    }

    /// 拆开兼容壳：核心与消费式入口只收关系身份。
    pub(crate) fn into_view(self) -> Arc<View> {
        self.0
    }

    pub fn address(&self) -> VirtAddr {
        self.0.initial_start()
    }

    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        self.0.initial_size()
    }
}

unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}
