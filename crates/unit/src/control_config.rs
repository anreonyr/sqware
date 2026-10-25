use crate::{Construction, Dependency, Ending, Ready, Retirement};

/// Lifecycle configuration owned by Control.
#[derive(Clone, Copy)]
pub struct ControlConfig {
    pub after: Option<&'static [Dependency]>,
    pub ending: Option<Ending>,
    pub ready: Ready,
    pub construction: &'static [Construction],
    pub retirement: &'static [Retirement],
    /// Deployment target group this unit may supervise. `None` keeps Control
    /// publication at the existing exact-root scope.
    pub supervision: Option<&'static str>,
}

impl ControlConfig {
    pub const DEFAULT: ControlConfig = ControlConfig {
        after: None,
        ending: None,
        ready: Ready::OnSpawn,
        construction: &[],
        retirement: &[],
        supervision: None,
    };
}
