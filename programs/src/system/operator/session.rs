//! Operator's admitted request sessions.

use alloc::vec::Vec;
use env::{PieToken, TaskId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeskFail {
    Already,
    Conflict,
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guest {
    who: TaskId,
    reply: PieToken,
    ask: PieToken,
}

impl Guest {
    pub fn who(&self) -> TaskId {
        self.who
    }

    pub fn reply(&self) -> PieToken {
        self.reply
    }

    pub fn ask(&self) -> PieToken {
        self.ask
    }
}

pub struct Desk {
    guests: Vec<Option<Guest>>,
}

impl Desk {
    pub const fn new() -> Desk {
        Desk { guests: Vec::new() }
    }

    /// An exact replay is idempotent. Other sessions, including sessions from the same task,
    /// occupy independent entries and never replace an existing capability pair.
    pub fn admit(&mut self, who: TaskId, pair: (PieToken, PieToken)) -> Result<usize, DeskFail> {
        let (reply, ask) = pair;
        for guest in self.guests.iter().flatten() {
            if guest.who == who && guest.reply == reply && guest.ask == ask {
                return Err(DeskFail::Already);
            }
            if guest.reply == reply || guest.ask == ask {
                return Err(DeskFail::Conflict);
            }
        }
        let guest = Guest { who, reply, ask };
        if let Some(slot) = self.guests.iter().position(Option::is_none) {
            self.guests[slot] = Some(guest);
            return Ok(slot);
        }
        self.guests.try_reserve(1).map_err(|_| DeskFail::Full)?;
        self.guests.push(Some(guest));
        Ok(self.guests.len() - 1)
    }

    pub fn evict(&mut self, who: TaskId, pair: (PieToken, PieToken)) -> bool {
        let (reply, ask) = pair;
        let Some(cell) = self.guests.iter_mut().find(|cell| {
            cell.as_ref()
                .is_some_and(|guest| guest.who == who && guest.reply == reply && guest.ask == ask)
        }) else {
            return false;
        };
        *cell = None;
        true
    }

    pub fn guest(&self, ask: PieToken) -> Option<&Guest> {
        self.guests.iter().flatten().find(|guest| guest.ask == ask)
    }

    pub fn contains_reply(&self, reply: PieToken) -> bool {
        self.guests
            .iter()
            .flatten()
            .any(|guest| guest.reply == reply)
    }

    #[cfg(test)]
    pub fn occupied(&self) -> usize {
        self.guests.iter().flatten().count()
    }

    pub fn sweep_each(&mut self, mut f: impl FnMut(Gone)) -> usize {
        let mut gone = 0;
        for cell in &mut self.guests {
            if let Some(guest) = cell
                && (ipc::session::establish::vested_by(guest.reply).is_none()
                    || ipc::session::establish::vested_by(guest.ask).is_none())
            {
                f(Gone {
                    who: guest.who,
                    reply: guest.reply,
                    ask: guest.ask,
                });
                *cell = None;
                gone += 1;
            }
        }
        gone
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gone {
    pub who: TaskId,
    pub reply: PieToken,
    pub ask: PieToken,
}
