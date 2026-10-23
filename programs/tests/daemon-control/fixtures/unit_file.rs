use super::ControlConfig;

pub struct UnitFile {
    pub name: &'static str,
    pub control: ControlConfig,
    pub operator: super::operator_config::OperatorConfig,
}

impl UnitFile {
    pub fn name(&self) -> &'static str {
        self.name
    }
}
