use alloc::sync::Arc;

use env::PieToken;

use crate::core::memory::manager::Map;
use crate::core::work::mail::pole::PoleMeta;

/// 一条已从 Pole 派生、等着随发布交给新任务的映射。
#[derive(Clone)]
pub(crate) struct Staging {
    pub(crate) token: PieToken,
    pub(crate) meta: Arc<PoleMeta>,
    pub(crate) map: Arc<Map>,
}
