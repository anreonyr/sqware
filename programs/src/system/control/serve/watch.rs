use alloc::vec::Vec;
use env::{HoleDir, PieToken};
use protocol::system::control as ccall;
use ::resource::pile::{Pile, Sub};

pub struct Watch {
    pub(crate) instance: Option<PieToken>,
    pub(crate) pile: Pile,
    pub(crate) faces: [Option<PieToken>; ccall::Grant::ALL.len()],
    members: Vec<PieToken>,
    subs: Vec<Sub>,
}

impl Watch {
    pub fn new() -> Result<Watch, ()> {
        let pile = Pile::unseal(false).map_err(|_| ())?;
        Ok(Watch {
            instance: None,
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


impl Watch {
    pub fn entries(&self) -> impl Iterator<Item = PieToken> + '_ {
        self.faces.iter().flatten().copied().chain(self.instance)
    }
    pub fn apply(&mut self, tokens: &[PieToken], subs: &[Sub]) -> bool {
        let mut at = 0;
        while at < self.members.len() {
            if tokens.contains(&self.members[at]) {
                at += 1;
            } else {
                let token = self.members.swap_remove(at);
                let _ = self.pile.detach(token, HoleDir::Pull);
            }
        }
        for &token in tokens {
            if self.members.contains(&token) {
                continue;
            }
            if self.pile.attach(token, HoleDir::Pull).is_err() {
                return false;
            }
            self.members.push(token);
        }
        let mut at = 0;
        while at < self.subs.len() {
            if subs.contains(&self.subs[at]) {
                at += 1;
            } else {
                let sub = self.subs.swap_remove(at);
                let _ = self.pile.unsubscribe(sub);
            }
        }
        for &sub in subs {
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
