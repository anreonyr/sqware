use resource_impl::{kernel::CapabilityKernel, mapping_kernel::MappingKernel};

/// 本夹具唯一的能力与映射内核适配：全部落回既有 STATE。
pub struct Kernel;

impl CapabilityKernel for Kernel {
    type PieError = erra::Error<env::PieFail>;
    type HoleError = erra::Error<env::HoleFail>;
    type PoleError = erra::Error<env::PoleFail>;

    fn unseal_hole(mark: env::Mark) -> Result<env::PieToken, Self::HoleError> {
        env::hole::unseal(mark, env::HoleLimits::default())
    }

    fn seal_hole(token: env::PieToken) -> Result<(), Self::HoleError> {
        env::hole::seal(token)
    }

    fn unseal_pole(
        reference: env::FlatReference,
        shared: bool,
    ) -> Result<env::PieToken, Self::PoleError> {
        env::pole::unseal(reference, shared)
    }

    fn derive(
        token: env::PieToken,
        permission: env::Permission,
        mark: env::Mark,
        range: env::GrantRange,
    ) -> Result<env::PieToken, Self::PieError> {
        env::pie::derive(token, permission, mark, range)
    }

    fn accord(
        token: env::PieToken,
        task: env::TaskId,
        permission: env::Permission,
        mark: env::Mark,
    ) -> Result<env::PieToken, Self::PieError> {
        env::pie::accord(token, task, permission, mark)
    }

    fn release(token: env::PieToken) -> Result<(), Self::PieError> {
        env::pie::release(token, env::ReleaseMode::Revoke)
    }

    fn revoke(task: env::TaskId, token: env::PieToken) -> Result<(), Self::PieError> {
        env::pie::revoke(task, token)
    }

    fn is_busy(error: &Self::PieError) -> bool {
        error.source == env::PieFail::Busy
    }

    fn starve() {
        env::room::starve();
    }
}

impl MappingKernel for Kernel {
    type Error = erra::Error<env::PoleFail>;

    fn open(token: env::PieToken) -> Result<(usize, usize), Self::Error> {
        crate::raw::open(token)
    }

    fn shut(token: env::PieToken) -> Result<(), Self::Error> {
        crate::pole::shut(token)
    }
}
