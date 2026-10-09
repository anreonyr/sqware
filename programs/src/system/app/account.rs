use crate::service::account::Configuration;
use ::schedule::{Progress, Res, ResMut};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
pub(crate) struct Account {
    pub config: Arc<Configuration>,
    pub task: Option<env::TaskId>,
}
pub(crate) fn install(
    resources: &mut ::schedule::Resources<'static>,
    catalog: crate::boot::Catalog<'static>,
) -> Result<(), &'static str> {
    resources
        .insert(Account {
            config: Arc::new(Configuration {
                name: crate::service::account::NAME,
                image: catalog
                    .find(crate::service::account::IMAGE)
                    .map(|entry| entry.elf),
                authority: env::TaskId::new(0),
                supervisor: env::unit::self_id(),
                entry: AtomicUsize::new(0),
                subject: AtomicUsize::new(0),
                login: AtomicUsize::new(0),
                terminal: AtomicUsize::new(0),
                stopping: AtomicBool::new(false),
            }),
            task: None,
        })
        .map_err(|_| "Account assembly capacity")
}
pub(crate) fn spawn(
    mut account: ResMut<Account>,
    roster: Res<crate::system::control::identity::Roster>,
    mut control: ResMut<crate::system::control::unit::Control>,
) -> Result<Progress, &'static str> {
    use alloc::boxed::Box;
    let config = Arc::get_mut(&mut account.config).ok_or("account configuration shared")?;
    config.authority = roster.authority().ok_or("account authority")?;
    let config = account.config.clone();
    let body: Box<dyn FnOnce(usize) + Send> = Box::new(move |_| {
        if let Err(why) = crate::service::account::run(config) {
            crate::debug::put(why);
            let _ = env::room::doom(env::unit::self_id());
        }
    });
    let ptr = Box::into_raw(Box::new(body));
    let task = match execution::unit::spawn(
        env::TeamId::new(0),
        execution::unit::task::trampoline as *const () as usize,
        &[ptr as usize],
        0,
    ) {
        Ok(task) => task,
        Err(_) => {
            unsafe {
                drop(Box::from_raw(ptr));
            }
            return Err("account spawn");
        }
    };
    account.task = Some(task);
    control.register_internal(task)?;
    roster.authorize(task)?;
    Ok(Progress::Done)
}
pub(crate) fn grant(
    account: Res<Account>,
    mut construction: ResMut<crate::system::control::Construction>,
) -> Result<Progress, &'static str> {
    let task = account.task.ok_or("Account task")?;
    construction
        .creators
        .try_reserve(1)
        .map_err(|_| "constructor grant capacity")?;
    construction.creators.push(task);
    resource::port::ship(
        construction.entry,
        task,
        env::Access::STORE,
        env::Policy::NONE,
    )
    .map_err(|_| "Account construction grant")?;
    let derive = ipc::session::establish::find(
        account.config.authority,
        system_api::identity::Grant::Derive.mark(),
    )
    .map_err(|_| "Account identity derivation face")?;
    resource::port::ship(derive, task, env::Access::STORE, env::Policy::NONE)
        .map_err(|_| "Account identity derivation grant")?;
    env::unit::embark(task).map_err(|_| "Account embark")?;
    Ok(Progress::Done)
}
pub(crate) fn publish(
    account: Res<Account>,
    mut mounts: ResMut<crate::system::publication::Mounts>,
) -> Result<Progress, &'static str> {
    let until = env::chrono::clock() + 5_000_000_000;
    let token = loop {
        let token = account.config.entry.load(Ordering::Acquire);
        if token != 0 {
            break token;
        }
        if env::chrono::clock() >= until {
            return Err("Account startup timeout");
        }
        execution::room::park(core::time::Duration::from_millis(1))
            .map_err(|_| "Account startup wait")?;
    };
    let task = account.task.ok_or("Account task")?;
    let entry = env::PieToken::from_bytes(&(token as u64).to_le_bytes()).ok_or("Account entry")?;
    // The service owns its entry; obtain a trusted reference through the kernel owner.
    if !matches!(resource::raw::reserve(entry), Ok((vestor, owner, mark)) if vestor == task && owner == task && mark == account_api::ENTRY)
    {
        return Err("Account entry source");
    }
    mounts.0.push(crate::system::publication::Internal {
        road: account_api::DIR.to_path_buf(),
        entry,
        access: (system_api::operator::Permit::Bound, task),
    });
    Ok(Progress::Done)
}
pub(crate) fn identity(
    account: Res<Account>,
    mut names: ResMut<crate::system::publication::Names>,
    mut tree: ResMut<crate::system::operator::management::Tree>,
) -> Result<Progress, &'static str> {
    let principal = system_api::identity::PrincipalId::new(
        account.config.authority,
        account.config.subject.load(Ordering::Acquire) as u64,
    );
    names.register(
        &mut tree,
        crate::system::publication::Registration {
            name: account.config.name.into(),
            object: system_api::control::Object::Principal(principal),
            lifetime: account.task,
        },
    )?;
    Ok(Progress::Done)
}
pub(crate) fn start() -> Result<::schedule::Plan<&'static str>, ::schedule::BuildError> {
    let mut plan = ::schedule::Schedule::sequence();
    plan.system("spawn", spawn)?;
    plan.system("grant", grant)?;
    plan.system("publish", publish)?;
    plan.system("identity", identity)?;
    plan.build()
}
pub(crate) fn consumers(
    active: Res<crate::system::control::lifecycle::Active>,
    account: Res<Account>,
) -> Result<Progress, crate::system::control::unit::verdict::Fail> {
    if let Some(task) = active.task() {
        if active.is_named(crate::service::account::LOGIN) {
            account.config.login.store(task.get(), Ordering::Release);
        }
        if active.is_named(crate::service::account::TERMINAL) {
            account.config.terminal.store(task.get(), Ordering::Release);
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn stop(account: Res<Account>) -> Result<Progress, crate::system::app::Fault> {
    account.config.stopping.store(true, Ordering::Release);
    if let Some(task) = account.task {
        if !env::unit::join(task, env::Wait::POLL).unwrap_or(true) {
            return Ok(Progress::Pending);
        }
    }
    Ok(Progress::Done)
}

/// The supervisor owns the consumer teams; Account cannot Join sibling teams.
pub(crate) fn refresh(
    account: Res<Account>,
    control: Res<crate::system::control::unit::Control>,
) -> Result<Progress, crate::system::app::Fault> {
    for slot in [&account.config.login, &account.config.terminal] {
        let task = slot.load(Ordering::Acquire);
        if task != 0 && !control.live(env::TaskId::new(task)) {
            let _ = slot.compare_exchange(task, 0, Ordering::AcqRel, Ordering::Acquire);
        }
    }
    Ok(Progress::Done)
}
