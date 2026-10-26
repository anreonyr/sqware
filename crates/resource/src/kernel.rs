//! 能力机制对内核的静态缝：每个入口只尝试一次，权威仍在 Kernel。

use env::{FlatReference, GrantRange, Mark, Permission, PieToken, TaskId};

/// 能力资源的单次内核调用。重试与让步由 [`crate::cleanup`] 负责。
pub trait CapabilityKernel {
    type PieError;
    type HoleError;
    type PoleError;

    fn unseal_hole(mark: Mark) -> Result<PieToken, Self::HoleError>;
    fn seal_hole(token: PieToken) -> Result<(), Self::HoleError>;
    fn unseal_pole(reference: FlatReference, shared: bool) -> Result<PieToken, Self::PoleError>;
    fn derive(
        token: PieToken,
        permission: Permission,
        mark: Mark,
        range: GrantRange,
    ) -> Result<PieToken, Self::PieError>;
    fn accord(
        token: PieToken,
        task: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> Result<PieToken, Self::PieError>;
    fn release(token: PieToken) -> Result<(), Self::PieError>;
    fn revoke(task: TaskId, token: PieToken) -> Result<(), Self::PieError>;
    fn is_busy(error: &Self::PieError) -> bool;
    fn starve();
}
