use env::TaskId;

/// A device coordinate; authority comes from its authenticated directory door.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceId {
    pub source: TaskId,
    pub key: u64,
}
