use crate::system::loader::{Image, Loader};
use alloc::vec::Vec;
use env::{UnitFail, VirtAddr, Wait, pie};
use protocol::system::loader::{Built, frame};
use ::resource::raw::{HolePie, inspect};

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
    let (at, size) = ::resource::raw::open(page).map_err(|_| frame::Fail::Denied)?;
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
            // Kernel copying holds the mapping lock; revocation becomes a copy error.
            env::mail::push(copy, VirtAddr::new(at + offset), len)
                .map_err(|_| frame::Fail::BadImage)?;
            let (n, _) = HolePie::from_token(copy)
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
pub struct Source<'a> {
    pub from: env::TaskId,
    pub ask: &'a frame::Ask,
}
pub struct Spawn<'a> {
    pub args: &'a [usize],
    pub stack: usize,
}
pub fn construct(
    loader: &mut Loader,
    image: Image<'_>,
    spawn: Spawn<'_>,
) -> Result<Built, frame::Fail> {
    let unit = loader.build(image).map_err(unit_fail)?;
    let team = unit.team();
    let task = unit.spawn(spawn.args, spawn.stack).map_err(unit_fail)?;
    Ok(Built { task, team })
}
fn unit_fail(error: erra::Error<UnitFail>) -> frame::Fail {
    match error.source {
        UnitFail::OoM => frame::Fail::Full,
        UnitFail::BadImage | UnitFail::BadEntry => frame::Fail::BadImage,
        _ => frame::Fail::Denied,
    }
}
