//! Control-side session admission: bounded, nonblocking, once per delivered LINK.
use crate::support::timing::BOOT_MS;
use ::resource::port::{self, Access, Policy};
use ::resource::raw::{Hole, alive, pies, reserve};
use alloc::vec::Vec;
use env::wire::Field;
use env::{MailFail, PieToken, TaskId, Wait, pie};
use ipc::hand::{SendFail, Sender};
use ipc::session::establish::{self, Held};
use system_api::operator::{LINK_MARK, Tip};

#[derive(Default)]
pub(super) struct Connections {
    requested: Vec<TaskId>,
    tickets: Vec<Ticket>,
}
struct Ticket {
    caller: TaskId,
    reply: PieToken,
    state: State,
}
enum State {
    Pending(Pending),
    Connected { _transport: Held },
    Rejected,
}
struct Pending {
    link: Held,
    deadline: ipc::time::Deadline,
    step: Step,
}
enum Step {
    Offer,
    Request,
    Handoff {
        ask: PieToken,
        delivered: Option<PieToken>,
    },
}

fn valid(caller: TaskId, reply: PieToken) -> bool {
    alive(reply)
        && matches!(reserve(reply), Ok((giver, owner, mark))
        if giver == caller && owner == caller && mark == LINK_MARK)
}
impl Connections {
    pub(super) fn request(&mut self, caller: TaskId) -> Result<(), &'static str> {
        if !self.requested.contains(&caller) {
            self.requested
                .try_reserve(1)
                .map_err(|_| "operator request capacity")?;
            self.requested.push(caller);
        }
        Ok(())
    }
    pub(super) fn maintain(&mut self, address: (TaskId, PieToken)) -> Result<(), &'static str> {
        let host = address.0;
        let mut index = 0;
        while index < self.tickets.len() {
            if !valid(self.tickets[index].caller, self.tickets[index].reply) {
                let old = self.tickets.swap_remove(index);
                if let State::Pending(pending) = old.state {
                    pending.reject(host);
                }
            } else {
                index += 1;
            }
        }
        for caller in self.requested.drain(..) {
            for candidate in pies() {
                let reply = candidate.token;
                if !valid(caller, reply) || self.tickets.iter().any(|ticket| ticket.reply == reply)
                {
                    continue;
                }
                self.tickets
                    .try_reserve(1)
                    .map_err(|_| "operator admission capacity")?;
                let state = match establish::accept(reply) {
                    Ok(link) => State::Pending(Pending {
                        link: Held(link),
                        deadline: ipc::time::Deadline::new(Wait::AtMost(BOOT_MS)),
                        step: Step::Offer,
                    }),
                    Err(_) => State::Rejected,
                };
                self.tickets.push(Ticket {
                    caller,
                    reply,
                    state,
                });
            }
        }
        for ticket in &mut self.tickets {
            ticket.state = match core::mem::replace(&mut ticket.state, State::Rejected) {
                State::Pending(pending) => pending.advance((ticket.caller, ticket.reply), address),
                terminal => terminal,
            };
        }
        Ok(())
    }
    pub(super) fn entries(&self) -> impl Iterator<Item = PieToken> + '_ {
        self.tickets
            .iter()
            .filter_map(|ticket| match &ticket.state {
                State::Pending(pending) => Some(pending.link.rx()),
                _ => None,
            })
    }
    pub(super) fn remaining(&self) -> Wait {
        self.tickets
            .iter()
            .filter_map(|ticket| match &ticket.state {
                State::Pending(pending) => Some(pending.deadline.remaining()),
                _ => None,
            })
            .fold(Wait::Forever, |left, right| match (left, right) {
                (Wait::Forever, value) | (value, Wait::Forever) => value,
                (Wait::AtMost(a), Wait::AtMost(b)) => Wait::AtMost(a.min(b)),
            })
    }
}
impl Pending {
    fn reject(self, host: TaskId) -> State {
        if let Step::Handoff {
            delivered: Some(seed),
            ..
        } = self.step
        {
            let _ = pie::revoke(host, seed);
        }
        State::Rejected
    }
    fn advance(mut self, requester: (TaskId, PieToken), address: (TaskId, PieToken)) -> State {
        let (caller, reply) = requester;
        let host = address.0;
        if self.deadline.remaining() == Wait::POLL {
            return self.reject(host);
        }
        if matches!(self.step, Step::Offer) {
            let mut bytes = [0; TaskId::WIDTH + PieToken::WIDTH];
            host.store(&mut bytes[..TaskId::WIDTH]);
            self.link.seed().store(&mut bytes[TaskId::WIDTH..]);
            match Hole::from_raw(reply).push(&bytes, Wait::POLL) {
                Ok(()) => self.step = Step::Request,
                Err(error) if error.source == MailFail::Busy => return State::Pending(self),
                Err(_) => return self.reject(host),
            }
        }
        if matches!(self.step, Step::Request) {
            let mut bytes = [0; PieToken::WIDTH];
            match Hole::from_raw(self.link.rx()).pull(&mut bytes, Wait::POLL) {
                Ok((len, from)) if len == bytes.len() && from == caller => {
                    let Some(ask) =
                        PieToken::fetch(&bytes).filter(|token| *token != PieToken::NONE)
                    else {
                        return self.reject(host);
                    };
                    self.step = Step::Handoff {
                        ask,
                        delivered: None,
                    };
                }
                Err(error) if error.source == MailFail::Busy => return State::Pending(self),
                _ => return self.reject(host),
            }
        }
        if let Step::Handoff { ask, delivered } = &mut self.step {
            if delivered.is_none() {
                match port::ship(reply, host, Access::FETCH | Access::STORE, Policy::NONE) {
                    Ok(to) => *delivered = Some(to.seed()),
                    Err(_) => return self.reject(host),
                }
            }
            let tip = Tip::Guest {
                who: caller,
                reply: delivered.unwrap(),
                ask: *ask,
            };
            match Sender::<Tip>::from_raw(address.1).send_within(tip, Wait::POLL) {
                Ok(()) => {
                    return State::Connected {
                        _transport: self.link,
                    };
                }
                Err(SendFail::Mail(MailFail::Busy)) => return State::Pending(self),
                Err(_) => return self.reject(host),
            }
        }
        State::Pending(self)
    }
}
