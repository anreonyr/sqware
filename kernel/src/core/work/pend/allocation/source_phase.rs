//! 一份来源授权当前归谁管：私有根，还是已经交给根 Grant。
//!
//! 两个集合分得很清楚：私有阶段已建的关系记在自己这边；交给 Pole 之后
//! 关系由那个 token 下的登记管，撤销随之关闭。

use alloc::sync::{Arc, Weak};

use crate::core::work::grant::Grant;
use crate::core::work::mail::pole::PoleMeta;
use env::PieToken;

pub(super) enum Phase {
    /// 还没有 Pole 接手。
    Private,
    /// 已由根 Grant 接手：撤销依赖那枚 Grant，后续关系交给 Pole 管。
    Root {
        meta: Weak<PoleMeta>,
        token: PieToken,
        grant: Arc<Grant>,
    },
}
