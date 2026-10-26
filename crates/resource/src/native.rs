//! 真实 Env 适配：单次调用、原错误域与原有映射转换，不含重试循环。

use env::{FlatReference, GrantRange, Mark, Permission, PieToken, TaskId};

use crate::{kernel::CapabilityKernel, mapping_kernel::MappingKernel};

/// 默认内核后端：每个入口直接转发真实 Env。
#[doc(hidden)]
pub struct Native;

impl CapabilityKernel for Native {
    type PieError = erra::Error<env::PieFail>;
    type HoleError = erra::Error<env::HoleFail>;
    type PoleError = erra::Error<env::PoleFail>;

    fn unseal_hole(mark: Mark) -> Result<PieToken, Self::HoleError> {
        env::hole::unseal(mark, env::HoleLimits::default())
    }

    fn seal_hole(token: PieToken) -> Result<(), Self::HoleError> {
        env::hole::seal(token)
    }

    fn unseal_pole(reference: FlatReference, shared: bool) -> Result<PieToken, Self::PoleError> {
        env::pole::unseal(reference, shared)
    }

    fn derive(
        token: PieToken,
        permission: Permission,
        mark: Mark,
        range: GrantRange,
    ) -> Result<PieToken, Self::PieError> {
        env::pie::derive(token, permission, mark, range)
    }

    fn accord(
        token: PieToken,
        task: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> Result<PieToken, Self::PieError> {
        env::pie::accord(token, task, permission, mark)
    }

    fn release(token: PieToken) -> Result<(), Self::PieError> {
        env::pie::release(token, env::ReleaseMode::Revoke)
    }

    fn revoke(task: TaskId, token: PieToken) -> Result<(), Self::PieError> {
        env::pie::revoke(task, token)
    }

    fn is_busy(error: &Self::PieError) -> bool {
        error.source == env::PieFail::Busy
    }

    fn starve() {
        env::room::starve();
    }
}

impl MappingKernel for Native {
    type Error = erra::Error<env::PoleFail>;

    fn open(token: PieToken) -> Result<(usize, usize), Self::Error> {
        env::pole::open(token).map(|(address, size)| (address.get(), size))
    }

    fn shut(token: PieToken) -> Result<(), Self::Error> {
        env::pole::shut(token)
    }
}
