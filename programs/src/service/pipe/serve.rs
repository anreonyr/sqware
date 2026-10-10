use super::core::Ledger;
use alloc::vec::Vec;
use env::{PieToken, TaskId, Wait};
use pipe_api::{Direction, Fail, Reply, Request};
use resource::pile::Pile;
use wire::Message;
const WAIT: Wait = Wait::AtMost(5000);
struct Grant {
    seed: PieToken,
    target: TaskId,
}
struct Root {
    id: u64,
    token: PieToken,
    address: usize,
    capacity: usize,
    owner_lease: PieToken,
    read: Option<Grant>,
    write: Option<Grant>,
}
impl Root {
    fn create(id: u64, capacity: usize, lease: PieToken) -> Result<Self, Fail> {
        let size = (pipe_api::HEADER_SIZE + capacity).div_ceil(env::PAGE_SIZE) * env::PAGE_SIZE;
        let token = env::pie::unseal(env::UnsealArgs::Pole { size, shared: true })
            .map_err(|_| Fail::Full)?;
        let (address, mapped) = match resource::raw::open(token) {
            Ok(view) => view,
            Err(_) => {
                let _ = env::pie::release(token, env::ReleaseMode::Revoke);
                return Err(Fail::Full);
            }
        };
        let root = Self {
            id,
            token,
            address,
            capacity,
            owner_lease: lease,
            read: None,
            write: None,
        };
        if mapped < pipe_api::HEADER_SIZE + capacity
            || address % core::mem::align_of::<pipe_api::Header>() != 0
        {
            return Err(Fail::Protocol);
        }
        root.header().initialize(capacity)?;
        Ok(root)
    }
    fn header(&self) -> &pipe_api::Header {
        // SAFETY: this service retains the root mapping until Root is dropped.
        unsafe { &*(self.address as *const pipe_api::Header) }
    }
    fn close(&self, direction: Direction) {
        self.header().close(direction);
        let bit = match direction {
            Direction::Read => pipe_api::WRITE_BIT,
            Direction::Write => pipe_api::READ_BIT,
        };
        let _ = env::mail::ring(self.token, bit);
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = env::pie::shut(self.token);
        let _ = env::pie::release(self.token, env::ReleaseMode::Revoke);
        let _ = env::pie::release(self.owner_lease, env::ReleaseMode::Revoke);
    }
}
fn lease(token: PieToken, actor: TaskId) -> Result<(), Fail> {
    let info = resource::raw::inspect(token).map_err(|_| Fail::Denied)?;
    if !info.alive
        || info.owner != actor
        || info.vestor != actor
        || info.kind != env::PieKind::Hole
        || info.mark != pipe_api::LEASE
    {
        return Err(Fail::Denied);
    }
    Ok(())
}
fn process(
    ledger: &mut Ledger,
    roots: &mut Vec<Root>,
    request: Request,
    from: TaskId,
    pile: &Pile,
) -> Result<Reply, Fail> {
    if request.op == pipe_api::CREATE {
        let capacity = usize::try_from(request.capacity).map_err(|_| Fail::Invalid)?;
        lease(request.lease, from)?;
        roots.try_reserve(1).map_err(|_| Fail::Full)?;
        let id = ledger.create(from.get(), capacity)?;
        match Root::create(id, capacity, request.lease) {
            Ok(root) => {
                if pile.attach(env::Source::Inspect { task: env::unit::self_id(), token: root.owner_lease }).is_err() { ledger.release(id); return Err(Fail::Full); }
                roots.push(root);
            },
            Err(error) => {
                ledger.release(id);
                return Err(error);
            }
        }
        return Ok(Reply {
            status: 0,
            id,
            capacity: capacity as u64,
            seed: PieToken::NONE,
        });
    }
    let at = roots
        .iter()
        .position(|root| root.id == request.id)
        .ok_or(Fail::Dead)?;
    ledger.authorize(request.id, from.get())?;
    match request.op {
        pipe_api::BIND => {
            let direction = pipe_api::direction(request.direction).ok_or(Fail::Invalid)?;
            let target = ledger.target(request.id, direction)?;
            if target.is_some() {
                return Err(Fail::Denied);
            }
            let root = &mut roots[at];
            if root.header().closed(direction) {
                return Err(Fail::Closed);
            }
            let old = match direction {
                Direction::Read => &root.read,
                Direction::Write => &root.write,
            };
            let seed = if let Some(grant) = old { grant.seed } else {
                let seed = env::pie::accord(root.token, request.target,
                    env::Permission::FETCH | env::Permission::STORE, pipe_api::DATA).map_err(|_| Fail::Denied)?;
                let source = env::Source::Inspect { task: request.target, token: seed };
                if pile.attach(source).is_err() {
                    let _ = env::pie::revoke(request.target, seed); return Err(Fail::Denied);
                }
                if let Err(error) = ledger.bind(request.id, from.get(), direction, request.target.get()) {
                    let _ = pile.detach(source); let _ = env::pie::revoke(request.target, seed); return Err(error);
                }
                let grant = Grant { seed, target: request.target };
                match direction { Direction::Read => root.read = Some(grant), Direction::Write => root.write = Some(grant) }
                seed
            };
            Ok(Reply {
                status: 0,
                id: root.id,
                capacity: root.capacity as u64,
                seed,
            })
        }
        pipe_api::CLOSE => {
            let direction = pipe_api::direction(request.direction).ok_or(Fail::Invalid)?;
            roots[at].close(direction);
            Ok(Reply {
                status: 0,
                id: request.id,
                capacity: 0,
                seed: PieToken::NONE,
                })
        }
        pipe_api::RELEASE => {
            roots[at].detach(pile);
            roots.swap_remove(at);
            ledger.release(request.id);
            Ok(Reply {
                status: 0,
                id: request.id,
                capacity: 0,
                seed: PieToken::NONE,
                })
        }
        _ => Err(Fail::Invalid),
    }
}
impl Root {
    fn detach(&self, pile: &Pile) {
        let _ = pile.detach(env::Source::Inspect { task: env::unit::self_id(), token: self.owner_lease });
        for grant in [&self.read, &self.write].into_iter().flatten() { let _ = pile.detach(env::Source::Inspect { task: grant.target, token: grant.seed }); }
    }
}
fn ended(ledger: &mut Ledger, roots: &mut Vec<Root>, pile: &Pile, source: env::Source) {
    let env::Source::Inspect { task, token } = source else { return };
    let mut at = 0;
    while at < roots.len() {
        let root = &mut roots[at];
        if task == env::unit::self_id() && token == root.owner_lease {
            root.close(Direction::Read); root.close(Direction::Write);
            root.detach(pile); let id = root.id; roots.swap_remove(at); ledger.release(id); continue;
        }
        for direction in [Direction::Read, Direction::Write] {
            let grant = match direction { Direction::Read => &root.read, Direction::Write => &root.write };
            if grant.as_ref().is_some_and(|g| g.target == task && g.seed == token) {
                root.close(direction); let _ = pile.detach(source);
                match direction { Direction::Read => root.read = None, Direction::Write => root.write = None }
            }
        }
        if root.header().closed(Direction::Read) && root.header().closed(Direction::Write) && root.read.is_none() && root.write.is_none() {
            root.detach(pile); let id = root.id; roots.swap_remove(at); ledger.release(id);
        } else { at += 1; }
    }
}

