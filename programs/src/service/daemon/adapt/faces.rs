use system_client::control::{Face, Observe};

/// The two scoped Control faces this process holds.
pub struct Faces {
    pub observe: Observe,
    pub recover: Face,
}
