use std::cell::RefCell;

use crate::{GrantRange, Mark, Permission, PieFail, PieToken, TaskId};

pub const APERTURE: PieToken = PieToken::mint(41);
pub const DERIVED: PieToken = PieToken::mint(42);
pub const REMOTE: PieToken = PieToken::mint(43);
pub const BAR_SIZE: u64 = 4 * crate::PAGE_SIZE as u64;

pub struct State {
    memory: [[u8; 256]; 2],
    request_accord: bool,
    request_release: bool,
    derives: usize,
    accords: Vec<(PieToken, TaskId, Permission, Mark)>,
    releases: Vec<PieToken>,
    revokes: Vec<(TaskId, PieToken)>,
    derived: Option<(PieToken, Permission, Mark, GrantRange)>,
    roots: Vec<(PieToken, GrantRange)>,
    bar_address: [u64; 2],
}

impl Default for State {
    fn default() -> Self {
        let mut memory = [0u8; 256];
        memory[0] = 0x01;
        memory[1] = 0xF4;
        memory[2] = 0x42;
        memory[3] = 0x10;
        memory[crate::ecam::CAPABILITY as usize] = 0x40;
        capability(&mut memory, 0x40, 0x50, 1, 0, 0x4000);
        capability(&mut memory, 0x50, 0x60, 2, 0x1000, 0x4000);
        write32(&mut memory, 0x50 + 16, 4);
        capability(&mut memory, 0x60, 0x70, 4, 0x3000, 0x4000);
        Self {
            memory: [memory; 2],
            request_accord: false,
            request_release: false,
            derives: 0,
            accords: Vec::new(),
            releases: Vec::new(),
            revokes: Vec::new(),
            derived: None,
            roots: Vec::new(),
            bar_address: [0; 2],
        }
    }
}

fn capability(
    memory: &mut [u8; 256],
    at: usize,
    next: u8,
    kind: u8,
    offset: u32,
    span: u32,
) {
    memory[at] = crate::ecam::CAP_VENDOR;
    memory[at + 1] = next;
    memory[at + 2] = 16;
    memory[at + 3] = kind;
    memory[at + 4] = 0;
    write32(memory, at + 8, offset);
    write32(memory, at + 12, span);
}

fn write32(memory: &mut [u8; 256], at: usize, value: u32) {
    memory[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn reset() {
    STATE.with(|state| *state.borrow_mut() = State::default());
}

pub fn fail_next_accord() {
    STATE.with(|state| state.borrow_mut().request_accord = true);
}

pub fn fail_next_release() {
    STATE.with(|state| state.borrow_mut().request_release = true);
}

pub fn read(at: usize) -> u8 {
    STATE.with(|state| state.borrow().memory[(at >> 12) & 1][at & 0xFF])
}

pub fn write(at: usize, value: u8) {
    STATE.with(|state| state.borrow_mut().memory[(at >> 12) & 1][at & 0xFF] = value);
}

pub fn requester() -> u16 {
    1
}

pub fn place_bar(requester: u16, address: u64) {
    STATE.with(|state| state.borrow_mut().bar_address[requester as usize] = address);
}

pub fn bar_address() -> u64 {
    STATE.with(|state| state.borrow().bar_address[1])
}

pub fn derive(
    source: PieToken,
    permission: Permission,
    mark: Mark,
    range: GrantRange,
) -> crate::PieResult<PieToken> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        assert_eq!(source, APERTURE);
        assert!(
            state.roots.iter().all(|(_, held)| *held != range),
            "each exact BAR range must be derived once while its root is live"
        );
        state.derives += 1;
        state.derived = Some((source, permission, mark, range));
        let token = if state.derives == 1 { DERIVED } else { PieToken::mint(42 + state.derives) };
        state.roots.push((token, range));
        Ok(token)
    })
}

pub fn accord(
    source: PieToken,
    peer: TaskId,
    permission: Permission,
    mark: Mark,
) -> crate::PieResult<PieToken> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.accords.push((source, peer, permission, mark));
        if state.request_accord {
            state.request_accord = false;
            return Err(crate::make_fail(PieFail::Denied));
        }
        let (_, range) = state.roots.iter().find(|(token, _)| *token == source).expect("live BAR root");
        let GrantRange::Pages { offset, .. } = *range else { panic!("exact BAR pages"); };
        Ok(if offset == 0 { REMOTE } else { PieToken::mint(100 + offset) })
    })
}

pub fn release(token: PieToken) -> crate::PieResult<()> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.releases.push(token);
        if state.request_release {
            state.request_release = false;
            return Err(crate::make_fail(PieFail::Denied));
        }
        state.roots.retain(|(held, _)| *held != token);
        if state.roots.is_empty() {
            state.derived = None;
        }
        Ok(())
    })
}

pub fn revoke(peer: TaskId, token: PieToken) {
    STATE.with(|state| state.borrow_mut().revokes.push((peer, token)));
}

pub fn derives() -> usize {
    STATE.with(|state| state.borrow().derives)
}

pub fn derived() -> (PieToken, Permission, Mark, GrantRange) {
    STATE.with(|state| state.borrow().derived.expect("derived BAR"))
}

pub fn accords() -> Vec<(PieToken, TaskId, Permission, Mark)> {
    STATE.with(|state| state.borrow().accords.clone())
}

pub fn releases() -> Vec<PieToken> {
    STATE.with(|state| state.borrow().releases.clone())
}

pub fn revokes() -> Vec<(TaskId, PieToken)> {
    STATE.with(|state| state.borrow().revokes.clone())
}
