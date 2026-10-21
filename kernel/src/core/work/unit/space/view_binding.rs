//! 目标空间与来源记录共享的 CPU 关系身份；不持帧，也不授予视图控制权。

use alloc::sync::{Arc, Weak};

use env::PieFail;

use super::Space;

pub(crate) struct ViewBinding {
    id: usize,
    space: Weak<Space>,
}

impl ViewBinding {
    pub(crate) fn new(id: usize, space: &Arc<Space>) -> Result<Arc<Self>, PieFail> {
        Arc::try_new(Self {
            id,
            space: Arc::downgrade(space),
        })
        .map_err(|_| PieFail::OoM)
    }

    pub(crate) fn id(&self) -> usize {
        self.id
    }

    pub(crate) fn space(&self) -> &Weak<Space> {
        &self.space
    }
}
