use ::resource::{
    pile::Pile,
    raw::{Hole, reserve},
};
use env::wire::Span as _;
use env::{MailCondition, Permission, PieToken, TaskId, Wait, pie};
use system_api::operator::Path;
use system_client::operator::Face;
use terminal_api::frame::{self, Command, Input, Reply};
use wire::message::Message;

const MS: Wait = Wait::AtMost(1000);

pub struct Terminal {
    entry: PieToken,
    host: TaskId,
}
impl Terminal {
    pub fn find(tree: &Face) -> Result<Self, ()> {
        let entry = tree
            .tile(Path::new("/svc/terminal/attach"), MS)
            .and_then(|tile| tile.token(MS))
            .map_err(|_| ())?;
        let host = match reserve(entry) {
            Ok((_, host, _)) => host,
            Err(_) => {
                let _ = pie::release(entry, env::ReleaseMode::Revoke);
                return Err(());
            }
        };
        Ok(Self { entry, host })
    }
    fn call(&self, mut command: Command) -> Result<Reply, ()> {
        let back = pie::unseal(env::UnsealArgs::hole(frame::BACK)).map_err(|_| ())?;
        let result = (|| {
            command.back =
                pie::accord(back, self.host, Permission::STORE, frame::BACK).map_err(|_| ())?;
            let mut bytes = [0; Command::LEN];
            let n = command.store_at(&mut bytes, 0).ok_or(())?;
            Hole::from_raw(self.entry)
                .push(&bytes[..n], MS)
                .map_err(|error| {
                    crate::debug::put(&alloc::format!("terminal client: command push {:?}", error));
                    ()
                })?;
            let mut bytes = [0; Reply::LEN];
            let (n, from) = Hole::from_raw(back).pull(&mut bytes, MS).map_err(|_| ())?;
            let (reply, end) = Reply::fetch_at(&bytes[..n], 0).ok_or(())?;
            if from == self.host && n == end && reply.status == 0 {
                Ok(reply)
            } else {
                Err(())
            }
        })();
        let _ = pie::release(back, env::ReleaseMode::Revoke);
        result
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = pie::release(self.entry, env::ReleaseMode::Revoke);
    }
}

/// An exclusive control capability; the service owns and revokes all data grants.
pub struct Connection {
    terminal: Terminal,
    root: PieToken,
    authority: PieToken,
    channels: core::cell::Cell<[PieToken; 3]>,
}
impl Connection {
    pub fn open(terminal: Terminal) -> Result<Self, ()> {
        let root = env::pie::unseal(env::UnsealArgs::Tole { shared: false }).map_err(|_| ())?;
        let mut connection = Self {
            terminal,
            root,
            authority: PieToken::NONE,
            channels: core::cell::Cell::new([PieToken::NONE; 3]),
        };
        let anchor = pie::accord(
            root,
            connection.terminal.host,
            Permission::FETCH | Permission::VEST | Permission::ONLY,
            frame::AUTHORITY,
        )
        .map_err(|_| ())?;
        let reply = connection
            .terminal
            .call(Command {
                op: frame::ATTACH,
                task: env::unit::self_id(),
                authority: anchor,
                back: PieToken::NONE,
            })?;
        connection.authority = reply.authority;
        connection.channels.set([reply.input, reply.output, reply.control]);
        if connection.authority == PieToken::NONE {
            return Err(());
        }
        Ok(connection)
    }
    pub fn host(&self) -> TaskId {
        self.terminal.host
    }
    pub fn io(&self) -> Result<Io, ()> {
        let [input, output, control] = self.channels.get();
        Io::of(input, output, control)
    }
    fn command(&self, op: u8, task: TaskId) -> Result<(), ()> {
        // The service returns this exclusive proof before acknowledging the command.
        let proof = pie::accord(
            self.authority,
            self.host(),
            Permission::FETCH | Permission::ONLY,
            frame::AUTHORITY,
        )
        .map_err(|_| ())?;
        let result = self.terminal.call(Command {
            op,
            task,
            authority: proof,
            back: PieToken::NONE,
        });
        if result.is_err() {
            let _ = pie::revoke(self.host(), proof);
        }
        result.map(|reply| {
            self.channels.set([reply.input, reply.output, reply.control]);
        })
    }
    /// The target remains held until control and data grants have both moved.
    pub fn lend(&mut self, task: TaskId) -> Result<Foreground<'_>, ()> {
        if task.get() == 0 || task == env::unit::self_id() {
            return Err(());
        }
        if self.command(frame::FOREGROUND, task).is_err() {
            let _ = self.command(frame::FOREGROUND, env::unit::self_id());
            return Err(());
        }
        match pie::accord(
            self.authority,
            task,
            Permission::FETCH | Permission::VEST | Permission::ONLY,
            frame::AUTHORITY,
        ) {
            Ok(token) => Ok(Foreground {
                connection: self,
                task,
                token,
                active: true,
            }),
            Err(_) => {
                let _ = self.command(frame::FOREGROUND, env::unit::self_id());
                Err(())
            }
        }
    }
    pub fn set_echo(&self, enabled: bool) -> Result<(), ()> {
        self.command(
            if enabled {
                frame::ECHO_ON
            } else {
                frame::ECHO_OFF
            },
            env::unit::self_id(),
        )
    }
    pub fn close(&self) -> Result<(), ()> {
        self.command(frame::DETACH, env::unit::self_id())
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = pie::release(self.root, env::ReleaseMode::Revoke);
    }
}

