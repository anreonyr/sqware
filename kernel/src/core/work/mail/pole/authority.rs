use alloc::sync::Arc;

use env::{Permission, PieToken};

use crate::core::work::grant::Grant;

/// 一份授权记录：token 与它当前的权限位。
///
/// 记录里不放来源，也不放实际 `View`：来源由 `PoleMeta` 唯一持有，
/// 该 token 下的活跃关系另存，撤销边由关系自己的 token 承担。
pub(super) struct Authority {
    token: PieToken,
    grant: Arc<Grant>,
}

impl Authority {
    pub(super) fn new(token: PieToken, grant: Arc<Grant>) -> Self {
        Self { token, grant }
    }

    pub(super) fn authorizes(&self, token: PieToken) -> bool {
        self.token == token
    }

    /// 授权是否仍然有效（失效后 permission 为空）。
    pub(super) fn permitted(&self) -> bool {
        self.grant.permission().is_some()
    }

    pub(super) fn stores(&self) -> bool {
        self.grant
            .permission()
            .is_some_and(|permission| permission.contains(Permission::STORE))
    }

    pub(super) fn grant(&self) -> Arc<Grant> {
        self.grant.clone()
    }
}
