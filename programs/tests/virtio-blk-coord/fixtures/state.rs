use alloc::vec::Vec;
use core::cell::RefCell;

use env::{PAGE_SIZE, Permission, PieInfo, PieKind, PieToken, TaskId};

use crate::pci_api::BAR_MARK;
use super::region::Region;

pub const PCI: TaskId = TaskId::new(10);

pub struct State {
    pub regions: Vec<Region>,
    pub shut_fail: bool,
    pub grant_fail: bool,
    pub opens: usize,
    pub shuts: usize,
    pub grants: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
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

fn region<'a>(regions: &'a mut Vec<Region>, token: PieToken) -> &'a mut Region {
    if let Some(index) = regions.iter().position(|region| region.token == token) {
        return &mut regions[index];
    }
    regions.push(Region {
        token,
        memory: vec![0; PAGE_SIZE / 8].into_boxed_slice(),
        mapped: false,
    });
    regions.last_mut().unwrap()
}

pub fn inspect(token: PieToken) -> Result<PieInfo, ()> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        region(&mut state.regions, token);
        Ok(PieInfo {
            token,
            kind: PieKind::Pole,
            permission: Permission::FETCH | Permission::STORE | Permission::ONLY | Permission::VEST,
            owner: PCI,
            vestor: PCI,
            mark: BAR_MARK,
            alive: true,
        })
    })
}

pub fn open(token: PieToken) -> Result<(usize, usize), env::PoleFail> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.opens += 1;
        let region = region(&mut state.regions, token);
        region.mapped = true;
        Ok((region.memory.as_ptr() as usize, region.memory.len() * 8))
    })
}

pub fn shut(token: PieToken) -> Result<(), env::PoleFail> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.shuts += 1;
        let fail = state.shut_fail;
        let region = region(&mut state.regions, token);
        assert!(region.mapped, "must retain the actual mapped handle");
        if fail {
            return Err(env::PoleFail::Busy);
        }
        region.mapped = false;
        Ok(())
    })
}

pub fn accord(token: PieToken) -> Result<PieToken, env::PieFail> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let region = region(&mut state.regions, token);
        assert!(region.mapped, "attachment mapping must precede the child grant");
        if state.grant_fail {
            return Err(env::PieFail::OoM);
        }
        state.grants += 1;
        Ok(PieToken::mint(9))
    })
}
