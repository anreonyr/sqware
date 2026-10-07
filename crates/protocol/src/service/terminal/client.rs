use super::frame::{self, Command, Input, Reply};
use crate::common::path::Path;
use crate::system::operator::Face;
use crate::wire::message::Message;
use env::wire::Span as _;
use env::{HoleDir, Permission, PieToken, TaskId, Wait, pie};
use ::resource::{
    raw::{Hole, reserve},
    pile::Pile,
};

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
                let _ = pie::release(entry);
                return Err(());
            }
        };
        Ok(Self { entry, host })
    }
    fn call(&self, mut command: Command) -> Result<Reply, ()> {
        let back = pie::unseal_hole(frame::BACK).map_err(|_| ())?;
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
            let (n, from) = Hole::from_raw(back)
                .pull(&mut bytes, MS)
                .map_err(|_| ())?;
            let (reply, end) = Reply::fetch_at(&bytes[..n], 0).ok_or(())?;
            if from == self.host && n == end && reply.status == 0 {
                Ok(reply)
            } else {
                Err(())
            }
        })();
        let _ = pie::release(back);
        result
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = pie::release(self.entry);
    }
}

/// An exclusive control capability; the service owns and revokes all data grants.
pub struct Connection {
    terminal: Terminal,
    root: PieToken,
    authority: PieToken,
}
impl Connection {
    pub fn open(terminal: Terminal) -> Result<Self, ()> {
        let root = env::tole::unseal(false).map_err(|_| ())?;
        let mut connection = Self {
            terminal,
            root,
            authority: PieToken::NONE,
        };
        let anchor = pie::accord(
            root,
            connection.terminal.host,
            Permission::FETCH | Permission::VEST | Permission::ONLY,
            frame::AUTHORITY,
        )
        .map_err(|_| ())?;
        connection.authority = connection
            .terminal
            .call(Command {
                op: frame::ATTACH,
                task: env::unit::self_id(),
                authority: anchor,
                back: PieToken::NONE,
            })?
            .authority;
        if connection.authority == PieToken::NONE {
            return Err(());
        }
        Ok(connection)
    }
    pub fn host(&self) -> TaskId {
        self.terminal.host
    }
    pub fn io(&self) -> Result<Io, ()> {
        Io::injected(self.host())
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
        result.map(|_| ())
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
        self.command(if enabled { frame::ECHO_ON } else { frame::ECHO_OFF }, env::unit::self_id())
    }
    pub fn close(&self) -> Result<(), ()> {
        self.command(frame::DETACH, env::unit::self_id())
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = pie::release(self.root);
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
        io.pile.attach(input, HoleDir::Pull).map_err(|_| ())?;
        io.pile.attach(control, HoleDir::Pull).map_err(|_| ())?;
        Ok(io)
    }
    pub fn injected(owner: TaskId) -> Result<Self, ()> {
        let find = |mark| ipc::session::establish::find(owner, mark).ok_or(());
        Self::of(
            find(frame::INPUT)?,
            find(frame::OUTPUT)?,
            find(frame::CONTROL)?,
        )
    }
    pub fn read(&self) -> Result<Read, ()> {
        let mut bytes = [0; Input::LEN];
        loop {
            match Hole::from_raw(self.control).pull(&mut bytes, Wait::POLL) {
                Ok((1, _)) if bytes[0] == frame::INTERRUPT => return Ok(Read::Interrupt),
                Err(error) if error.source.is_busy() => {}
                _ => return Err(()),
            }
            match Hole::from_raw(self.input).pull(&mut bytes, Wait::POLL) {
                Ok((n, _)) => {
                    let input = Input::fetch(&bytes[..n]).ok_or(())?;
                    return Ok(if input.kind == frame::EOF {
                        Read::Eof
                    } else {
                        Read::Data(input)
                    });
                }
                Err(error) if error.source.is_busy() => {}
                Err(_) => return Err(()),
            }
            self.pile.await_(Wait::Forever).map_err(|_| ())?;
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
            .wait(HoleDir::Push, MS)
            .map_err(|_| ())?
            .then_some(())
            .ok_or(())
    }
}
impl Drop for Io {
    fn drop(&mut self) {
        let _ = pie::release(self.pile.token());
    }
}
