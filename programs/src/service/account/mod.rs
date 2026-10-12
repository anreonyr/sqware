//! Account selection and login-session construction through public System capabilities.
use alloc::{string::String, vec::Vec};
use env::{TaskId, Wait};
use system_api::identity::{Grant, Reply, Subject, Wire};
const NAME: &str = "anran";
const LOGIN: &str = "login";
const WAIT: Wait = Wait::AtMost(5000);
struct Configuration {
    supervisor: TaskId,
    next_create_id: u64,
    operations: Vec<Operation>,
    watermarks: Vec<(TaskId, u64)>,
}
#[derive(Clone)]
struct Operation {
    caller: TaskId,
    id: u64,
    account: String,
    owner: TaskId,
    subject: Subject,
    host: TaskId,
    constructor_id: system_api::control::construction::CreateId,
    cancel_requested: bool,
    terminal: bool,
}
impl Configuration {
    fn reserve(&mut self, caller: TaskId, id: u64) -> Result<(), system_api::control::Fail> {
        use system_api::control::Fail;
        if id == 0 {
            return Err(Fail::Bad);
        }
        if let Some((_, high)) = self.watermarks.iter().find(|(task, _)| *task == caller) {
            if id <= *high {
                return Err(Fail::Conflict);
            }
        } else {
            if self.watermarks.len() >= 64 {
                return Err(Fail::Full);
            }
            self.watermarks.try_reserve(1).map_err(|_| Fail::Full)?;
        }
        if self.operations.len() >= 256 {
            if let Some(at) = self.operations.iter().position(|record| record.terminal) {
                self.operations.swap_remove(at);
            } else {
                return Err(Fail::Full);
            }
        } else {
            self.operations.try_reserve(1).map_err(|_| Fail::Full)?;
        }
        if let Some((_, high)) = self.watermarks.iter_mut().find(|(task, _)| *task == caller) {
            *high = id;
        } else {
            self.watermarks.push((caller, id));
        }
        Ok(())
    }
    fn allocate(
        &mut self,
    ) -> Result<system_api::control::construction::CreateId, system_api::control::Fail> {
        let value = self.next_create_id;
        self.next_create_id = value
            .checked_add(1)
            .ok_or(system_api::control::Fail::Full)?;
        Ok(system_api::control::construction::CreateId { value })
    }
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
    let mut config = Configuration {
        supervisor,
        next_create_id: 1,
        operations: Vec::new(),
        watermarks: Vec::new(),
    };
    let tree = system_client::operator::Face::of(
        ipc::session::Session::open(supervisor, system_client::operator::BERTH, WAIT)
            .map_err(|_| "Account Operator session")?,
    );
    let namespace = system_client::operator::Capability::injected(0, supervisor, WAIT)
        .map_err(|_| "Account Operator grant")?;
    let service = tree
        .resolve(system_api::namespace::SERVICE, WAIT)
        .map_err(|_| "Account service namespace")?;
    let state_entry = tree
        .tile_from(service, system_api::operator::Path::new("control/state"), WAIT)
        .and_then(|tile| tile.token(WAIT))
        .map_err(|_| "Account task query entry")?;
    if !matches!(resource::raw::inspect(state_entry), Ok(info)
        if info.alive && info.owner == supervisor
            && info.kind == env::PieKind::Hole
            && info.mark == system_api::control::Grant::State.mark()
            && info.permission.contains(env::Permission::STORE))
    {
        let _ = env::pie::release(state_entry, env::ReleaseMode::Revoke);
        return Err("Account task query source");
    }
    let state = system_client::control::Face::of(state_entry).map_err(|_| "Account task query")?;
    let entry =
        env::pie::unseal(env::UnsealArgs::hole(account_api::ENTRY)).map_err(|_| "Account entry")?;
    let anchor = namespace.anchor().map_err(|_| "Account namespace anchor")?;
    let view = system_api::operator::ResourceView::new(env::Access::STORE)
        .ok_or("Account endpoint view")?;
    let binding = namespace
        .bind(
            anchor,
            "create",
            entry,
            view,
            system_api::operator::Permit::Bound,
            system_api::operator::OperationId(1),
            WAIT,
        )
        .map_err(|_| "Account endpoint binding")?
        .binding;
    let export = resource::raw::pies()
        .find(|info| {
            info.alive
                && info.owner == supervisor
                && info.vestor == supervisor
                && info.kind == env::PieKind::Hole
                && info.mark == system_api::identity::export::ENTRY
                && info.permission.contains(env::Permission::STORE)
        })
        .map(|info| info.token)
        .ok_or("Account identity export grant")?;
    let entry_ref = system_api::identity::OperatorEntryRef {
        operator: binding.entry.operator,
        root: matches!(
            binding.entry.target,
            system_api::operator::EntryTarget::Root
        ),
        id: match binding.entry.target {
            system_api::operator::EntryTarget::Root => 0,
            system_api::operator::EntryTarget::Node(id) => id.get() as u64,
        },
        revision: binding.revision,
    };
    use system_client::identity::{ExportBinding, IdentityExport};
    system_client::identity::Exporter::direct(supervisor, export)
        .map_err(|_| "Account identity exporter")?
        .export(
            IdentityExport {
                object: system_api::identity::IdentityObject::Principal(principal),
                alias: system_api::identity::IdentityName::new(NAME)
                    .ok_or("Account identity alias")?,
                binding: ExportBinding {
                    name: system_api::identity::IdentityName::new("create")
                        .ok_or("Account endpoint name")?,
                    entry: entry_ref,
                },
            },
            WAIT,
        )
        .map_err(|why| {
            crate::debug::put(&alloc::format!("Account identity export: {why:?}"));
            "Account identity export"
        })?;
    let _ready = ipc::session::establish::lend(supervisor, crate::unit::READY_MARK)
        .map_err(|_| "Account ready")?;
    use wire::Message;
    let mut bytes = account_api::Request::EMPTY;
    let receiver = ipc::rpc::request::Receiver::<account_api::Call>::from_raw(
        entry,
        account_api::Call::BACK,
        account_api::Call::back,
    );
    loop {
        let incoming = match receiver.receive(&mut bytes, Wait::AtMost(50)) {
            Ok(incoming) => incoming,
            Err(_) => continue,
        };
        let from = incoming.from;
        let (request, exact) = incoming.request;
        let authorized = login(&state, &resolve, authority, from);
        let result = if !authorized {
            Err(system_api::control::Fail::Denied)
        } else if !exact || !system_api::operator::name::valid(&request.account) {
            Err(system_api::control::Fail::Bad)
        } else {
            operate(
                &mut config,
                &tree,
                from,
                (from, subject),
                request.op,
                request.id,
                &request.account,
            )
        };
        let said = match result {
            Ok(said) => said,
            Err(system_api::control::Fail::NotReady) => system_api::loader::Said {
                status: wire::OK,
                state: system_api::control::construction::PENDING,
                task: TaskId::new(0),
                team: 0,
                startup: system_api::loader::StartupMetadata {
                    resolve: env::PieToken::NONE,
                },
            },
            Err(fail) => system_api::loader::Said {
                status: system_api::control::frame::fail_to_code(Some(fail)),
                state: system_api::control::construction::FAILED,
                task: TaskId::new(0),
                team: 0,
                startup: system_api::loader::StartupMetadata {
                    resolve: env::PieToken::NONE,
                },
            },
        };
        let _ = incoming.reply.send(said);
    }
}
fn login(
    state: &system_client::control::Face,
    resolve: &system_client::identity::Face,
    authority: TaskId,
    from: TaskId,
) -> bool {
    if !matches!(state.task(LOGIN.into(), WAIT), Ok(task) if task == from) {
        return false;
    }
    matches!(resolve.call(Wire::Resolve(from), WAIT), Ok(Reply::Binding(Some(binding)))
        if binding.origin.principal.authority == authority
            && binding.current.principal.authority == authority
            && binding.origin.principal.slot != 0)
}
fn create(
    config: &mut Configuration,
    tree: &system_client::operator::Face,
    (owner, subject): (TaskId, Subject),
    id: system_api::control::construction::CreateId,
    host: TaskId,
) -> Result<system_api::loader::Said, system_api::control::Fail> {
    use system_api::control::{Fail, construction as api};
    let _ = tree;
    if host.get() == 0 {
        return Err(Fail::Denied);
    }
    let sender = ipc::rpc::request::Sender::<api::Call>::from_raw(
        ipc::session::establish::find(config.supervisor, api::ENTRY).map_err(|_| Fail::Denied)?,
        api::Call::BACK,
    )
    .map_err(|_| Fail::NotReady)?;
    let said = sender
        .call(ipc::time::Deadline::new(Wait::AtMost(5000)), |back| {
            let mut args = [0; system_api::loader::MAX_ARGS];
            args[0] = host.get() as u64;
            api::Request {
                op: api::BUILD,
                id,
                input: api::InputOption(Some(api::Input::Unit(api::UnitRef {
                    name: "shell".into(),
                    stack: 0,
                    count: 1,
                    args,
                }))),
                owner,
                subject,
                back,
            }
        })
        .map_err(|_| Fail::NotReady)?;
    Ok(said)
}

