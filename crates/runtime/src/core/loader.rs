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
    source: Source,
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

impl Source {
    fn matches(&self, bytes: &[u8], region: &loader::Region) -> bool {
        let Some((at, size)) = self.mapping else { return false; };
        if size != region.data_size { return false; }
        // SAFETY: cached sources retain a read-only mapping until removal from CACHE.
        let mapped = unsafe { core::slice::from_raw_parts(at as *const u8, size) };
        let end = region.prefix + region.file_size;
        mapped[..region.prefix].iter().all(|&byte| byte == 0)
            && mapped[region.prefix..end] == bytes[region.file_offset..region.file_offset + region.file_size]
            && mapped[end..].iter().all(|&byte| byte == 0)
    }
}

fn initialize(bytes: &[u8], region: &loader::Region) -> UnitResult<Source> {
    let private = region.flags & 4 != 0;
    let token = if private { pie::unseal_pole_exclusive(region.data_size) } else { pie::unseal_pole(region.data_size) }
        .map_err(|e| if matches!(e.source, env::PieFail::OoM) { fail(UnitFail::OoM) } else { fail(UnitFail::Denied) })?;
    let mut source = Source { token, mapping: None };
    pie::shut(token).map_err(|_| fail(UnitFail::Denied))?;
    let at = memory::map(TeamId::new(0), 0, region.data_size, token, 0, 6)
        .map_err(|e| if matches!(e.source, env::MemoryFail::OoM) { fail(UnitFail::OoM) } else { fail(UnitFail::Denied) })?;
    source.mapping = Some((at, region.data_size));
    // SAFETY: the parsed ELF payload fits the newly allocated, zeroed writable Pole.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr().add(region.file_offset),
        (at + region.prefix) as *mut u8, region.file_size) };
    if private {
        memory::munmap(at, region.data_size).map_err(|_| fail(UnitFail::Denied))?;
        source.mapping = None;
    }
    Ok(source)
}

fn shared(bytes: &[u8], region: &loader::Region) -> UnitResult<PieToken> {
    let owner = unit::self_id();
    let found = CACHE.with(|cache| cache.iter().find(|item|
        item.owner == owner && item.va == region.va && item.flags == region.flags && item.source.matches(bytes, region))
        .map(|item| item.source.token));
    if let Some(token) = found { return Ok(token); }
    let mut source = Some(initialize(bytes, region)?);
    let token = source.as_ref().unwrap().token;
    // Narrow also removes write access from the retained local mapping.
    pie::narrow(token, Permission::FETCH | Permission::VEST).map_err(|_| fail(UnitFail::Denied))?;
    // Prepare pages outside the mutex; discard a concurrent duplicate after releasing it.
    CACHE.with(|cache| -> UnitResult<PieToken> {
        if let Some(item) = cache.iter().find(|item|
            item.owner == owner && item.va == region.va && item.flags == region.flags && item.source.matches(bytes, region)) {
            return Ok(item.source.token);
        }
        cache.try_reserve(1).map_err(|_| fail(UnitFail::OoM))?;
        cache.push(Cached { owner, va: region.va, flags: region.flags, source: source.take().unwrap() });
        Ok(token)
    })
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
                let mut source = initialize(bytes, region)?;
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
        drop(item);
    }
}
