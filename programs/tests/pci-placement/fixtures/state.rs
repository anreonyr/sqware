use std::cell::RefCell;

use crate::{
    AcquireSource, FlatReference, GrantRange, Mark, MemoryFail, MemoryResult, Permission, PieFail,
    PieToken, TaskId,
};

pub const APERTURE: PieToken = PieToken::mint(41);
pub const DERIVED: PieToken = PieToken::mint(42);
pub const REMOTE: PieToken = PieToken::mint(43);
pub const BAR_SIZE: u64 = 4 * crate::PAGE_SIZE as u64;
const DEFAULT_APERTURE_PAGES: usize = 4 * BAR_SIZE as usize / crate::PAGE_SIZE;
pub use crate::{reference_state::ReferenceState, unseal::Unseal};
use crate::root::Root;

pub struct State {
    memory: [[u8; 256]; 2],
    request_accord: bool,
    request_release: bool,
    request_unseal: bool,
    request_reference_release: bool,
    derives: usize,
    unseals: Vec<Unseal>,
    references: Vec<ReferenceState>,
    reference_releases: Vec<FlatReference>,
    accords: Vec<(PieToken, TaskId, Permission, Mark)>,
    releases: Vec<PieToken>,
    revokes: Vec<(TaskId, PieToken)>,
    derived: Option<(PieToken, Permission, Mark, GrantRange)>,
    roots: Vec<Root>,
    next_reference: usize,
    next_root: usize,
    aperture_pages: usize,
    parent_heir: Option<GrantRange>,
    bar_address: [u64; 2],
    bar_size: [u64; 2],
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
            request_unseal: false,
            request_reference_release: false,
            derives: 0,
            unseals: Vec::new(),
            references: Vec::new(),
            reference_releases: Vec::new(),
            accords: Vec::new(),
            releases: Vec::new(),
            revokes: Vec::new(),
            derived: None,
            roots: Vec::new(),
            next_reference: 1,
            next_root: 0,
            aperture_pages: DEFAULT_APERTURE_PAGES,
            parent_heir: None,
            bar_address: [0; 2],
            bar_size: [BAR_SIZE; 2],
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

pub fn fail_next_unseal() {
    STATE.with(|state| state.borrow_mut().request_unseal = true);
}

pub fn fail_next_reference_release() {
    STATE.with(|state| state.borrow_mut().request_reference_release = true);
}

/// Model the declared aperture extent for the exact-full-window boundary.
pub fn set_aperture_pages(pages: usize) {
    STATE.with(|state| state.borrow_mut().aperture_pages = pages);
}

pub fn set_bar_size(requester: usize, size: u64) {
    STATE.with(|state| state.borrow_mut().bar_size[requester] = size);
}

pub fn bar_size(requester: u16) -> u64 {
    STATE.with(|state| state.borrow().bar_size[usize::from(requester)])
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

/// Ordinary task reference over one exact aperture window.
pub fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<FlatReference> {
    let AcquireSource::Source { token, offset, count } = source else {
        return Err(crate::make_fail(MemoryFail::Denied));
    };
    if token != APERTURE
        || retention != PieToken::NONE
        || count == 0
        || offset
            .checked_add(count)
            .is_none_or(|end| end > STATE.with(|state| state.borrow().aperture_pages))
    {
        return Err(crate::make_fail(MemoryFail::Denied));
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let id = state.next_reference;
        state.next_reference += 1;
        let handle = FlatReference::from_bytes(&(id as u64).to_le_bytes()).unwrap();
        state.references.push(ReferenceState {
            handle,
            offset,
            count,
            released: false,
            unsealed: false,
        });
        Ok(handle)
    })
}

pub fn release_reference(handle: FlatReference) -> MemoryResult<()> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let fail = std::mem::take(&mut state.request_reference_release);
        let Some(reference) = state.references.iter_mut().find(|item| item.handle == handle) else {
            return Err(crate::make_fail(MemoryFail::Denied));
        };
        if reference.released {
            return Err(crate::make_fail(MemoryFail::Denied));
        }
        if fail {
            return Err(crate::make_fail(MemoryFail::Denied));
        }
        reference.released = true;
        state.reference_releases.push(handle);
        Ok(())
    })
}

