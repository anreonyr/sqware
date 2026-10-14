use crate::core::memory::address::VirtAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameKind {
    Root,
    Kernel,
    Normal,
    Unknown,
}

impl FrameKind {
    pub(crate) fn of(pc: VirtAddr) -> Self {
        let k = crate::machine::image::kernel_edge();
        if pc.as_usize() >= k && pc.as_usize() < k + crate::machine::image::ROOT_STACK_SIZE {
            Self::Root
        } else if pc.is_kernel() {
            Self::Kernel
        } else if pc.is_user() {
            Self::Normal
        } else {
            Self::Unknown
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Root => "R",
            Self::Kernel => "K",
            Self::Normal => "N",
            Self::Unknown => "?",
        }
    }
}
