//! ELF planning and construction from the existing Unit, Memory, and Pie calls.
use alloc::vec::Vec;
use env::{Permission, PieToken, ProgramKind, TaskId, TeamId, UnitFail, UnitResult};
use crate::env::{memory, pie, unit};
use super::task::lock::Lock;

fn fail(source: UnitFail) -> erra::Error<UnitFail> { erra::Error::new("constructing ELF image", source) }

struct Cached {
    owner: TaskId,
    va: usize,
    flags: u64,
    payload: Vec<u8>,
    token: PieToken,
}
// SAFETY: cache lookup checks the calling TaskId before exposing a task-local token.
unsafe impl Send for Cached {}
static CACHE: Lock<Vec<Cached>> = Lock::new(Vec::new());

struct Source {
    token: PieToken,
    mapping: Option<(usize, usize)>,
}
impl Drop for Source {
    fn drop(&mut self) {
        if let Some((at, size)) = self.mapping.take() { let _ = memory::munmap(at, size); }
        if self.token != PieToken::NONE { let _ = pie::release(self.token); }
    }
}

/// Owns an unpublished child and its private root tokens until the first Spawn.
pub struct Image {
    team: TeamId,
    entry: usize,
    private: Vec<PieToken>,
    committed: bool,
}
impl Image {
    pub fn team(&self) -> TeamId { self.team }
    pub fn spawn(mut self, args: &[usize], stack: usize) -> UnitResult<TaskId> {
        let task = unit::spawn(self.team, self.entry, args, stack)?;
        self.committed = true;
        Ok(task)
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        if !self.committed {
            let _ = unit::oust(self.team);
            for token in self.private.drain(..) { let _ = pie::release(token); }
        }
    }
}

fn payload(bytes: &[u8], region: &loader::Region) -> UnitResult<Vec<u8>> {
    let mut data = Vec::new();
    data.try_reserve_exact(region.data_size).map_err(|_| fail(UnitFail::OoM))?;
    data.resize(region.data_size, 0);
    data[region.prefix..region.prefix + region.file_size]
        .copy_from_slice(&bytes[region.file_offset..region.file_offset + region.file_size]);
    Ok(data)
}

fn initialize(data: &[u8], private: bool) -> UnitResult<Source> {
    let token = if private { pie::unseal_pole_exclusive(data.len()) } else { pie::unseal_pole(data.len()) }
        .map_err(|e| if matches!(e.source, env::PieFail::OoM) { fail(UnitFail::OoM) } else { fail(UnitFail::Denied) })?;
    let mut source = Source { token, mapping: None };
    pie::shut(token).map_err(|_| fail(UnitFail::Denied))?;
    let at = memory::map(TeamId::new(0), 0, data.len(), token, 0, 6)
        .map_err(|e| if matches!(e.source, env::MemoryFail::OoM) { fail(UnitFail::OoM) } else { fail(UnitFail::Denied) })?;
    source.mapping = Some((at, data.len()));
    // SAFETY: Mmap returned a private writable view covering data.len() bytes.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), at as *mut u8, data.len()) };
    memory::munmap(at, data.len()).map_err(|_| fail(UnitFail::Denied))?;
    source.mapping = None;
    Ok(source)
}

fn shared(bytes: &[u8], region: &loader::Region) -> UnitResult<PieToken> {
    let owner = unit::self_id();
    let data = payload(bytes, region)?;
    let found = CACHE.with(|cache| cache.iter().find(|item|
        item.owner == owner && item.va == region.va && item.flags == region.flags && item.payload == data)
        .map(|item| item.token));
    if let Some(token) = found { return Ok(token); }
    let mut source = initialize(&data, false)?;
    pie::narrow(source.token, Permission::FETCH | Permission::VEST).map_err(|_| fail(UnitFail::Denied))?;
    // Only short table operations hold the user mutex. Concurrent builders may prepare duplicates.
    let token = CACHE.with(|cache| -> UnitResult<PieToken> {
        if let Some(item) = cache.iter().find(|item|
            item.owner == owner && item.va == region.va && item.flags == region.flags && item.payload == data) {
            return Ok(item.token);
        }
        cache.try_reserve(1).map_err(|_| fail(UnitFail::OoM))?;
        let token = source.token;
        cache.push(Cached { owner, va: region.va, flags: region.flags, payload: data, token });
        source.token = PieToken::NONE;
        Ok(token)
    })?;
    Ok(token)
}

pub fn build(bytes: &[u8], kind: ProgramKind) -> UnitResult<Image> {
    let plan = loader::parse(bytes).map_err(|error|
        if error == loader::Error::Memory { fail(UnitFail::OoM) } else { fail(UnitFail::BadImage) })?;
    let mut private = Vec::new();
    private.try_reserve(plan.regions.len()).map_err(|_| fail(UnitFail::OoM))?;
    let mut image = Image { team: unit::build(kind)?, entry: plan.entry, private, committed: false };
    for region in &plan.regions {
        if region.data_size != 0 {
            let token = if region.flags & 4 != 0 {
                let mut source = initialize(&payload(bytes, region)?, true)?;
                let token = source.token;
                image.private.push(token);
                source.token = PieToken::NONE;
                token
            } else { shared(bytes, region)? };
            memory::map(image.team, region.va, region.data_size, token, 0, region.flags)
                .map_err(|e| if matches!(e.source, env::MemoryFail::OoM) { fail(UnitFail::OoM) } else { fail(UnitFail::Denied) })?;
        }
        if region.data_size < region.size {
            memory::map(image.team, region.va + region.data_size, region.size - region.data_size,
                PieToken::NONE, 0, region.flags)
                .map_err(|e| if matches!(e.source, env::MemoryFail::OoM) { fail(UnitFail::OoM) } else { fail(UnitFail::Denied) })?;
        }
    }
    Ok(image)
}


pub(crate) fn retire() {
    let owner = unit::self_id();
    loop {
        let item = CACHE.with(|cache| cache.iter().position(|item| item.owner == owner)
            .map(|index| cache.swap_remove(index)));
        let Some(item) = item else { break };
        let _ = pie::release(item.token);
        drop(item);
    }
}
