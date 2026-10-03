use super::unit::Control;
use crate::system::control::core::unit::Slot;
use crate::system::identity::serve::names::Names;
use alloc::vec::Vec;
use env::{HoleDir, PieToken};
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

    pub(crate) fn sync(
        &mut self,
        control: &Control,
        entry: PieToken,
        names: &Names,
        activation: &Option<crate::service::hub::bridge::Activation>,
    ) -> bool {
        let mut wanted: Vec<PieToken> = Vec::new();
        {
            let capacity = self.faces.len() + 2 + names.entries().len();
            if wanted.try_reserve(capacity).is_err() {
                return false;
            }
            wanted.extend(self.faces.iter().flatten().copied());
            wanted.push(entry);
            wanted.extend(names.entries());
        }
        if let Some(activation) = activation {
            wanted.push(activation.entry());
        }
        let mut at = 0;
        while at < self.members.len() {
            if wanted.contains(&self.members[at]) {
                at += 1;
            } else {
                let token = self.members.swap_remove(at);
                let _ = self.pile.detach(&HolePie::from_token(token), HoleDir::Pull);
            }
        }
        for token in wanted {
            if self.members.contains(&token) {
                continue;
            }
            if self
                .pile
                .attach(&HolePie::from_token(token), HoleDir::Pull)
                .is_err()
            {
                return false;
            }
            self.members.push(token);
        }
        let mut wanted: Vec<Sub> = Vec::new();
        let count = control
            .table
            .living()
            .filter(|row| matches!(row.slot, Slot::Live { .. }))
            .count();
        if wanted.try_reserve(count + 3).is_err() {
            return false;
        }
        wanted.extend(control.table.living().filter_map(|row| match row.slot {
            Slot::Live { task, .. } => Some(Sub::TaskCompleted(task)),
            Slot::None => None,
        }));
        wanted.push(Sub::TaskCompleted(control.task("operator").unwrap()));
        wanted.push(Sub::TaskCompleted(control.task("identity").unwrap()));
        wanted.push(Sub::Capabilities);
        let mut at = 0;
        while at < self.subs.len() {
            if wanted.contains(&self.subs[at]) {
                at += 1;
            } else {
                let sub = self.subs.swap_remove(at);
                let _ = self.pile.unsubscribe(sub);
            }
        }
        for sub in wanted {
            if self.subs.contains(&sub) {
                continue;
            }
            if self.pile.subscribe(sub).is_err() {
                return false;
            }
            self.subs.push(sub);
        }
        true
    }
}
