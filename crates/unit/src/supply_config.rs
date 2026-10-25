/// Resources required during preparation.
#[derive(Clone, Copy)]
pub struct SupplyConfig {
    pub bindings: &'static [crate::Binding],
}

impl SupplyConfig {
    pub const DEFAULT: SupplyConfig = SupplyConfig { bindings: &[] };
}
