use super::ControlConfig;

pub struct UnitFile {
    pub name: &'static str,
    pub control: ControlConfig,
}

impl UnitFile {
    pub fn name(&self) -> &'static str {
        self.name
    }
}
