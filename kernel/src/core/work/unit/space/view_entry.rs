use alloc::sync::Arc;

use crate::core::memory::nest::View;
use crate::core::work::pend::SourceAuthorization;

use super::{ViewBinding, ViewControlRecord};

#[derive(Clone)]
pub(crate) struct ViewEntry {
    /// 与来源名下那条成员**同一个** `Arc`：控制身份与结清都只有这一处。
    pub(crate) binding: Arc<ViewBinding>,
    /// 资源映射没有 Memory 控制记录，不能凭关系号取得额外操作权。
    pub(crate) control: Option<Arc<ViewControlRecord>>,
    pub(crate) view: Option<Arc<View>>,
    pub(crate) source: Arc<SourceAuthorization>,
}

impl ViewEntry {
    pub(crate) fn view(&self) -> Option<&Arc<View>> {
        self.view.as_ref()
    }
}
