use env::{Permission, PieKind, TaskId, Wait};
use resource::raw::{self, Hole};
use system_api::bootstrap::{self as frame, Fail, Frame};
use wire::Message;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bootstrap {
    pub system: TaskId,
    pub provider: TaskId,
}

/// Locate this task's carrier by its real creator, then verify source, role and
/// the whole frame. Arbitrary Mark matches and duplicate candidates are refused.
pub fn claim(wait: Wait) -> Result<Bootstrap, Fail> {
    let creator = env::unit::creator();
    if creator == TaskId::new(0) {
        return Err(Fail::Source);
    }
    let mut found = None;
    for info in raw::pies() {
        if !info.alive || info.mark != frame::MARK {
            continue;
        }
        if info.kind != PieKind::Hole || info.owner != creator || info.vestor != creator {
            continue;
        }
        if info.permission != Permission::FETCH {
            continue;
        }
        if found.is_some() {
            return Err(Fail::Source);
        }
        found = Some(info.token);
    }
    let Some(token) = found else {
        return Err(Fail::Missing);
    };
    let mut bytes = [0u8; Frame::LEN];
    let (len, from) = Hole::from_raw(token)
        .pull(&mut bytes, wait)
        .map_err(|_| Fail::Channel)?;
    if len != Frame::LEN {
        return Err(Fail::Frame);
    }
    let frame = <Frame as Message>::fetch(&bytes).ok_or(Fail::Frame)?;
    if frame.version != frame::VERSION || frame.role != frame::ROLE {
        return Err(Fail::Frame);
    }
    if from != creator || frame.provider != creator || frame.system == TaskId::new(0) {
        return Err(Fail::Source);
    }
    Ok(Bootstrap {
        system: frame.system,
        provider: frame.provider,
    })
}