/// Mirror the kernel's independent child-root partition: disjoint windows create
/// separate ONLY roots and never publish the broad aperture's single heir.
pub fn unseal(handle: FlatReference, shared: bool) -> crate::PoleResult<PieToken> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if std::mem::take(&mut state.request_unseal) || shared {
            return Err(crate::make_fail(crate::PoleFail::Denied));
        }
        let Some(at) = state.references.iter().position(|item| item.handle == handle) else {
            return Err(crate::make_fail(crate::PoleFail::Denied));
        };
        let reference = state.references[at];
        if reference.released || reference.unsealed {
            return Err(crate::make_fail(crate::PoleFail::Denied));
        }
        let end = reference.offset + reference.count;
        if state.roots.iter().any(|root| {
            let root_end = root.offset + root.count;
            reference.offset < root_end && root.offset < end
        }) {
            return Err(crate::make_fail(crate::PoleFail::Busy));
        }
        let token = PieToken::mint(200 + state.next_root);
        state.next_root += 1;
        let permission = Permission::FETCH
            | Permission::STORE
            | Permission::VEST
            | Permission::ONLY;
        state.roots.push(Root {
            token,
            offset: reference.offset,
            count: reference.count,
            permission,
            mark: Mark::NONE,
        });
        state.references[at].unsealed = true;
        state.unseals.push(Unseal {
            reference: handle,
            shared,
            token,
            permission,
            mark: Mark::NONE,
            offset: reference.offset,
            count: reference.count,
        });
        Ok(token)
    })
}

/// The old same-parent derive path models the kernel's single ONLY heir.
pub fn derive(
    source: PieToken,
    permission: Permission,
    mark: Mark,
    range: GrantRange,
) -> crate::PieResult<PieToken> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        assert_eq!(source, APERTURE);
        let GrantRange::Pages { offset, count } = range else {
            panic!("exact BAR pages");
        };
        let end = offset + count;
        if end > state.aperture_pages {
            return Err(crate::make_fail(PieFail::Denied));
        }
        if permission.contains(Permission::ONLY) {
            if state.parent_heir.is_some() {
                return Err(crate::make_fail(PieFail::HandedOver));
            }
        }
        if state.roots.iter().any(|root| {
            let root_end = root.offset + root.count;
            offset < root_end && root.offset < end
        }) {
            return Err(crate::make_fail(PieFail::Busy));
        }
        if permission.contains(Permission::ONLY) {
            state.parent_heir = Some(range);
        }
        state.derives += 1;
        state.derived = Some((source, permission, mark, range));
        let token = if state.derives == 1 {
            DERIVED
        } else {
            PieToken::mint(42 + state.derives)
        };
        state.roots.push(Root {
            token,
            offset,
            count,
            permission,
            mark,
        });
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
        let root = state.roots.iter().find(|root| root.token == source).expect("live BAR root");
        Ok(if root.offset == 0 { REMOTE } else { PieToken::mint(100 + root.offset) })
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
        if state.roots.iter().any(|root| root.token == token
            && state.parent_heir == Some(GrantRange::Pages { offset: root.offset, count: root.count }))
        { state.parent_heir = None; }
        state.roots.retain(|root| root.token != token);
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

pub fn unseals() -> Vec<Unseal> {
    STATE.with(|state| state.borrow().unseals.clone())
}

pub fn references() -> Vec<ReferenceState> {
    STATE.with(|state| state.borrow().references.clone())
}

pub fn reference_releases() -> Vec<FlatReference> {
    STATE.with(|state| state.borrow().reference_releases.clone())
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
