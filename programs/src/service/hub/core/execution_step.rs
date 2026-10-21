#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionStep {
    Identify,
    Create,
    Populate,
    Deliver,
    Restrict,
    Embark,
    Running,
    Stop,
    Oust,
    Release,
    Done,
}
