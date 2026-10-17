use super::{DomainRange, Failure};

/// 停机权威：只由受信组成装上。
pub trait Platform {
    fn domain_range(&self) -> DomainRange;

    /// 撤翻译、失效设备缓存并排空外部读写者；`Pending` 保留全部引用。
    fn synchronize(&mut self) -> Result<(), Failure>;

    /// 独立停住整个硬件作用域并确认外部访问结束。
    fn isolate(&mut self) -> Result<(), Failure>;
}
