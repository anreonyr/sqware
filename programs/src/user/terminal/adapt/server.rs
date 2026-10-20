//! Private task endpoints and foreground ownership; one attachment at a time.
use super::E_TERMINAL;
use crate::core::mode::Mode;
use ::resource::{
    pile::Pile,
    raw::{Hole, inspect, reserve},
};
use ::schedule::{Progress, Res, ResMut};
use alloc::collections::VecDeque;
use env::wire::Span as _;
use env::{HoleCondition, Permission, PieToken, TaskId, ToleAwaitReply, ToleEventSource, Wait, pie};
use programs::driver::uart::client::Console;
use system_api::operator::{self as operator_api, Permit};
use terminal_api::frame::{self, Command, Input, Reply};
use wire::Message;

pub(super) struct Endpoints {
    pub input: PieToken,
    pub output: PieToken,
    pub control: PieToken,
}
pub(super) struct Attachment {
    pub foreground: TaskId,
    pub endpoints: Endpoints,
    pub authority: PieToken,
    grants: [PieToken; 3],
}
impl Attachment {
    fn open(authority: PieToken, owner: TaskId) -> Result<Self, ()> {
        let mut attachment = Self {
            foreground: owner,
            authority,
            endpoints: Endpoints {
                input: PieToken::NONE,
                output: PieToken::NONE,
                control: PieToken::NONE,
            },
            grants: [PieToken::NONE; 3],
        };
        attachment.endpoints.input =
            env::hole::unseal(frame::INPUT, env::HoleLimits::default()).map_err(|_| ())?;
        attachment.endpoints.output =
            env::hole::unseal(frame::OUTPUT, env::HoleLimits::default()).map_err(|_| ())?;
        attachment.endpoints.control =
            env::hole::unseal(frame::CONTROL, env::HoleLimits::default()).map_err(|_| ())?;
        attachment.grant(owner)?;
        Ok(attachment)
    }
    fn revoke(&mut self) -> Result<(), ()> {
        for token in &mut self.grants {
            if *token == PieToken::NONE {
                continue;
            }
            match pie::revoke(self.foreground, *token) {
                Ok(()) => {}
                // No transfer rights were given, so a missing direct child is already gone.
                Err(error) if error.source == env::PieFail::Denied => {}
                Err(_) => return Err(()),
            }
            *token = PieToken::NONE;
        }
        Ok(())
    }
    fn grant(&mut self, task: TaskId) -> Result<(), ()> {
        self.foreground = task;
        for (index, (token, rights, mark)) in [
            (self.endpoints.input, Permission::FETCH, frame::INPUT),
            (self.endpoints.output, Permission::STORE, frame::OUTPUT),
            (self.endpoints.control, Permission::FETCH, frame::CONTROL),
        ]
        .into_iter()
        .enumerate()
        {
            match pie::accord(token, task, rights, mark) {
                Ok(token) => self.grants[index] = token,
                Err(_) => {
                    let _ = self.revoke();
                    return Err(());
                }
            }
        }
        Ok(())
    }
}
impl Drop for Attachment {
    fn drop(&mut self) {
        for token in [
            self.endpoints.input,
            self.endpoints.output,
            self.endpoints.control,
            self.authority,
        ] {
            if token != PieToken::NONE {
                let _ = pie::release(token, env::ReleaseMode::Revoke);
            }
        }
    }
}
pub(super) struct Server {
    pub entry: PieToken,
    pub pile: Pile,
    pub attachment: Option<Attachment>,
    pub pending: VecDeque<Input>,
    pub event: Option<u8>,
    pub active: bool,
    pub running: bool,
    pub echo: bool,
    pub waiting_input: bool,
}
impl Server {
    pub fn open(console: &Console) -> Result<Self, env::Reason> {
        let entry = env::hole::unseal(frame::ENTRY, env::HoleLimits::default()).map_err(|_| E_TERMINAL)?;
        let pile = Pile::unseal(false).map_err(|_| E_TERMINAL)?;
        pile.attach(ToleEventSource::Hole {
            token: entry,
            condition: HoleCondition::Pull,
        })
        .map_err(|_| E_TERMINAL)?;
        pile.attach(console.rx.source())
        .map_err(|_| E_TERMINAL)?;
        pile.attach(console.tx.source()).map_err(|_| E_TERMINAL)?;
        let capability =
            system_client::operator::Capability::injected(0, env::unit::sire(), Wait::AtMost(1000))
                .map_err(|_| E_TERMINAL)?;
        let anchor = capability.anchor().map_err(|_| E_TERMINAL)?;
        let view = operator_api::ResourceView::new(env::Access::STORE).ok_or(E_TERMINAL)?;
        capability
            .bind(
                anchor,
                "attach",
                entry,
                view,
                Permit::Public,
                operator_api::OperationId(1),
                Wait::AtMost(1000),
            )
            .map_err(|_| E_TERMINAL)?;
        let _ = ipc::session::establish::lend(env::unit::sire(), programs::unit::READY_MARK);
        Ok(Self {
            entry,
            pile,
            attachment: None,
            pending: VecDeque::with_capacity(programs::driver::uart::core::frame::MAX),
            event: None,
            active: false,
            running: true,
            echo: true,
            waiting_input: false,
        })
    }
    pub fn reset(&mut self) {
        self.pending.clear();
        self.event = None;
        if let Some(attachment) = &self.attachment {
            let mut bytes = [0; Input::LEN];
            for token in [
                attachment.endpoints.input,
                attachment.endpoints.control,
                attachment.endpoints.output,
            ] {
                while Hole::from_raw(token).pull(&mut bytes, Wait::POLL).is_ok() {}
            }
        }
    }
    fn detach(&mut self) {
        self.echo = true;
        self.reset();
        if let Some(attachment) = self.attachment.take() {
            let _ = self.pile.detach(ToleEventSource::Inspect {
                task: env::unit::self_id(),
                token: attachment.authority,
            });
            let _ = self.pile.detach(ToleEventSource::Hole {
                token: attachment.endpoints.output,
                condition: HoleCondition::Pull,
            });
            if self.waiting_input {
                let _ = self.pile.detach(ToleEventSource::Hole {
                    token: attachment.endpoints.input,
                    condition: HoleCondition::Empty,
                });
                self.waiting_input = false;
            }
        }
    }
    fn command(&mut self, command: Command, from: TaskId) -> Result<PieToken, ()> {
        match command.op {
            frame::ATTACH if self.attachment.is_none() && command.task == from => {
                let attachment = Attachment::open(command.authority, from)?;
                self.pile
                    .attach(ToleEventSource::Hole {
                        token: attachment.endpoints.output,
                        condition: HoleCondition::Pull,
                    })
                    .map_err(|_| ())?;
                let authority = match pie::accord(
                    command.authority,
                    from,
                    Permission::FETCH | Permission::VEST | Permission::ONLY,
                    frame::AUTHORITY,
                ) {
                    Ok(token) => token,
                    Err(_) => {
                        let _ = self.pile.detach(ToleEventSource::Hole {
                            token: attachment.endpoints.output,
                            condition: HoleCondition::Pull,
                        });
                        return Err(());
                    }
                };
                if self
                    .pile
                    .attach(ToleEventSource::Inspect {
                        task: env::unit::self_id(),
                        token: attachment.authority,
                    })
                    .is_err()
                {
                    let _ = self.pile.detach(ToleEventSource::Hole {
                        token: attachment.endpoints.output,
                        condition: HoleCondition::Pull,
                    });
                    let _ = pie::revoke(from, authority);
                    return Err(());
                }
                self.echo = true;
                self.attachment = Some(attachment);
                Ok(authority)
            }
            frame::FOREGROUND if command.task.get() != 0 => {
                let attachment = self.attachment.as_mut().ok_or(())?;
                if !pie::same(attachment.authority, command.authority).map_err(|_| ())? {
                    return Err(());
                }
                attachment.revoke()?;
                self.echo = true;
                self.reset();
                self.attachment.as_mut().unwrap().grant(command.task)?;
                Ok(PieToken::NONE)
            }
            frame::ECHO_ON | frame::ECHO_OFF => {
                let attachment = self.attachment.as_ref().ok_or(())?;
                if from != attachment.foreground
                    || command.task != from
                    || !pie::same(attachment.authority, command.authority).map_err(|_| ())?
                {
                    return Err(());
                }
                self.echo = command.op == frame::ECHO_ON;
                Ok(PieToken::NONE)
            }
            frame::DETACH => {
                let attachment = self.attachment.as_ref().ok_or(())?;
                if !pie::same(attachment.authority, command.authority).map_err(|_| ())? {
                    return Err(());
                }
                self.detach();
                Ok(PieToken::NONE)
            }
            _ => Err(()),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.detach();
        let _ = pie::release(self.entry, env::ReleaseMode::Revoke);
        let _ = pie::release(self.pile.token(), env::ReleaseMode::Revoke);
    }
}

pub(super) fn requests(
    mut server: ResMut<Server>,
    mut mode: ResMut<Mode>,
    console: Res<Console>,
) -> Result<Progress, env::Reason> {
    server.active = false;
    if !ipc::session::alive(console.rx.bell()) {
        server.running = false;
        return Ok(Progress::Pending);
    }
    if server
        .attachment
        .as_ref()
        .is_some_and(|a| !ipc::session::alive(a.authority))
    {
        server.detach();
        mode.reset();
    }
    let mut bytes = [0; env::PAGE_SIZE];
    // Each turn admits at most the current bounded queue, keeping UART and output progressing.
    for _ in 0..4 {
        let Ok((n, from)) = Hole::from_raw(server.entry).pull(&mut bytes, Wait::POLL) else {
            break;
        };
        server.active = true;
        let Some((command, end)) = Command::fetch_at(&bytes[..n], 0) else {
            continue;
        };
        if end != n
            || !matches!(reserve(command.back), Ok((vestor, owner, mark))
            if vestor == from && owner == from && mark == frame::BACK)
        {
            continue;
        }
        // Inspect only identifies the loan. Await also proves that this exclusive
        // capability is currently usable, rejecting a guessed, handed-over ancestor.
        let loaned = matches!(inspect(command.authority), Ok(info)
            if info.alive && info.vestor == from && info.mark == frame::AUTHORITY)
            && env::tole::await_(command.authority, Wait::POLL).is_ok();
        let result = if loaned {
            server.command(command, from)
        } else {
            Err(())
        };
        if result.is_ok() {
            if matches!(command.op, frame::ECHO_ON | frame::ECHO_OFF) {
                mode.clear();
            } else {
                mode.reset();
            }
        }
        if loaned && (command.op != frame::ATTACH || result.is_err()) {
            let _ = pie::release(command.authority, env::ReleaseMode::Revoke);
        }
        let grants = if result.is_ok() {
            server
                .attachment
                .as_ref()
                .filter(|a| a.foreground == from)
                .map(|a| a.grants)
                .unwrap_or([PieToken::NONE; 3])
        } else {
            [PieToken::NONE; 3]
        };
        let reply = Reply {
            status: if result.is_ok() { 0 } else { 1 },
            authority: result.unwrap_or(PieToken::NONE),
            input: grants[0],
            output: grants[1],
            control: grants[2],
        };
        let mut ack = [0; Reply::LEN];
        if let Some(n) = reply.store_at(&mut ack, 0) {
            let _ = Hole::from_raw(command.back).push(&ack[..n], Wait::POLL);
        }
        let _ = pie::release(command.back, env::ReleaseMode::Revoke);
    }
    Ok(Progress::Done)
}

pub(super) fn deliver(mut server: ResMut<Server>) -> Result<Progress, env::Reason> {
    let Some((input, control)) = server
        .attachment
        .as_ref()
        .map(|a| (a.endpoints.input, a.endpoints.control))
    else {
        return Ok(Progress::Done);
    };
    if let Some(event) = server.event {
        match Hole::from_raw(control).push(&[event], Wait::POLL) {
            Ok(()) => {
                server.event = None;
                server.active = true;
            }
            Err(e) if e.source.is_busy() => {}
            Err(_) => {
                server.detach();
                return Ok(Progress::Done);
            }
        }
    }
    let mut bytes = [0; Input::LEN];
    while let Some(event) = server.pending.front() {
        let n = event.store(&mut bytes).ok_or(E_TERMINAL)?;
        match Hole::from_raw(input).push(&bytes[..n], Wait::POLL) {
            Ok(()) => {
                server.pending.pop_front();
                server.active = true;
            }
            Err(e) if e.source.is_busy() => break,
            Err(_) => {
                server.detach();
                return Ok(Progress::Done);
            }
        }
    }
    let blocked = !server.pending.is_empty();
    if blocked && !server.waiting_input {
        server
            .pile
            .attach(ToleEventSource::Hole {
                token: input,
                condition: HoleCondition::Empty,
            })
            .map_err(|_| E_TERMINAL)?;
    } else if !blocked && server.waiting_input {
        server
            .pile
            .detach(ToleEventSource::Hole {
                token: input,
                condition: HoleCondition::Empty,
            })
            .map_err(|_| E_TERMINAL)?;
    }
    server.waiting_input = blocked;
    Ok(Progress::Done)
}

pub(super) fn wait(mut server: ResMut<Server>) -> Result<Progress, env::Reason> {
    if !server.active {
        match server.pile.await_(Wait::Forever).map_err(|_| E_TERMINAL)? {
            ToleAwaitReply::Source {
                source: ToleEventSource::Inspect { token, .. },
                ..
            } if server
                .attachment
                .as_ref()
                .is_some_and(|a| a.authority == token) =>
            {
                server.detach()
            }
            ToleAwaitReply::Source { fail: Some(_), .. } => {
                server.running = false;
            }
            _ => {}
        }
    }
    Ok(Progress::Done)
}
