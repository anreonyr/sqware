use fack::prelude::Error;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum FaultKind {
    #[error("Execute OOM Instruction")]
    Instruction,
    #[error("Load OOM Data")]
    Load,
    #[error("Store OOM Data")]
    Store,
}
