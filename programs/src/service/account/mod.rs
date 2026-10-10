//! Account selection and login-session construction through public System capabilities.
mod supplies;
use alloc::vec::Vec;
use env::{TaskId, Wait};
use system_api::identity::{Grant, Reply, Subject, Wire};
const NAME: &str = "anran";
const LOGIN: &str = "login";
const WAIT: Wait = Wait::AtMost(5000);
struct Configuration {
    image: Vec<u8>,
    supervisor: TaskId,
    commands: supplies::Supplies,
}
fn face(authority: TaskId, grant: Grant) -> Result<system_client::identity::Face, ()> {
    let entry = ipc::session::establish::find(authority, grant.mark()).map_err(|_| ())?;
    system_client::identity::Face::direct(authority, grant, entry).map_err(|_| ())
}
pub fn run() -> Result<(), &'static str> {
    let me = env::unit::self_id();
    let supervisor = env::unit::sire();
    let authority = system_client::identity::authority().ok_or("Account authority")?;
    let resolve = face(authority, Grant::Resolve).map_err(|_| "Account Resolve face")?;
    let Reply::Binding(Some(binding)) = resolve
        .call(Wire::Resolve(me), WAIT)
        .map_err(|_| "Account Resolve call")?
    else {
        return Err("Account binding unavailable");
    };
    let Reply::Principal(Some(principal)) = face(authority, Grant::Derive)
        .map_err(|_| "Account Derive face")?
        .call(Wire::Derive(binding.current.principal), WAIT)
        .map_err(|_| "Account Derive call")?
    else {
        return Err("Account principal unavailable");
    };
    if principal == binding.current.principal {
        return Err("Account principal isolation");
    }
    let subject = Subject::new(principal, &[]).map_err(|_| "Account subject")?;
    let (image, _load) = image(supervisor)?;
    let mut config = Configuration {
        image,
        supervisor,
        commands: supplies::Supplies::receive(supervisor)?,
    };
    let tree = system_client::operator::Face::of(
        ipc::session::Session::open(supervisor, system_client::operator::BERTH, WAIT)
            .map_err(|_| "Account Operator session")?,
    );
    let publisher = system_client::control::publication::Client::injected()
        .map_err(|_| "Account publication face")?;
    publisher
        .publish_identity(
            system_api::control::publication::Object::Principal(principal),
            NAME,
            WAIT,
        )
        .map_err(|_| "Account identity publication")?;
    let state_entry = tree
        .tile(
            system_api::operator::Path::new("/svc/sys/control/state"),
            WAIT,
        )
        .and_then(|tile| tile.token(WAIT))
        .map_err(|_| "Account task query entry")?;
    if !resource::raw::alive(state_entry)
        || !matches!(resource::raw::reserve(state_entry),
        Ok((_, owner, mark)) if owner == supervisor && mark == system_api::control::Grant::State.mark())
    {
        let _ = env::pie::release(state_entry, env::ReleaseMode::Revoke);
        return Err("Account task query source");
    }
    let state = system_client::control::Face::of(state_entry).map_err(|_| "Account task query")?;
    let entry =
        env::pie::unseal(env::UnsealArgs::hole(account_api::ENTRY)).map_err(|_| "Account entry")?;
    publisher
        .publish(
            system_api::control::publication::Target::Service {
                scope: system_api::control::publication::Scope(4),
                group: "".into(),
                name: "create".into(),
            },
            entry,
            system_api::operator::Permit::Bound,
            WAIT,
        )
        .map_err(|_| "Account publication")?;
    let _ready = ipc::session::establish::endpoint(supervisor, crate::unit::READY_MARK, Wait::POLL)
        .map_err(|_| "Account ready")?;
    use wire::Message;
    let mut bytes = account_api::Request::EMPTY;
    let receiver = ipc::rpc::request::Receiver::<account_api::Call>::from_raw(
        entry,
        account_api::Call::BACK,
        account_api::Call::back,
    );
    loop {
        config.commands.sweep();
        let incoming = match receiver.receive(&mut bytes, Wait::AtMost(50)) {
            Ok(incoming) => incoming,
            Err(_) => continue,
        };
        let from = incoming.from;
        let (request, exact) = incoming.request;
        let authorized = login(&tree, &state, authority, &resolve, from);
        let result = if !authorized {
            Err(system_api::control::Fail::Denied)
        } else if !exact || !system_api::operator::name::valid(&request.account) {
            Err(system_api::control::Fail::Bad)
        } else if request.account != NAME {
            Err(system_api::control::Fail::Unknown)
        } else {
            create(&mut config, &tree, (from, subject))
        };
        let said = match result {
            Ok(built) => system_api::loader::Said {
                status: wire::OK,
                task: built.task,
                team: built.team.get() as u64,
            },
            Err(fail) => system_api::loader::Said {
                status: system_api::control::frame::fail_to_code(Some(fail)),
                task: TaskId::new(0),
                team: 0,
            },
        };
        let _ = incoming.reply.send(said);
    }
}
fn login(
    tree: &system_client::operator::Face,
    state: &system_client::control::Face,
    authority: TaskId,
    resolve: &system_client::identity::Face,
    from: TaskId,
) -> bool {
    if !matches!(state.task(LOGIN.into(), WAIT), Ok(task) if task == from) {
        return false;
    }
    let Ok(Reply::Binding(Some(binding))) = resolve.call(Wire::Resolve(from), WAIT) else {
        return false;
    };
    let object = system_api::control::publication::Object::Principal(
        system_api::identity::PrincipalId::root(authority),
    );
    let Some(road) = object.road(LOGIN) else {
        return false;
    };
    let Ok(entry) = tree.tile(&road, WAIT).and_then(|tile| tile.token(WAIT)) else {
        return false;
    };
    let result = system_client::control::publication::Client::reference_direct(
        env::unit::sire(),
        authority,
        entry,
        1,
        LOGIN,
        WAIT,
    );
    let _ = env::pie::release(entry, env::ReleaseMode::Revoke);
    matches!(result, Ok(system_api::control::publication::Object::Principal(principal)) if principal == binding.current.principal)
}
fn image(supervisor: TaskId) -> Result<(Vec<u8>, ipc::session::establish::Held), &'static str> {
    use wire::Message;
    let load = ipc::session::establish::Held(
        ipc::session::establish::endpoint(supervisor, env::Mark::of("account-image"), Wait::POLL)
            .map_err(|_| "Account image channel")?,
    );
    let mut bytes = [0; crate::unit::ImageSupplyFrame::LEN];
    let (length, from) = resource::raw::Hole::from_raw(load.0.rx())
        .pull(&mut bytes, WAIT)
        .map_err(|_| "Account image receive")?;
    let frame =
        crate::unit::ImageSupplyFrame::fetch(&bytes[..length]).ok_or("Account image frame")?;
    if from != supervisor
        || !resource::raw::alive(frame.seed)
        || !matches!(resource::raw::inspect(frame.seed), Ok(info) if info.alive && info.vestor == supervisor && info.owner == supervisor && info.mark == crate::unit::IMAGE_MARK)
    {
        return Err("Account image source");
    }
    struct Mapping(env::PieToken);
    impl Drop for Mapping {
        fn drop(&mut self) {
            let _ = env::pie::shut(self.0);
            let _ = env::pie::release(self.0, env::ReleaseMode::Revoke);
        }
    }
    let _mapping = Mapping(frame.seed);
    let (address, capacity) =
        resource::raw::open(frame.seed).map_err(|_| "Account image mapping")?;
    let size = usize::try_from(frame.length).map_err(|_| "Account image length")?;
    if size == 0 || size > capacity || address.checked_add(size).is_none() {
        return Err("Account image range");
    }
    let mut image = Vec::new();
    image
        .try_reserve_exact(size)
        .map_err(|_| "Account image capacity")?;
    // SAFETY: Open verified the Pole mapping and size is bounded by its capacity.
    image.extend_from_slice(unsafe { core::slice::from_raw_parts(address as *const u8, size) });
    Ok((image, load))
}
fn create(
    config: &mut Configuration,
    tree: &system_client::operator::Face,
    (owner, subject): (TaskId, Subject),
) -> Result<system_api::loader::Built, system_api::control::Fail> {
    use resource::raw::Loan;
    use system_api::control::{Fail, construction as api};
    let entry = tree
        .tile(
            system_api::operator::Path::new("svc/terminal/attach"),
            Wait::AtMost(5000),
        )
        .and_then(|tile| tile.token(Wait::AtMost(5000)))
        .map_err(|_| Fail::NotReady)?;
    let facts = resource::raw::reserve(entry);
    let live = resource::raw::alive(entry);
    let _ = env::pie::release(entry, env::ReleaseMode::Revoke);
    let host = match facts {
        Ok((_, owner, mark)) if live && owner.get() != 0 && mark == terminal_api::marks::ENTRY => {
            owner
        }
        _ => return Err(Fail::Denied),
    };
    let bytes = config.image.as_slice();
    let image = env::pie::unseal(env::UnsealArgs::Pole {
        size: bytes.len().div_ceil(env::PAGE_SIZE) * env::PAGE_SIZE,
        shared: true,
    })
    .map_err(|_| Fail::Full)?;
    struct Image(env::PieToken);
    impl Drop for Image {
        fn drop(&mut self) {
            let _ = env::pie::shut(self.0);
            let _ = env::pie::release(self.0, env::ReleaseMode::Revoke);
        }
    }
    let _image = Image(image);
    let (address, _) = resource::raw::open(image).map_err(|_| Fail::Full)?;
    // SAFETY: This task owns a writable mapping of at least bytes.len() bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
    }
    let sender = ipc::rpc::request::Sender::<api::Call>::from_raw(
        ipc::session::establish::find(config.supervisor, api::ENTRY).map_err(|_| Fail::Denied)?,
        api::Call::BACK,
    )
    .map_err(|_| Fail::NotReady)?;
    let loan = Loan::accord(
        &image,
        sender.peer(),
        env::Permission::FETCH,
        system_api::loader::IMAGE,
    )
    .map_err(|_| Fail::Denied)?;
    let said = sender
        .call(ipc::time::Deadline::new(Wait::AtMost(5000)), |back| {
            let mut args = [0; system_api::loader::MAX_ARGS];
            args[0] = host.get() as u64;
            args[1] = env::unit::self_id().get() as u64;
            api::Request {
                constructor: true,
                image: system_api::loader::Ask {
                    op: system_api::loader::BUILD,
                    image: loan.remote(),
                    offset: 0,
                    len: bytes.len() as u64,
                    stack: 0,
                    count: 2,
                    args,
                    back,
                },
                owner,
                subject,
            }
        })
        .map_err(|_| Fail::NotReady)?;
    if said.status != wire::OK {
        return Err(system_api::control::frame::code_to_fail(said.status).unwrap_or(Fail::Bad));
    }
    if said.task.get() == 0 || said.team == 0 {
        return Err(Fail::Bad);
    }
    let built = system_api::loader::Built {
        task: said.task,
        team: env::TeamId::new(said.team as usize),
    };
    if let Err(error) = config.commands.send(built.task) {
        crate::debug::put(error);
        if let Ok(entry) = tree
            .tile(system_api::control::INSTANCE, WAIT)
            .and_then(|tile| tile.token(WAIT))
        {
            if let Ok(lifecycle) = system_client::control::Face::of(entry) {
                let _ = lifecycle.instance(built.task).ruin(WAIT);
            }
        }
        return Err(Fail::Full);
    }
    Ok(built)
}
