#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionStep {
    Identify,
    Embark,
    Running,
    Stop,
    Oust,
    Done,
}
