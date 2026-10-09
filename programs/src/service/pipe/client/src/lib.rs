#![no_std]
use core::sync::atomic::AtomicU8;
use env::{PieToken, Wait};
use pipe_api::{Call, Endpoint, Fail, Reply, Request};
pub use pipe_api::{Direction, Read, Write};
use resource::dock::Dock;

pub struct Client {
    sender: ipc::rpc::request::Sender<Call>,
}
pub struct Pipe {
    pub id: u64,
    pub capacity: usize,
    lease: PieToken,
}
impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = env::pie::release(self.lease, env::ReleaseMode::Revoke);
    }
}
impl Client {
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        Ok(Self {
            sender: ipc::rpc::request::Sender::from_raw(entry, Call::BACK)
                .map_err(|_| Fail::Denied)?,
        })
    }
    pub fn find(tree: &system_client::operator::Face, within: Wait) -> Result<Self, Fail> {
        let entry = tree
            .tile(system_api::operator::Path::new(pipe_api::DIR), within)
            .and_then(|t| t.token(within))
            .map_err(|_| Fail::Denied)?;
        Self::of(entry)
    }
    fn call(&self, mut request: Request, within: Wait) -> Result<Reply, Fail> {
        let reply = self
            .sender
            .call(ipc::time::Deadline::new(within), |back| {
                request.back = back;
                request
            })
            .map_err(|_| Fail::Dead)?;
        if reply.status != 0 {
            return Err(Fail::of(reply.status).unwrap_or(Fail::Protocol));
        }
        Ok(reply)
    }
    fn request(op: u8, id: u64) -> Request {
        Request {
            op,
            id,
            capacity: 0,
            target: env::unit::self_id(),
            direction: 0,
            lease: PieToken::NONE,
            back: PieToken::NONE,
        }
    }
    fn lease(&self) -> Result<(PieToken, PieToken), Fail> {
        let local =
            env::pie::unseal(env::UnsealArgs::hole(pipe_api::LEASE)).map_err(|_| Fail::Full)?;
        match env::pie::accord(
            local,
            self.sender.peer(),
            env::Permission::FETCH,
            pipe_api::LEASE,
        ) {
            Ok(remote) => Ok((local, remote)),
            Err(_) => {
                let _ = env::pie::release(local, env::ReleaseMode::Revoke);
                Err(Fail::Denied)
            }
        }
    }
    pub fn create(&self, capacity: usize, within: Wait) -> Result<Pipe, Fail> {
        let (local, remote) = self.lease()?;
        let mut request = Self::request(pipe_api::CREATE, 0);
        request.capacity = capacity as u64;
        request.lease = remote;
        match self.call(request, within) {
            Ok(reply) if reply.id != 0 => Ok(Pipe {
                id: reply.id,
                capacity: reply.capacity as usize,
                lease: local,
            }),
            result => {
                let _ = env::pie::release(local, env::ReleaseMode::Revoke);
                Err(result.err().unwrap_or(Fail::Protocol))
            }
        }
    }
    pub fn begin_create(&self, capacity: usize, within: Wait) -> Result<Creating, Fail> {
        let (local, remote) = self.lease()?;
        let mut request = Self::request(pipe_api::CREATE, 0);
        request.capacity = capacity as u64;
        request.lease = remote;
        match self.sender.begin(ipc::time::Deadline::new(within), |back| {
            request.back = back;
            request
        }) {
            Ok(pending) => Ok(Creating {
                pending,
                lease: Some(local),
            }),
            Err(_) => {
                let _ = env::pie::release(local, env::ReleaseMode::Revoke);
                Err(Fail::Dead)
            }
        }
    }
    pub fn begin_release(
        &self,
        pipe: &Pipe,
        within: Wait,
    ) -> Result<ipc::rpc::request::Pending<Call>, Fail> {
        let mut request = Self::request(pipe_api::RELEASE, pipe.id);
        self.sender
            .begin(ipc::time::Deadline::new(within), |back| {
                request.back = back;
                request
            })
            .map_err(|_| Fail::Dead)
    }
    pub fn begin_bind(
        &self,
        pipe: &Pipe,
        direction: Direction,
        task: env::TaskId,
        within: Wait,
    ) -> Result<ipc::rpc::request::Pending<Call>, Fail> {
        let mut request = Self::request(pipe_api::BIND, pipe.id);
        request.direction = pipe_api::code(direction);
        request.target = task;
        self.sender
            .begin(ipc::time::Deadline::new(within), |back| {
                request.back = back;
                request
            })
            .map_err(|_| Fail::Dead)
    }
    pub fn bind(
        &self,
        pipe: &Pipe,
        direction: Direction,
        task: env::TaskId,
        within: Wait,
    ) -> Result<Endpoint, Fail> {
        let mut request = Self::request(pipe_api::BIND, pipe.id);
        request.direction = pipe_api::code(direction);
        request.target = task;
        let reply = self.call(request, within)?;
        Ok(Endpoint {
            id: pipe.id,
            capacity: pipe.capacity,
            seed: reply.seed,
            life: reply.life,
            direction,
        })
    }
    pub fn close(&self, pipe: &Pipe, direction: Direction, within: Wait) -> Result<(), Fail> {
        let mut request = Self::request(pipe_api::CLOSE, pipe.id);
        request.direction = pipe_api::code(direction);
        self.call(request, within).map(|_| ())
    }
    pub fn release(&self, pipe: Pipe, within: Wait) -> Result<(), Fail> {
        self.call(Self::request(pipe_api::RELEASE, pipe.id), within)
            .map(|_| ())
    }
    pub fn import(&self, endpoint: Endpoint, within: Wait) -> Result<Port, Fail> {
        let port = Port::import(endpoint)?;
        let _ = within;
        Ok(port)
    }
}
pub struct Creating {
    pending: ipc::rpc::request::Pending<Call>,
    lease: Option<PieToken>,
}
impl Creating {
    pub fn poll(&mut self) -> Result<Option<Pipe>, Fail> {
        let Some(reply) = self.pending.poll().map_err(|_| Fail::Dead)? else {
            return Ok(None);
        };
        if reply.status != 0 {
            return Err(Fail::of(reply.status).unwrap_or(Fail::Protocol));
        }
        if reply.id == 0 || reply.capacity == 0 || reply.capacity > pipe_api::MAX_CAPACITY as u64 {
            return Err(Fail::Protocol);
        }
        Ok(Some(Pipe {
            id: reply.id,
            capacity: reply.capacity as usize,
            lease: self.lease.take().ok_or(Fail::Protocol)?,
        }))
    }
}
impl Drop for Creating {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            let _ = env::pie::release(lease, env::ReleaseMode::Revoke);
        }
    }
}

