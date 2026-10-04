use super::{fail, source::initialize};
use crate::system::loader::{Image, Loader, Unit};
use alloc::vec::Vec;
use env::{PieToken, TaskId, UnitFail, UnitResult, pie, unit};
use runtime::core::adapt;

impl Unit {
    pub fn spawn(mut self, args: &[usize], stack: usize) -> UnitResult<TaskId> {
        let task = adapt::spawn(self.team, self.entry, args, stack)?;
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
    pub fn mint(&mut self, image: Image<'_>) -> UnitResult<Unit> {
        let Image { bytes, kind } = image;
        let plan = loader::parse(bytes).map_err(|error| {
            if error == loader::Error::Memory {
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
                let token = if region.flags & 4 != 0 {
                    let mut source = initialize(bytes, region)?;
                    let token = source.token;
                    minted.private.push(token);
                    source.token = PieToken::NONE;
                    token
                } else {
                    self.shared(bytes, region)?
                };
                adapt::map(
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
            }
            if region.data_size < region.size {
                adapt::map(
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