pub fn run() -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let _session = ipc::session::Session::open(sire, system_client::operator::BERTH, WAIT)
        .map_err(|_| "pipe operator session")?;
    let entry =
        env::pie::unseal(env::UnsealArgs::hole(pipe_api::ENTRY)).map_err(|_| "pipe entry")?;
    let publisher = system_client::control::publication::Client::injected()
        .map_err(|_| "pipe publication face")?;
    publisher
        .publish(
            system_api::control::publication::Target::Service {
                scope: system_api::control::Scope(6),
                group: "".into(),
                name: "create".into(),
            },
            entry,
            system_api::operator::Permit::Public,
            WAIT,
        )
        .map_err(|_| "pipe publication")?;
    let _ready = ipc::session::establish::endpoint(sire, crate::unit::READY_MARK, Wait::POLL)
        .map_err(|_| "pipe readiness")?;
    let receiver = ipc::rpc::request::Receiver::<pipe_api::Call>::from_raw(
        entry,
        pipe_api::Call::BACK,
        pipe_api::Call::back,
    );
    let pile = Pile::unseal(false).map_err(|_| "pipe wait group")?;
    pile.attach(env::Source::Mail { pie: entry, condition: env::MailCondition::Pull }).map_err(|_| "pipe request wait")?;
    let mut ledger = Ledger::default();
    let mut roots = Vec::new();
    let mut bytes = Request::EMPTY;
    loop {
        match pile.await_(Wait::Forever).map_err(|_| "pipe wait")? {
            env::AwaitReply::Source { source: env::Source::Inspect { task, token }, .. } => { ended(&mut ledger, &mut roots, &pile, env::Source::Inspect { task, token }); continue; }
            env::AwaitReply::Source { fail: Some(_), .. } => return Err("pipe entry lost"),
            env::AwaitReply::Pending => continue,
            _ => {},
        }
        let incoming = match receiver.receive(&mut bytes, Wait::POLL) {
            Ok(request) => request,
            Err(_) => continue,
        };
        let result = process(&mut ledger, &mut roots, incoming.request, incoming.from, &pile);
        let reply = match result {
            Ok(reply) => reply,
            Err(error) => Reply {
                status: error.code(),
                id: incoming.request.id,
                capacity: 0,
                seed: PieToken::NONE,
                },
        };
        if incoming.reply.send(reply).is_err() && incoming.request.op == pipe_api::CREATE {
            if let Some(at) = roots.iter().position(|root| root.id == reply.id) {
                roots[at].detach(&pile);
                roots.swap_remove(at);
                ledger.release(reply.id);
            }
        }
    }
}
