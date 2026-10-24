//! 取得一次私有 IOVA 映射的请求：已认证的实际 caller 在门外给出其余坐标。

use env::{PieToken, TaskId};
use hub_api::BindingId;

use super::domain_id::DomainId;

pub struct AcquireRequest {
    pub domain: DomainId,
    pub binding: BindingId,
    pub execution: TaskId,
    pub source: PieToken,
    pub access: u32,
}
