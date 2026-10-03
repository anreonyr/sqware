use super::unit::Control;
use crate::system::control::core::unit::Slot;
use crate::system::identity::serve::names::Names;
use alloc::vec::Vec;
use env::{HoleDir, PieToken, Wait};
use protocol::system::control as ccall;
use runtime::core::res::pile::{Pile, Sub};
use runtime::env::mail::HolePie;
pub struct Watch {
    pub(crate) pile: Pile,
    pub(crate) faces: [Option<PieToken>; ccall::Grant::ALL.len()],
    members: Vec<PieToken>,
    subs: Vec<Sub>,
}

impl Watch {
    pub fn new() -> Result<Watch, ()> {
        let pile = Pile::unseal(false).map_err(|_| ())?;
        Ok(Watch {
            pile,
            faces: [None; ccall::Grant::ALL.len()],
            members: Vec::new(),
            subs: Vec::new(),
        })
    }

    pub fn attach_face(&mut self, grant: ccall::Grant, face: PieToken) {
        self.faces[grant.index()] = Some(face);
    }

}

use protocol::common::schedule::{Progress, Res, ResMut};
pub struct Interests { pub tokens: Vec<PieToken>, pub subs: Vec<Sub>, pub armed: bool }
pub fn entries(watch: Res<Watch>, names: Res<Names>, mut wanted: ResMut<Interests>) -> Result<Progress, super::Fail> {
    wanted.tokens.clear(); wanted.subs.clear(); wanted.armed = false;
    wanted.tokens.try_reserve(watch.faces.len() + names.entries().len() + 3).map_err(|_| super::Fail::Room)?;
    wanted.tokens.extend(watch.faces.iter().flatten().copied());
    wanted.tokens.extend(names.entries());
    Ok(Progress::Done)
}
pub fn publication(images: Res<super::start::Images>, mut wanted: ResMut<Interests>) -> Result<Progress, super::Fail> {
    wanted.tokens.push(images.entry); Ok(Progress::Done)
}
pub fn activation(activation: Res<Option<crate::service::hub::bridge::Activation>>, mut wanted: ResMut<Interests>) -> Result<Progress, super::Fail> {
    if let Some(activation) = &*activation { wanted.tokens.push(activation.entry()); }
    Ok(Progress::Done)
}
pub fn tasks(control: Res<Control>, mut wanted: ResMut<Interests>) -> Result<Progress, super::Fail> {
    wanted.subs.try_reserve(control.table.living().count() + 3).map_err(|_| super::Fail::Room)?;
    wanted.subs.extend(control.table.living().filter_map(|row| match row.slot { Slot::Live { task, .. } => Some(Sub::TaskCompleted(task)), _ => None }));
    wanted.subs.push(Sub::TaskCompleted(control.task("operator").ok_or(super::Fail::Dead)?));
    wanted.subs.push(Sub::TaskCompleted(control.task("identity").ok_or(super::Fail::Dead)?));
    wanted.subs.push(Sub::Capabilities);
    Ok(Progress::Done)
}
pub fn apply(mut watch: ResMut<Watch>, mut wanted: ResMut<Interests>) -> Result<Progress, super::Fail> {
    let mut at = 0;
    while at < watch.members.len() {
        if wanted.tokens.contains(&watch.members[at]) { at += 1; }
        else { let token = watch.members.swap_remove(at); let _ = watch.pile.detach(&HolePie::from_token(token), HoleDir::Pull); }
    }
    for &token in &wanted.tokens {
        if watch.members.contains(&token) { continue; }
        if watch.pile.attach(&HolePie::from_token(token), HoleDir::Pull).is_err() { return Ok(Progress::Done); }
        watch.members.push(token);
    }
    let mut at = 0;
    while at < watch.subs.len() {
        if wanted.subs.contains(&watch.subs[at]) { at += 1; }
        else { let sub = watch.subs.swap_remove(at); let _ = watch.pile.unsubscribe(sub); }
    }
    for &sub in &wanted.subs {
        if watch.subs.contains(&sub) { continue; }
        if watch.pile.subscribe(sub).is_err() { return Ok(Progress::Done); }
        watch.subs.push(sub);
    }
    wanted.armed = true;
    Ok(Progress::Done)
}
pub fn wait(watch: Res<Watch>, wanted: Res<Interests>, bound: Res<super::frame::Bound>) -> Result<Progress, super::Fail> {
    watch.pile.await_(if wanted.armed { bound.0 } else { Wait::AtMost(10) }).map_err(|_| super::Fail::Wait)?;
    Ok(Progress::Done)
}

pub fn identity_changes(
    changed: Res<crate::system::identity::serve::revision::Changed>,
    mut wanted: ResMut<Interests>,
) -> Result<Progress, super::Fail> {
    wanted.tokens.push(changed.0.token());
    Ok(Progress::Done)
}
