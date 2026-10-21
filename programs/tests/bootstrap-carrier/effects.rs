use crate::{PieInfo, TaskId};
use std::vec::Vec;

pub struct Effects {
    pub now: u64,
    pub me: TaskId,
    pub creator: TaskId,
    pub unseals: usize,
    pub pushes: usize,
    pub ships: usize,
    pub narrows: usize,
    pub releases: usize,
    pub fail_unseal: bool,
    pub fail_push: bool,
    pub fail_ship: bool,
    pub fail_narrow: bool,
    pub fail_release: bool,
    pub pushed: Vec<u8>,
    pub shipped_to: Vec<TaskId>,
    pub pies: Vec<PieInfo>,
    pub pull_from: TaskId,
    pub pull_bytes: Vec<u8>,
    pub pull_len: usize,
    pub operations: Vec<(u8, crate::PieToken)>,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            now: 0,
            me: TaskId::new(0), creator: TaskId::new(0), pull_from: TaskId::new(0),
            unseals: 0, pushes: 0, ships: 0, narrows: 0, releases: 0,
            fail_unseal: false, fail_push: false, fail_ship: false,
            fail_narrow: false, fail_release: false,
            pushed: Vec::new(), shipped_to: Vec::new(), pies: Vec::new(),
            pull_bytes: Vec::new(), pull_len: 0, operations: Vec::new(),
        }
    }
}
