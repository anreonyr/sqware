use std::cell::RefCell;
use crate::{PAGE_SIZE, Permission, PieInfo, PieKind, PieToken, TaskId};

pub const BAR: PieToken = PieToken::mint(1);
pub const PCI: TaskId = TaskId::new(2);

pub struct State {
    pub info: PieInfo,
    pub memory: Box<[u64]>,
    pub mapped: bool,
    pub shut_fail: bool,
    pub grant_fail: bool,
    pub opens: usize,
    pub shuts: usize,
    pub grants: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            info: PieInfo {
                token: BAR,
                kind: PieKind::Pole,
                permission: Permission::FETCH | Permission::STORE | Permission::ONLY | Permission::VEST,
                owner: PCI,
                vestor: PCI,
                mark: crate::pci_api::BAR_MARK,
                alive: true,
            },
            memory: vec![0; 4 * PAGE_SIZE / 8].into_boxed_slice(),
            mapped: false,
            shut_fail: false,
            grant_fail: false,
            opens: 0,
            shuts: 0,
            grants: 0,
        }
    }
}

thread_local! {
    pub static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn reset() {
    STATE.with(|state| *state.borrow_mut() = State::default());
}
