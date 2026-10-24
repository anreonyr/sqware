use super::Retirement;

#[derive(Clone, Copy)]
pub struct ControlConfig {
    pub retirement: &'static [Retirement],
}