pub struct Foreground<'a> {
    connection: &'a Connection,
    task: TaskId,
    token: PieToken,
    active: bool,
}
impl Foreground<'_> {
    fn reclaim(&mut self) -> Result<(), ()> {
        // A completed task has already released the child capability.
        let _ = pie::revoke(self.task, self.token);
        self.connection
            .command(frame::FOREGROUND, env::unit::self_id())?;
        self.active = false;
        Ok(())
    }
    pub fn restore(mut self) -> Result<(), ()> {
        self.reclaim()
    }
}
impl Drop for Foreground<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.reclaim();
        }
    }
}

pub enum Read {
    Data(Input),
    Eof,
    Interrupt,
    Suspend,
}
pub struct Io {
    input: PieToken,
    output: PieToken,
    control: PieToken,
    pile: Pile,
}
impl Io {
    fn of(input: PieToken, output: PieToken, control: PieToken) -> Result<Self, ()> {
        let pile = Pile::unseal(false).map_err(|_| ())?;
        let io = Self {
            input,
            output,
            control,
            pile,
        };
        io.pile.attach(input, MailCondition::Pull).map_err(|_| ())?;
        io.pile.attach(control, MailCondition::Pull).map_err(|_| ())?;
        Ok(io)
    }
    pub fn injected(owner: TaskId) -> Result<Self, ()> {
        let find = |mark| ipc::session::establish::find(owner, mark).map_err(|_| ());
        Self::of(
            find(frame::INPUT)?,
            find(frame::OUTPUT)?,
            find(frame::CONTROL)?,
        )
    }
    /// Task-local data handles. Foreground transitions revoke previously returned handles.
    pub fn raw_channels(&self) -> [PieToken; 3] {
        [self.input, self.output, self.control]
    }
    pub fn read(&self) -> Result<Read, ()> { self.read_with(Wait::Forever)?.ok_or(()) }
    pub fn read_with(&self, within: Wait) -> Result<Option<Read>, ()> {
        let deadline = ipc::time::Deadline::new(within);
        let mut bytes = [0; Input::LEN];
        loop {
            match Hole::from_raw(self.control).pull(&mut bytes, Wait::POLL) {
                Ok((1, _)) if bytes[0] == frame::INTERRUPT => return Ok(Some(Read::Interrupt)),
                Ok((1, _)) if bytes[0] == frame::SUSPEND => return Ok(Some(Read::Suspend)),
                Err(error) if error.source.is_busy() => {}, _ => return Err(()),
            }
            match Hole::from_raw(self.input).pull(&mut bytes, Wait::POLL) {
                Ok((n, _)) => {
                    let input = Input::fetch(&bytes[..n]).ok_or(())?;
                    return Ok(Some(if input.kind == frame::EOF { Read::Eof } else { Read::Data(input) }));
                }
                Err(error) if error.source.is_busy() => {}, Err(_) => return Err(()),
            }
            let left = deadline.remaining(); if left == Wait::POLL { return Ok(None); }
            self.pile.await_(left).map_err(|_| ())?;
        }
    }
    pub fn try_write(&self, bytes: &[u8]) -> Result<bool, ()> {
        if bytes.len() > frame::MAX || bytes.is_empty() { return Err(()); }
        match Hole::from_raw(self.output).push(bytes, Wait::POLL) {
            Ok(()) => Ok(true), Err(error) if error.source.is_busy() => Ok(false), Err(_) => Err(()),
        }
    }
    pub fn write(&self, bytes: &[u8]) -> Result<(), ()> {
        for chunk in bytes.chunks(frame::MAX) {
            Hole::from_raw(self.output)
                .push(chunk, MS)
                .map_err(|_| ())?;
        }
        Ok(())
    }
    pub fn drain(&self) -> Result<(), ()> {
        Hole::from_raw(self.output)
            .wait(MailCondition::Empty, MS)
            .map_err(|_| ())?
            .then_some(())
            .ok_or(())
    }
}
impl Drop for Io {
    fn drop(&mut self) {
        let _ = pie::release(self.pile.token(), env::ReleaseMode::Revoke);
    }
}
