use std::cell::RefCell;

use crate::{PieToken, TaskId};

use crate::source::Source;

pub struct State {
    pub failure: Option<&'static str>,
    pub events: Vec<&'static str>,
    pub poles: usize,
    pub references: usize,
    pub views: usize,
    pub view_address: usize,
    pub view_bytes: usize,
    pub mmio: Vec<u64>,
    pub held_queue: bool,
    pub blocked_queue: Option<usize>,
    pub blocked_lower: Option<usize>,
    pub clock: u64,
    pub sources: Vec<Source>,
    pub next_reference: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            failure: None,
            events: Vec::new(),
            poles: 0,
            references: 0,
            views: 0,
            view_address: 0,
            view_bytes: 0,
            mmio: Vec::new(),
            held_queue: false,
            blocked_queue: None,
            blocked_lower: None,
            clock: 0,
            sources: Vec::new(),
            next_reference: 2,
        }
    }
}

thread_local! {
    pub static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn reset() {
    STATE.with_borrow_mut(|state| *state = State::default());
}

pub fn step(operation: &'static str) -> Result<(), ()> {
    STATE.with_borrow_mut(|state| {
        state.events.push(operation);
        if state.failure == Some(operation) {
            Err(())
        } else {
            Ok(())
        }
    })
}

pub fn fail(operation: Option<&'static str>) {
    STATE.with_borrow_mut(|state| state.failure = operation);
}

pub fn counts() -> (usize, usize, usize) {
    STATE.with_borrow(|state| (state.poles, state.references, state.views))
}

pub fn events() -> Vec<&'static str> {
    STATE.with_borrow(|state| state.events.clone())
}

pub fn add_source(token: usize, pages: usize, vestor: TaskId, mark: crate::Mark) {
    let physical = 0x4000_0000 + (token as u64) * 0x0100_0000;
    STATE.with_borrow_mut(|state| {
        assert!(
            !state.sources.iter().any(|source| source.token == token),
            "duplicate fixture source"
        );
        state
            .sources
            .push(Source::new(token, pages, vestor, mark, physical));
    });
}

pub fn kill_source(token: usize) {
    STATE.with_borrow_mut(|state| {
        let source = state
            .sources
            .iter_mut()
            .find(|source| source.token == token)
            .expect("fixture source");
        source.alive = false;
    });
}

pub fn adopt(token: usize, count: usize) -> Option<usize> {
    STATE.with_borrow_mut(|state| {
        let handle = state.next_reference;
        let source = state
            .sources
            .iter_mut()
            .find(|source| source.token == token)?;
        if !source.alive || count == 0 || count > source.pages || source.reference.is_some() {
            return None;
        }
        state.next_reference = handle.checked_add(1)?;
        source.reference = Some(handle);
        state.references += 1;
        Some(handle)
    })
}

pub fn release_reference(handle: usize) -> bool {
    STATE.with_borrow_mut(|state| {
        let Some(source) = state
            .sources
            .iter_mut()
            .find(|source| source.reference == Some(handle))
        else {
            return false;
        };
        source.reference = None;
        state.references -= 1;
        true
    })
}

pub fn source_of(handle: usize) -> Option<(u64, usize)> {
    STATE.with_borrow(|state| {
        state
            .sources
            .iter()
            .find(|source| source.reference == Some(handle))
            .map(|source| (source.physical, source.pages))
    })
}

pub fn source(token: usize) -> Option<Source> {
    STATE.with_borrow(|state| {
        state
            .sources
            .iter()
            .find(|source| source.token == token)
            .cloned()
    })
}

pub fn source_token_for(handle: usize) -> Option<usize> {
    STATE.with_borrow(|state| {
        state
            .sources
            .iter()
            .find(|source| source.reference == Some(handle))
            .map(|source| source.token)
    })
}

pub fn window_source(token: PieToken) -> Option<usize> {
    STATE.with_borrow(|state| {
        state
            .sources
            .iter()
            .find(|source| source.token == token.get())
            .map(|source| source.bytes())
    })
}
