use resource_impl::mapping_kernel::MappingKernel;

/// 本夹具唯一的映射内核适配：两个入口都落到既有 STATE。
pub struct Kernel;

impl MappingKernel for Kernel {
    type Error = env::PoleFail;

    fn open(token: env::PieToken) -> Result<(usize, usize), Self::Error> {
        crate::raw::open(token)
    }

    fn shut(token: env::PieToken) -> Result<(), Self::Error> {
        crate::pole::shut(token)
    }
}
