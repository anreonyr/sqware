use env::{AcquireSource, PieToken};

use super::ScopeId;

pub struct Registration {
    pub scope: ScopeId,
    pub source: AcquireSource,
    pub retention: PieToken,
}
