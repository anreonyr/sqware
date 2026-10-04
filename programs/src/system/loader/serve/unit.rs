use crate::system::loader::core::elf;
use super::{fail, source::initialize};
use crate::system::loader::{Image, Loader, Unit};
use alloc::vec::Vec;
use env::{Permission, PieToken, TaskId, UnitFail, UnitResult, pie, unit};

impl Unit {
    pub fn spawn(mut self, args: &[usize], stack: usize) -> UnitResult<TaskId> {
        let task = runtime::core::task::spawn(self.team, self.entry, args, stack)?;
        self.committed = true;
        Ok(task)
    }
}
impl Drop for Unit {
    fn drop(&mut self) {
        if !self.committed {
            let _ = unit::oust(self.team);
            for token in self.private.drain(..) {
                let _ = pie::release(token);
            }
        }
    }
}

impl Loader {
    pub fn build(&mut self, image: Image<'_>) -> UnitResult<Unit> {
        let Image { bytes, kind } = image;
        let plan = elf::parse(bytes).map_err(|error| {
            if error == elf::Error::Memory {
                fail(UnitFail::OoM)
            } else {
                fail(UnitFail::BadImage)
            }
        })?;
        let mut private = Vec::new();
        private
            .try_reserve(plan.regions.len())
            .map_err(|_| fail(UnitFail::OoM))?;
        let mut minted = Unit {
            team: unit::build(kind)?,
            entry: plan.entry,
            private,
            committed: false,
        };
        for region in &plan.regions {
            if region.data_size != 0 {
                let cached = if region.flags & 4 == 0 {
                    self.cache.find(bytes, region)
                } else {
                    None
                };
                let mut source = match cached {
                    Some(_) => None,
                    None => Some(initialize(bytes, region)?),
                };
                let token = cached.unwrap_or_else(|| source.as_ref().unwrap().token);
                if region.flags & 4 == 0 && cached.is_none() {
                    pie::narrow(token, Permission::FETCH | Permission::VEST)
                        .map_err(|_| fail(UnitFail::Denied))?;
                }
                runtime::core::memory::map(
                    minted.team,
                    region.va,
                    region.data_size,
                    token,
                    0,
                    region.flags,
                )
                .map_err(|e| {
                    if matches!(e.source, env::MemoryFail::OoM) {
                        fail(UnitFail::OoM)
                    } else {
                        fail(UnitFail::Denied)
                    }
                })?;
                if let Some(mut source) = source.take() {
                    if region.flags & 4 != 0 {
                        minted.private.push(token);
                        source.token = PieToken::NONE;
                    } else {
                        // The destination mapping owns its backing even when admission fails.
                        let _ = self.cache.insert(region, source);
                    }
                }
            }
            if region.data_size < region.size {
                runtime::core::memory::map(
                    minted.team,
                    region.va + region.data_size,
                    region.size - region.data_size,
                    PieToken::NONE,
                    0,
                    region.flags,
                )
                .map_err(|e| {
                    if matches!(e.source, env::MemoryFail::OoM) {
                        fail(UnitFail::OoM)
                    } else {
                        fail(UnitFail::Denied)
                    }
                })?;
            }
        }
        Ok(minted)
    }
}
