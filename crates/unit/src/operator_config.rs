use crate::operator::Namespace;

/// Static publication and construction rights owned by Operator.
#[derive(Clone, Copy)]
pub struct OperatorConfig {
    pub namespace: &'static [Namespace],
}

impl OperatorConfig {
    pub const DEFAULT: OperatorConfig = OperatorConfig { namespace: &[] };
}
