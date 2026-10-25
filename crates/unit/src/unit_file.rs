use crate::{LoaderConfig, OperatorConfig, SupplyConfig};
use crate::{ControlConfig, Identity};

/// A program's module configuration.
#[derive(Clone, Copy)]
pub struct UnitFile {
    pub name: &'static str,
    pub loader: LoaderConfig,
    pub operator: OperatorConfig,
    pub identity: Identity,
    pub control: ControlConfig,
    pub supply: SupplyConfig,
}

impl UnitFile {
    /// Declaration name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn listed(&self) -> bool {
        self.control.after.is_some()
    }

    /// This unit's explicitly declared resources and startup credentials.
    pub fn supply(&self) -> &'static [crate::Binding] {
        self.supply.bindings
    }

    pub fn valid(&self) -> bool {
        crate::valid_supplies(self.supply())
            && self.control.retirement.iter().all(crate::Retirement::valid)
    }
}
