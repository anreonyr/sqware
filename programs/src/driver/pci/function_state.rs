use alloc::vec::Vec;
use env::TaskId;
use super::bar::Bar;

pub(super) struct FunctionState {
    pub requester: u16,
    pub recipient: TaskId,
    pub serial: Option<u64>,
    pub floor: u64,
    pub master: bool,
    pub disabled: bool,
    pub bars: Vec<Bar>,
}
