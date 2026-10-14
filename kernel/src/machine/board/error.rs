use fack::prelude::Error;

#[derive(Error, Debug)]
pub enum MachineError {
    #[error("invalid device tree: {0}")]
    DeviceTree(fdt::FdtError),
    #[error("device tree has no /cpus node")]
    MissingCpus,
    #[error("device tree has no usable memory region")]
    MissingMemory,
    #[error("RAM does not contain the kernel and root stack")]
    InvalidMemory,
    #[error("invalid hart topology: count={count}, boot={boot}")]
    Harts { count: usize, boot: usize },
    #[error("invalid initrd region")]
    Initrd,
    #[error("device tree region is outside RAM")]
    DtbRegion,
}
