use alloc::sync::Arc;

/// 帧分段只看帧的物理地址，故宿主帧只钉住这个值，不映射真实内存。
pub struct Frame {
    physical: usize,
}

impl Frame {
    pub fn at(physical: usize) -> Arc<Self> {
        Arc::new(Self { physical })
    }

    pub fn physical_address(&self) -> usize {
        self.physical
    }
}
