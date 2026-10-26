use resource_impl::kernel::CapabilityKernel;

/// 本夹具唯一的能力内核适配：错误域保持既有表达，Busy 无法表示。
pub struct Kernel;

impl CapabilityKernel for Kernel {
    type PieError = ();
    type HoleError = erra::Error<abi::HoleFail>;
    type PoleError = erra::Error<abi::PoleFail>;

    fn unseal_hole(mark: abi::Mark) -> Result<abi::PieToken, Self::HoleError> {
        crate::hole::unseal(mark, abi::HoleLimits::default())
    }

    fn seal_hole(token: abi::PieToken) -> Result<(), Self::HoleError> {
        crate::hole::seal(token)
    }

    fn unseal_pole(
        reference: abi::FlatReference,
        shared: bool,
    ) -> Result<abi::PieToken, Self::PoleError> {
        crate::pole::unseal(reference, shared)
    }

    fn derive(
        token: abi::PieToken,
        permission: abi::Permission,
        mark: abi::Mark,
        range: abi::GrantRange,
    ) -> Result<abi::PieToken, Self::PieError> {
        crate::pie::derive(token, permission, mark, range)
    }

    fn accord(
        token: abi::PieToken,
        task: abi::TaskId,
        permission: abi::Permission,
        mark: abi::Mark,
    ) -> Result<abi::PieToken, Self::PieError> {
        crate::pie::accord(token, task, permission, mark)
    }

    fn release(token: abi::PieToken) -> Result<(), Self::PieError> {
        crate::pie::release(token, abi::ReleaseMode::Revoke)
    }

    fn revoke(task: abi::TaskId, token: abi::PieToken) -> Result<(), Self::PieError> {
        crate::pie::revoke(task, token)
    }

    fn is_busy(_: &Self::PieError) -> bool {
        false
    }

    fn starve() {
        unreachable!("operator-watch fixture cannot represent Busy")
    }
}
