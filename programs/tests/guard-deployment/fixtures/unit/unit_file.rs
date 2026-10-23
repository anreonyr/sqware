#[derive(Clone, Copy)]
pub struct UnitFile {
    pub name: &'static str,
    pub control: crate::control_config::ControlConfig,
}

impl UnitFile {
    pub fn name(&self) -> &'static str {
        self.name
    }
}
