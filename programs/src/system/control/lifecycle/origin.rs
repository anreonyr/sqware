use env::TaskId;

/// Who asked for the in-flight lifecycle action.
///
/// The origin is request context: it decides intent updates and recovery
/// authority independently of the current lifecycle stage. An accepted Ruin
/// takeover or a failure rollback replaces it explicitly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// An explicit Control request from its actual IPC sender.
    External,
    /// Deployment startup sequencing.
    Startup,
    /// Natural task-exit reaping.
    Reaper,
    /// Global retirement closing its remaining providers.
    Closing,
    /// Internal failure cleanup.
    Rollback,
    /// A declared supervisor asking to recover a fully retired target.
    Recovery { caller: TaskId },
}
