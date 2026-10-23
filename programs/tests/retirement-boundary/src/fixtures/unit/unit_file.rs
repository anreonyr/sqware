use super::ControlConfig;

#[derive(Clone, Copy)]
pub struct UnitFile {
    pub name: &'static str,
    pub control: ControlConfig,
}
