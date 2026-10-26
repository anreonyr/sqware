//! 本测试自己的 capability-only 适配：只转发原 fake Env，不实现 MappingKernel。

use crate::{
    FlatReference, GrantRange, HoleLimits, Mark, Permission, PieToken, ReleaseMode, TaskId,
    kernel::CapabilityKernel,
};

pub struct Native;

impl CapabilityKernel for Native {
    type PieError = erra::Error<crate::PieFail>;
    type HoleError = erra::Error<crate::HoleFail>;
    type PoleError = erra::Error<crate::PoleFail>;

    fn unseal_hole(mark: Mark) -> Result<PieToken, Self::HoleError> {
        crate::hole::unseal(mark, HoleLimits::default())
    }

    fn seal_hole(token: PieToken) -> Result<(), Self::HoleError> {
        crate::hole::seal(token)
    }

    fn unseal_pole(reference: FlatReference, shared: bool) -> Result<PieToken, Self::PoleError> {
        crate::pole::unseal(reference, shared)
    }

    fn derive(
        token: PieToken,
        permission: Permission,
        mark: Mark,
        range: GrantRange,
    ) -> Result<PieToken, Self::PieError> {
        crate::pie::derive(token, permission, mark, range)
    }

    fn accord(
        token: PieToken,
        task: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> Result<PieToken, Self::PieError> {
        crate::pie::accord(token, task, permission, mark)
    }

    fn release(token: PieToken) -> Result<(), Self::PieError> {
        crate::pie::release(token, ReleaseMode::Revoke)
    }

    fn revoke(task: TaskId, token: PieToken) -> Result<(), Self::PieError> {
        crate::pie::revoke(task, token)
    }

    fn is_busy(error: &Self::PieError) -> bool {
        error.source == crate::PieFail::Busy
    }

    fn starve() {
        crate::test_backend::record(crate::test_backend::Event::Starve);
    }
}
