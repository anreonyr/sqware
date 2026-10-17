//! **只给目标景**用的一次注入：把一枚真平台的**第一次**停住答成 `Pending`。
//!
//! 这一层只用来验“平台没确认时，保留逐个仍在、业务不重启；后来真确认了才释放”
//! 那一条。它**不**改生产平台，也**不**替真硬件超时作证：注入是装配的事，
//! 真实 `Pending` 的证据归真实平台。

use crate::system::supply::retention::{DeviceRange, Failure, Platform};

/// 包着一枚真平台：第一次叫它停就答 `Pending`，第二次起原样转发。
pub struct FaultOnce<P> {
    inner: P,
    injected: bool,
}

impl<P> FaultOnce<P> {
    pub const fn new(inner: P) -> Self {
        Self {
            inner,
            injected: false,
        }
    }
}

impl<P: Platform> Platform for FaultOnce<P> {
    fn range(&self) -> DeviceRange {
        self.inner.range()
    }

    fn synchronize(&mut self) -> Result<(), Failure> {
        if !self.injected {
            self.injected = true;
            return Err(Failure::Pending);
        }
        self.inner.synchronize()
    }

    fn isolate(&mut self) -> Result<(), Failure> {
        if !self.injected {
            self.injected = true;
            return Err(Failure::Pending);
        }
        self.inner.isolate()
    }
}
