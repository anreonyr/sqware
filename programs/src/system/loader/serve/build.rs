use super::answer::Incoming;
use crate::system::loader::Image;
use crate::system::{
    control::{
        core::{
            instance::{INSTANCE_CAP, Instance},
            unit::State,
        },
        serve::unit::Control,
    },
    identity::serve::install::Roster,
};
use alloc::vec::Vec;
use env::{ProgramKind, UnitFail, VirtAddr, Wait, pie};
use protocol::system::loader::{Built, frame};
use runtime::core::res::pie::{HolePie, inspect};

fn snapshot(incoming: &Incoming) -> Result<Vec<u8>, frame::Fail> {
    let ask = &incoming.ask;
    if !matches!(inspect(ask.image), Ok((vestor, _, mark)) if vestor == incoming.from && mark == frame::IMAGE)
    {
        return Err(frame::Fail::Denied);
    }
    let offset = usize::try_from(ask.offset).map_err(|_| frame::Fail::BadImage)?;
    let len = usize::try_from(ask.len).map_err(|_| frame::Fail::BadImage)?;
    if len == 0 || len > frame::MAX_IMAGE {
        return Err(frame::Fail::BadImage);
    }
    let page = ask.image;
    let (at, size) = runtime::core::res::pie::open(page).map_err(|_| frame::Fail::Denied)?;
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
pub(super) fn construct(
    control: &mut Control,
    roster: &Roster,
    incoming: &Incoming,
) -> Result<Built, frame::Fail> {
    let bytes = snapshot(incoming)?;
    let mut args = [0; frame::MAX_ARGS];
    let count = incoming.ask.count as usize;
    if count > args.len() {
        return Err(frame::Fail::Bad);
    }
    for (to, from) in args.iter_mut().zip(&incoming.ask.args[..count]) {
        *to = *from as usize;
    }
    construct_image(
        control,
        roster,
        Build {
            image: Image {
                bytes: &bytes,
                kind: ProgramKind::User,
            },
            spawn: Spawn {
                owner: incoming.from,
                args: &args[..count],
                stack: incoming.ask.stack as usize,
            },
            subject: None,
        },
    )
}

pub(crate) struct Spawn<'a> {
    pub owner: env::TaskId,
    pub args: &'a [usize],
    pub stack: usize,
}
pub(crate) struct Build<'a> {
    pub image: Image<'a>,
    pub spawn: Spawn<'a>,
    pub subject: Option<protocol::system::identity::Subject>,
}
pub(crate) fn construct_image(
    control: &mut Control,
    roster: &Roster,
    build: Build<'_>,
) -> Result<Built, frame::Fail> {
    let Build {
        image,
        spawn: Spawn { owner, args, stack },
        subject,
    } = build;
    if control.instances.len() >= INSTANCE_CAP {
        if let Some(at) = control
            .instances
            .iter()
            .position(|item| item.team.is_none())
        {
            control.instances.remove(at);
        } else {
            return Err(frame::Fail::Full);
        }
    }
    control
        .instances
        .try_reserve(1)
        .map_err(|_| frame::Fail::Full)?;
    let unit = control.loader.build(image).map_err(unit_fail)?;
    let team = unit.team();
    let task = unit.spawn(args, stack).map_err(unit_fail)?;
    control.instances.push(Instance {
        owner,
        task,
        team: Some(team),
        state: State::Stopping,
        claimed: false,
        claim_until: env::chrono::clock() + frame::CLAIM_MS as u64 * 1_000_000,
    });
    match subject {
        Some(subject) => roster.install_subject(task, subject),
        None => roster.inherit(task, owner),
    }
    .map_err(|_| frame::Fail::Denied)?;
    control.instances.last_mut().unwrap().state = State::Debarked;
    Ok(Built { task, team })
}
fn unit_fail(error: erra::Error<UnitFail>) -> frame::Fail {
    match error.source {
        UnitFail::OoM => frame::Fail::Full,
        UnitFail::BadImage | UnitFail::BadEntry => frame::Fail::BadImage,
        _ => frame::Fail::Denied,
    }
}
