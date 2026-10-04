use super::{
    Fail,
    answer::{self, Inbox},
    unit::Control,
};
use crate::system::control::core::unit::State;
use env::{Wait, unit};
use protocol::{
    common::schedule::{Progress, Res, ResMut},
    system::control::{self as call},
};

pub fn answer(mut control: ResMut<Control>, mut inbox: ResMut<Inbox>) -> Result<Progress, Fail> {
    let count = inbox.0.len();
    for _ in 0..count {
        let incoming = inbox.0.pop_front().ok_or(Fail::Room)?;
        let task = match &incoming.wire {
            call::frame::Wire::EmbarkInstance(task)
            | call::frame::Wire::DebarkInstance(task)
            | call::frame::Wire::RuinInstance(task)
            | call::frame::Wire::StateInstance(task) => *task,
            _ => {
                inbox.0.push_back(incoming);
                continue;
            }
        };
        if !runtime::core::res::pie::alive(incoming.back) {
            let _ = env::pie::release(incoming.back);
            continue;
        }
        let result = (|| {
            let item = control
                .instances
                .iter_mut()
                .find(|item| item.task == task)
                .ok_or(call::Fail::Unknown)?;
            if item.owner != incoming.from {
                return Err(call::Fail::Denied);
            }
            if item.team.is_some()
                && !item.claimed
                && matches!(incoming.wire, call::frame::Wire::EmbarkInstance(_))
                && (env::chrono::clock() >= item.claim_until)
            {
                item.stop();
                return Err(call::Fail::NotReady);
            }
            match incoming.wire {
                call::frame::Wire::StateInstance(_) => {
                    return Ok(Some(call::frame::said_state(answer::wire_state(
                        item.state,
                    ))));
                }
                call::frame::Wire::EmbarkInstance(_) if item.state == State::Debarked => {
                    if unit::embark(task).is_err() {
                        item.stop();
                        return Err(call::Fail::NotReady);
                    }
                    item.claimed = true;
                    item.state = State::Ready;
                }
                call::frame::Wire::EmbarkInstance(_) if item.state == State::Ready => {}
                call::frame::Wire::DebarkInstance(_) if item.state == State::Ready => {
                    match unit::debark(task) {
                        Ok(()) => {}
                        Err(e) if e.source == env::UnitFail::Busy => return Ok(None),
                        Err(_) => return Err(call::Fail::NotReady),
                    }
                    item.state = State::Debarked;
                }
                call::frame::Wire::RuinInstance(_) => {
                    if item.team.is_some() {
                        item.stop();
                        let _ = env::room::doom(task);
                        return Ok(None);
                    }
                }
                _ => return Err(call::Fail::NotReady),
            }
            Ok(Some(call::frame::said_status(call::frame::OK)))
        })();
        match result {
            Ok(Some(said)) => answer::reply(incoming.back, said),
            Ok(None) => inbox.0.push_back(incoming),
            Err(fail) => answer::reply(
                incoming.back,
                call::frame::said_status(call::frame::fail_to_code(Some(fail))),
            ),
        }
    }
    Ok(Progress::Done)
}

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
    mut watch: ResMut<super::watch::Watch>,
    mut mounts: ResMut<crate::system::boot::Mounts>,
) -> Result<Progress, &'static str> {
    let entry = env::pie::unseal_hole(call::ASK_MARK).map_err(|_| "instance entry")?;
    watch.instance = Some(entry);
    mounts.0.push(crate::system::run::publication::Internal {
        road: call::client::INSTANCE.to_path_buf(),
        entry,
        access: (protocol::system::operator::Permit::Bound, unit::self_id()),
    });
    Ok(Progress::Done)
}
pub fn receive(
    watch: Res<super::watch::Watch>,
    mut buffer: ResMut<answer::Buffer>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, Fail> {
    let Some(entry) = watch.instance else {
        return Ok(Progress::Done);
    };
    for _ in 0..16 {
        let Ok((len, from)) =
            runtime::core::res::pie::HolePie::from_token(entry).pull(&mut buffer.0, Wait::POLL)
        else {
            break;
        };
        let Some((wire, back)) = call::frame::Wire::take(&buffer.0[..len]) else {
            continue;
        };
        if !matches!(runtime::core::res::pie::reserve(back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == call::BACK)
        {
            continue;
        }
        match wire {
            Some(
                wire @ (call::frame::Wire::EmbarkInstance(_)
                | call::frame::Wire::DebarkInstance(_)
                | call::frame::Wire::RuinInstance(_)
                | call::frame::Wire::StateInstance(_)),
            ) => {
                if inbox.0.try_reserve(1).is_err() {
                    answer::reply(back, call::frame::said_status(call::frame::FULL));
                } else {
                    inbox.0.push_back(answer::Incoming { wire, from, back });
                }
            }
            _ => answer::reply(back, call::frame::said_status(call::frame::DENIED)),
        }
    }
    Ok(Progress::Done)
}
