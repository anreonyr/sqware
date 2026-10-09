//! Account service: account selection and login-session creation, through authorized Control IPC.
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use env::{TaskId, Wait};
use system_api::identity::{Grant, Reply, Subject, Wire};
pub(crate) const NAME: &str = "anran";
pub(crate) const IMAGE: &str = "cat";
pub(crate) const LOGIN: &str = "login";
pub(crate) const TERMINAL: &str = "terminal";
pub(crate) struct Configuration {
    pub name: &'static str,
    pub image: Option<&'static [u8]>,
    pub authority: TaskId,
    pub supervisor: TaskId,
    pub entry: AtomicUsize,
    pub subject: AtomicUsize,
    pub login: AtomicUsize,
    pub terminal: AtomicUsize,
    pub stopping: AtomicBool,
}
fn face(authority: TaskId, grant: Grant) -> Result<system_client::identity::Face, ()> {
    let entry = ipc::session::establish::find(authority, grant.mark()).map_err(|_| ())?;
    system_client::identity::Face::direct(authority, grant, entry).map_err(|_| ())
}
pub(crate) fn run(config: Arc<Configuration>) -> Result<(), &'static str> {
    let me = env::unit::self_id();
    let wait = Wait::AtMost(5000);
    let Reply::Binding(Some(binding)) = face(config.authority, Grant::Resolve)
        .map_err(|_| "Account Resolve face")?
        .call(Wire::Resolve(me), wait)
        .map_err(|_| "Account Resolve call")?
    else {
        return Err("Account binding unavailable");
    };
    let Reply::Principal(Some(principal)) = face(config.authority, Grant::Derive)
        .map_err(|_| "Account Derive face")?
        .call(Wire::Derive(binding.current.principal), wait)
        .map_err(|_| "Account Derive call")?
    else {
        return Err("Account principal unavailable");
    };
    let subject = Subject::new(principal, &[]).map_err(|_| "Account subject")?;
    let entry = env::pie::unseal_hole(account_api::ENTRY).map_err(|_| "Account entry")?;
    config
        .subject
        .store(principal.slot as usize, Ordering::Release);
    let remote = resource::port::ship(
        entry,
        config.supervisor,
        env::Access::FETCH_STORE,
        env::Policy::VEST,
    )
    .map_err(|_| "Account entry delivery")?
    .seed();
    config.entry.store(remote.get(), Ordering::Release);
    use wire::Message;
    let mut bytes = account_api::Request::EMPTY;
    let receiver = ipc::rpc::request::Receiver::<account_api::Call>::from_raw(
        entry,
        account_api::Call::BACK,
        account_api::Call::back,
    );
    while !config.stopping.load(Ordering::Acquire) {
        let incoming = match receiver.receive(&mut bytes, Wait::AtMost(100)) {
            Ok(incoming) => incoming,
            Err(rejected) if matches!(rejected.fail, ipc::rpc::Fail::Receive(_)) => continue,
            Err(_) => continue,
        };
        let from = incoming.from;
        let (request, exact) = incoming.request;
        let result = if from.get() != config.login.load(Ordering::Acquire) {
            Err(system_api::control::Fail::Denied)
        } else if !exact || !system_api::operator::name::valid(&request.account) {
            Err(system_api::control::Fail::Bad)
        } else if request.account != config.name {
            Err(system_api::control::Fail::Unknown)
        } else {
            create(&config, (from, subject))
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
    let _ = env::pie::seal(entry);
    let _ = env::pie::release(entry);
    Ok(())
}
fn create(
    config: &Configuration,
    (owner, subject): (TaskId, Subject),
) -> Result<system_api::loader::Built, system_api::control::Fail> {
    use resource::raw::Loan;
    use system_api::control::{Fail, construction as api};
    let host = TaskId::new(config.terminal.load(Ordering::Acquire));
    if host.get() == 0 {
        return Err(Fail::NotReady);
    }
    let bytes = config.image.ok_or(Fail::Unknown)?;
    let image = env::pie::unseal_pole(bytes.len().div_ceil(env::PAGE_SIZE) * env::PAGE_SIZE, true)
        .map_err(|_| Fail::Full)?;
    struct Image(env::PieToken);
    impl Drop for Image {
        fn drop(&mut self) {
            let _ = env::pie::shut(self.0);
            let _ = env::pie::release(self.0);
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
            api::Request {
                image: system_api::loader::Ask {
                    op: system_api::loader::BUILD,
                    image: loan.remote(),
                    offset: 0,
                    len: bytes.len() as u64,
                    stack: 0,
                    count: 1,
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
    Ok(system_api::loader::Built {
        task: said.task,
        team: env::TeamId::new(said.team as usize),
    })
}