pub struct Port {
    dock: Option<Dock>,
    endpoint: Endpoint,
    closed: bool,
}
impl Port {
    pub fn import(endpoint: Endpoint) -> Result<Self, Fail> {
        if endpoint.capacity == 0 || endpoint.capacity > pipe_api::MAX_CAPACITY {
            return Err(Fail::Invalid);
        }
        let data = resource::raw::inspect(endpoint.seed).map_err(|_| Fail::Dead)?;
        let life = resource::raw::inspect(endpoint.life).map_err(|_| Fail::Dead)?;
        if !data.alive
            || data.kind != env::PieKind::Pole
            || !life.alive
            || life.kind != env::PieKind::Tole
            || life.owner != data.owner
            || !life
                .permission
                .contains(env::Permission::FETCH | env::Permission::ONLY)
        {
            return Err(Fail::Denied);
        }
        let dock = Dock::open(endpoint.seed).map_err(|_| Fail::Dead)?;
        let port = Self {
            dock: Some(dock),
            endpoint,
            closed: false,
        };
        port.ring()?;
        Ok(port)
    }
    pub fn token(&self) -> PieToken {
        self.endpoint.seed
    }
    pub fn direction(&self) -> Direction {
        self.endpoint.direction
    }
    fn ring(&self) -> Result<stream::Ring<'_>, Fail> {
        if !resource::raw::alive(self.endpoint.seed) {
            return Err(Fail::Dead);
        }
        let view = self.dock.as_ref().ok_or(Fail::Closed)?.view();
        if self.endpoint.capacity > view.size().saturating_sub(pipe_api::HEADER_SIZE)
            || view.base() % core::mem::align_of::<pipe_api::Header>() != 0
        {
            return Err(Fail::Protocol);
        }
        // SAFETY: the held Dock bounds both slices; all shared fields use atomic access.
        let (header, bytes) = unsafe {
            (
                &*(view.base() as *const pipe_api::Header),
                core::slice::from_raw_parts(
                    (view.base() + pipe_api::HEADER_SIZE) as *const AtomicU8,
                    self.endpoint.capacity,
                ),
            )
        };
        stream::Ring::attach(header, bytes, self.endpoint.capacity).map_err(Fail::from)
    }
    fn notify(&self, bit: env::Bit) -> Result<(), Fail> {
        match env::mail::ring(self.token(), bit.bits()) {
            Ok(()) => Ok(()),
            Err(e) if e.source.is_busy() => Ok(()),
            Err(_) => Err(Fail::Dead),
        }
    }
    fn hush(&self, bit: env::Bit) -> Result<(), Fail> {
        match env::mail::hush(self.token(), bit.bits()) {
            Ok(()) => Ok(()),
            Err(e) if e.source.is_busy() => Ok(()),
            Err(_) => Err(Fail::Dead),
        }
    }
    fn bit(&self) -> env::Bit {
        match self.direction() {
            Direction::Read => pipe_api::READ_BIT,
            Direction::Write => pipe_api::WRITE_BIT,
        }
    }
    pub fn wait(&self, within: Wait) -> Result<bool, Fail> {
        let deadline = ipc::time::Deadline::new(within);
        loop {
            if self.closed {
                return Err(Fail::Closed);
            }
            if self.ring()?.ready(self.direction())? {
                return Ok(true);
            }
            self.hush(self.bit())?;
            if self.ring()?.ready(self.direction())? {
                return Ok(true);
            }
            let left = deadline.remaining();
            if left == Wait::POLL {
                return Ok(false);
            }
            env::mail::wait(self.token(), env::MailCondition::Signal(self.bit()), left)
                .map_err(|_| Fail::Dead)?;
        }
    }
    pub fn read(&mut self, out: &mut [u8], within: Wait) -> Result<Read, Fail> {
        if self.direction() != Direction::Read {
            return Err(Fail::Denied);
        }
        let deadline = ipc::time::Deadline::new(within);
        loop {
            if self.closed {
                return Err(Fail::Closed);
            }
            match self.ring()?.read(out)? {
                Read::Pending => {
                    self.notify(pipe_api::DEMAND_BIT)?;
                    if !self.wait(deadline.remaining())? {
                        return Ok(Read::Pending);
                    }
                }
                progress => {
                    if matches!(progress, Read::Bytes(n) if n != 0) {
                        self.notify(pipe_api::WRITE_BIT)?;
                    }
                    return Ok(progress);
                }
            }
        }
    }
    pub fn write(&mut self, bytes: &[u8], within: Wait) -> Result<Write, Fail> {
        if self.direction() != Direction::Write {
            return Err(Fail::Denied);
        }
        let deadline = ipc::time::Deadline::new(within);
        loop {
            if self.closed {
                return Err(Fail::Closed);
            }
            match self.ring()?.write(bytes)? {
                Write::Pending => {
                    if !self.wait(deadline.remaining())? {
                        return Ok(Write::Pending);
                    }
                }
                progress => {
                    if matches!(progress, Write::Bytes(n) if n != 0) {
                        self.notify(pipe_api::READ_BIT)?;
                    }
                    return Ok(progress);
                }
            }
        }
    }
    pub fn close(&mut self) -> Result<(), Fail> {
        if self.closed {
            return Ok(());
        }
        self.ring()?.close(self.direction());
        self.closed = true;
        let _ = self.notify(match self.direction() {
            Direction::Read => pipe_api::WRITE_BIT,
            Direction::Write => pipe_api::READ_BIT,
        });
        Ok(())
    }
}
impl Drop for Port {
    fn drop(&mut self) {
        let _ = self.close();
        if let Some(dock) = self.dock.take() {
            let _ = dock.shut();
        }
        let _ = env::pie::release(self.endpoint.seed, env::ReleaseMode::Revoke);
        let _ = env::pie::release(self.endpoint.life, env::ReleaseMode::Revoke);
    }
}
