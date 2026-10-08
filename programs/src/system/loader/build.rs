use super::{Image, Loader, Spawn, Unit, fail, image::elf, mapping::initialize};
use alloc::vec::Vec;
use env::{Permission, PieToken, UnitFail, UnitResult, VirtAddr, Wait, pie, unit};
use resource::raw::{Hole, inspect};
use system_api::loader as frame;
use system_api::loader::Built;

pub(crate) struct Source<'a> {
    pub from: env::TaskId,
    pub ask: &'a frame::Ask,
}

pub(crate) fn snapshot(source: Source<'_>) -> Result<Vec<u8>, frame::Fail> {
    let ask = source.ask;
    if !matches!(inspect(ask.image), Ok((vestor, _, mark)) if vestor == source.from && mark == frame::IMAGE)
    {
        return Err(frame::Fail::Denied);
    }
    let offset = usize::try_from(ask.offset).map_err(|_| frame::Fail::BadImage)?;
    let len = usize::try_from(ask.len).map_err(|_| frame::Fail::BadImage)?;
    if len == 0 || len > frame::MAX_IMAGE {
        return Err(frame::Fail::BadImage);
    }
    let page = ask.image;
    let (at, size) = resource::raw::open(page).map_err(|_| frame::Fail::Denied)?;
    let result = (|| {
        if offset.checked_add(len).is_none_or(|end| end > size) {
            return Err(frame::Fail::BadImage);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| frame::Fail::Full)?;
        bytes.resize(len, 0);
        let copy = pie::unseal_hole(env::Mark::NONE).map_err(|_| frame::Fail::Full)?;
        let result = (|| {
            env::mail::push(copy, VirtAddr::new(at + offset), len)
                .map_err(|_| frame::Fail::BadImage)?;
            let (n, _) = Hole::from_raw(copy)
                .pull(&mut bytes, Wait::POLL)
                .map_err(|_| frame::Fail::BadImage)?;
            if n != len {
                return Err(frame::Fail::BadImage);
            }
            Ok(bytes)
        })();
        let _ = pie::release(copy);
        result
    })();
    let _ = pie::shut(page);
    result
}

pub(super) fn load(loader: &mut Loader, image: Image<'_>) -> UnitResult<Unit> {
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
                loader.cache.find(bytes, region)
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
            execution::memory::map(
                minted.team,
                region.va,
                region.data_size,
                token,
                0,
                region.flags,
            )
            .map_err(memory_fail)?;
            if let Some(mut source) = source.take() {
                if region.flags & 4 != 0 {
                    minted.private.push(token);
                    source.token = PieToken::NONE;
                } else {
                    let _ = loader.cache.insert(region, source);
                }
            }
        }
        if region.data_size < region.size {
            execution::memory::map(
                minted.team,
                region.va + region.data_size,
                region.size - region.data_size,
                PieToken::NONE,
                0,
                region.flags,
            )
            .map_err(memory_fail)?;
        }
    }
    Ok(minted)
}

pub(super) fn construct(
    loader: &mut Loader,
    image: Image<'_>,
    spawn: Spawn<'_>,
) -> Result<Built, frame::Fail> {
    let unit = loader.build(image).map_err(unit_fail)?;
    let team = unit.team();
    let task = unit.spawn(spawn.args, spawn.stack).map_err(unit_fail)?;
    Ok(Built { task, team })
}

fn memory_fail(error: erra::Error<env::MemoryFail>) -> erra::Error<UnitFail> {
    if matches!(error.source, env::MemoryFail::OoM) {
        fail(UnitFail::OoM)
    } else {
        fail(UnitFail::Denied)
    }
}

fn unit_fail(error: erra::Error<UnitFail>) -> frame::Fail {
    match error.source {
        UnitFail::OoM => frame::Fail::Full,
        UnitFail::BadImage | UnitFail::BadEntry => frame::Fail::BadImage,
        _ => frame::Fail::Denied,
    }
}
