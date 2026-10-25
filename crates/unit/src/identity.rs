use system_api::identity::Grant;

/// Identity facts owned by the identity module.
#[derive(Clone, Copy)]
pub struct Identity {
    /// Names this unit may publish for principals inside its own identity subtree.
    pub aliases: &'static [&'static str],
    pub grants: &'static [Grant],
}

impl Identity {
    pub const DEFAULT: Identity = Identity {
        aliases: &[],
        grants: &[],
    };
}
