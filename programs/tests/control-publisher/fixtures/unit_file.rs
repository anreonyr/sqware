pub struct UnitFile {
    pub name: &'static str,
    pub operator: crate::fixtures::operator_config::OperatorConfig,
}

impl UnitFile {
    pub fn name(&self) -> &'static str {
        self.name
    }
}