fn operate(
    config: &mut Configuration,
    tree: &system_client::operator::Face,
    caller: TaskId,
    (owner, subject): (TaskId, Subject),
    op: u8,
    id: system_api::control::construction::CreateId,
    account: &str,
) -> Result<system_api::loader::Said, system_api::control::Fail> {
    use system_api::control::construction as api;
    if id.value == 0 {
        return Err(system_api::control::Fail::Bad);
    }
    let existing = config
        .operations
        .iter()
        .position(|record| record.caller == caller && record.id == id.value);
    if let Some(at) = existing {
        let record = config.operations[at].clone();
        if record.account != account || record.owner != owner || record.subject != subject {
            return Err(system_api::control::Fail::Conflict);
        }
        if op == api::CANCEL {
            config.operations[at].cancel_requested = true;
        }
        let said = if op == account_api::CREATE && !record.cancel_requested {
            let observed = forward(
                config,
                api::INSPECT,
                record.constructor_id,
                (record.owner, record.subject),
                record.host,
            )?;
            if observed.state == api::UNSEEN {
                create(
                    config,
                    tree,
                    (record.owner, record.subject),
                    record.constructor_id,
                    record.host,
                )?
            } else {
                observed
            }
        } else if record.cancel_requested {
            let observed = forward(
                config,
                api::INSPECT,
                record.constructor_id,
                (record.owner, record.subject),
                record.host,
            )?;
            if matches!(observed.state, api::CANCELLED | api::FAILED | api::CLOSED) {
                observed
            } else {
                forward(
                    config,
                    api::CANCEL,
                    record.constructor_id,
                    (record.owner, record.subject),
                    record.host,
                )?
            }
        } else {
            forward(
                config,
                op,
                record.constructor_id,
                (record.owner, record.subject),
                record.host,
            )?
        };
        if said.state == api::COMPLETED || said.state == api::FAILED || said.state == api::CANCELLED
        {
            config.operations[at].terminal = true;
        }
        return Ok(said);
    }
    if let Some((_, high)) = config.watermarks.iter().find(|(task, _)| *task == caller) {
        if id.value <= *high {
            return match op {
                api::INSPECT => Ok(system_api::loader::Said {
                    status: wire::OK,
                    state: api::CLOSED,
                    task: TaskId::new(0),
                    team: 0,
                    startup: system_api::loader::StartupMetadata {
                        resolve: env::PieToken::NONE,
                    },
                }),
                api::CANCEL => Ok(system_api::loader::Said {
                    status: wire::OK,
                    state: api::CANCELLED,
                    task: TaskId::new(0),
                    team: 0,
                    startup: system_api::loader::StartupMetadata {
                        resolve: env::PieToken::NONE,
                    },
                }),
                _ => Err(system_api::control::Fail::Conflict),
            };
        }
    }
    match op {
        api::INSPECT => {
            return Ok(system_api::loader::Said {
                status: wire::OK,
                state: api::UNSEEN,
                task: TaskId::new(0),
                team: 0,
                startup: system_api::loader::StartupMetadata {
                    resolve: env::PieToken::NONE,
                },
            });
        }
        _ => {}
    }
    if op != account_api::CREATE && op != api::CANCEL {
        return Err(system_api::control::Fail::Bad);
    }
    if op == account_api::CREATE && account != NAME {
        return Err(system_api::control::Fail::Unknown);
    }
    let host = if op == api::CANCEL {
        TaskId::new(0)
    } else {
        terminal_host(tree)?
    };
    let constructor_id = config.allocate()?;
    config.reserve(caller, id.value)?;
    let record = Operation {
        caller,
        id: id.value,
        account: account.into(),
        owner,
        subject,
        host,
        constructor_id,
        cancel_requested: op == api::CANCEL,
        terminal: false,
    };
    config.operations.push(record);
    let result = if op == account_api::CREATE {
        create(config, tree, (owner, subject), constructor_id, host)
    } else if op == api::CANCEL {
        forward(config, api::CANCEL, constructor_id, (owner, subject), host)
    } else {
        return Err(system_api::control::Fail::Bad);
    }?;
    if matches!(result.state, api::COMPLETED | api::FAILED | api::CANCELLED) {
        if let Some(record) = config
            .operations
            .iter_mut()
            .find(|record| record.caller == caller && record.id == id.value)
        {
            record.terminal = true;
        }
    }
    Ok(result)
}

