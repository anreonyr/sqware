use ::schedule::{Progress, Res, ResMut};
use crate::system::control::{core::unit::State, serve::{Fail, unit::Control}};
use env::{Wait, unit};
use system_api::control as call;

pub fn reap(mut control: ResMut<Control>, flow: Res<super::frame::Flow>) -> Result<Progress, Fail> {
    for item in &mut control.instances {
        if item.team.is_none() {
            continue;
        }
        if flow.settling
            || (!item.claimed && env::chrono::clock() >= item.claim_until)
            || unit::join(item.owner, Wait::POLL).unwrap_or(true)
            || unit::join(item.task, Wait::POLL).unwrap_or(true)
        {
            item.stop();
        }
        if item.state == State::Stopping {
            let _ = env::room::doom(item.task);
        }
    }
    control
        .instances
        .retain(|item| item.team.is_some() || !unit::join(item.owner, Wait::POLL).unwrap_or(true));
    Ok(Progress::Done)
}
pub fn pending(
    control: Res<Control>,
    mut bound: ResMut<super::frame::Bound>,
) -> Result<Progress, Fail> {
    for item in control
        .instances
        .iter()
        .filter(|item| !item.claimed && item.team.is_some())
    {
        let ms = item
            .claim_until
            .saturating_sub(env::chrono::clock())
            .div_ceil(1_000_000)
            .max(1) as usize;
        bound.0 = match bound.0 {
            Wait::AtMost(old) => Wait::AtMost(old.min(ms)),
            Wait::Forever => Wait::AtMost(ms),
        };
    }
    if control
        .instances
        .iter()
        .any(|item| matches!(item.state, State::Starting | State::Stopping))
    {
        bound.0 = Wait::AtMost(1);
    }
    Ok(Progress::Done)
}

pub fn publication(
    mut watch: ResMut<crate::system::control::serve::watch::Watch>,
    mut mounts: ResMut<crate::system::boot::Mounts>,
) -> Result<Progress, &'static str> {
    let entry = env::pie::unseal_hole(call::ASK_MARK).map_err(|_| "instance entry")?;
    watch.instance = Some(entry);
    mounts.0.push(crate::system::run::publication::Internal {
        road: call::INSTANCE.to_path_buf(),
        entry,
        access: (system_client::operator::Permit::Bound, unit::self_id()),
    });
    Ok(Progress::Done)
}
