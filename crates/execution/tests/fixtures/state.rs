use std::sync::Mutex;

use env::{
    AcquireSource, FlatReference, MemoryFail, MemoryResult, PieToken, Placement, ReferenceRange,
    TeamId, ViewControl, ViewId, ViewSelection,
};

pub struct State {
    pub next_handle: usize,
    pub live: Vec<usize>,
    pub released: Vec<usize>,
    pub acquire_calls: usize,
    pub release_calls: usize,
    pub attach_calls: usize,
    pub detach_calls: usize,
    pub set_access_calls: usize,
    pub last_range: Option<(usize, usize, usize)>,
    pub last_access: Option<(usize, usize, ViewSelection, u64)>,
    pub view: ViewId,
    pub address: usize,
    pub fail_acquire: bool,
    pub fail_release: bool,
    pub fail_attach: bool,
    pub fail_detach: bool,
    pub fail_set_access: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            next_handle: 1,
            live: Vec::new(),
            released: Vec::new(),
            acquire_calls: 0,
            release_calls: 0,
            attach_calls: 0,
            detach_calls: 0,
            set_access_calls: 0,
            last_range: None,
            last_access: None,
            view: ViewId::from_bytes(&3u64.to_le_bytes()).unwrap(),
            address: 0x4000_0000,
            fail_acquire: false,
            fail_release: false,
            fail_attach: false,
            fail_detach: false,
            fail_set_access: false,
        }
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn reset() {
    *STATE.lock().unwrap() = Some(State::default());
}

pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut state = STATE.lock().unwrap();
    f(state.as_mut().expect("fixture state reset"))
}

pub fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<FlatReference> {
    with(|state| {
        state.acquire_calls += 1;
        if state.fail_acquire
            || !matches!(source, AcquireSource::Source { .. })
            || retention != PieToken::NONE
        {
            return Err(env::make_fail(MemoryFail::Denied));
        }
        let handle = state.next_handle;
        state.next_handle += 1;
        state.live.push(handle);
        Ok(FlatReference::from_bytes(&(handle as u64).to_le_bytes()).unwrap())
    })
}

pub fn release(reference: FlatReference) -> MemoryResult<()> {
    with(|state| {
        state.release_calls += 1;
        if state.fail_release {
            return Err(env::make_fail(MemoryFail::Denied));
        }
        let Some(at) = state.live.iter().position(|held| *held == reference.get()) else {
            return Err(env::make_fail(MemoryFail::Denied));
        };
        state.live.remove(at);
        state.released.push(reference.get());
        Ok(())
    })
}

pub fn attach(
    range: ReferenceRange,
    _: TeamId,
    _: Placement,
    _: u64,
    _: ViewControl,
) -> MemoryResult<(ViewId, usize)> {
    with(|state| {
        state.attach_calls += 1;
        state.last_range = Some((range.reference.get(), range.offset, range.count));
        if state.fail_attach {
            return Err(env::make_fail(MemoryFail::NoRegion));
        }
        Ok((state.view, state.address))
    })
}

pub fn detach(team: TeamId, view: ViewId, selection: ViewSelection) -> MemoryResult<()> {
    with(|state| {
        state.detach_calls += 1;
        state.last_access = Some((team.get(), view.get(), selection, 0));
        if state.fail_detach {
            return Err(env::make_fail(MemoryFail::NoRegion));
        }
        Ok(())
    })
}

pub fn set_access(
    team: TeamId,
    view: ViewId,
    selection: ViewSelection,
    access: u64,
) -> MemoryResult<()> {
    with(|state| {
        state.set_access_calls += 1;
        state.last_access = Some((team.get(), view.get(), selection, access));
        if state.fail_set_access {
            return Err(env::make_fail(MemoryFail::NoRegion));
        }
        Ok(())
    })
}
