use super::Dependency;

pub struct ControlConfig {
    pub supervision: Option<&'static str>,
    pub after: Option<&'static [Dependency]>,
}