fn terminal_host(
    tree: &system_client::operator::Face,
) -> Result<TaskId, system_api::control::Fail> {
    let entry = tree
        .tile_from(
            tree.resolve(system_api::namespace::SERVICE, Wait::AtMost(5000))
                .map_err(|_| system_api::control::Fail::NotReady)?,
            system_api::operator::Path::new(terminal_api::ENTRY_PATH),
            Wait::AtMost(5000),
        )
        .and_then(|tile| tile.token(Wait::AtMost(5000)))
        .map_err(|_| system_api::control::Fail::NotReady)?;
    let facts = resource::raw::reserve(entry);
    let live = resource::raw::alive(entry);
    let _ = env::pie::release(entry, env::ReleaseMode::Revoke);
    match facts {
        Ok((_, owner, mark)) if live && owner.get() != 0 && mark == terminal_api::marks::ENTRY => {
            Ok(owner)
        }
        _ => Err(system_api::control::Fail::Denied),
    }
}

fn forward(
    config: &Configuration,
    op: u8,
    id: system_api::control::construction::CreateId,
    (owner, subject): (TaskId, Subject),
    host: TaskId,
) -> Result<system_api::loader::Said, system_api::control::Fail> {
    use system_api::control::construction as api;
    let sender = ipc::rpc::request::Sender::<api::Call>::from_raw(
        ipc::session::establish::find(config.supervisor, api::ENTRY)
            .map_err(|_| system_api::control::Fail::Denied)?,
        api::Call::BACK,
    )
    .map_err(|_| system_api::control::Fail::NotReady)?;
    let _ = host;
    sender
        .call(ipc::time::Deadline::new(WAIT), |back| api::Request {
            op,
            id,
            input: api::InputOption(None),
            owner,
            subject,
            back,
        })
        .map_err(|_| system_api::control::Fail::NotReady)
}
