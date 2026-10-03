use crate::system::common::face::mount;
use crate::system::control::core::publication::Publications;
use crate::system::control::serve::{start::BOOT_MS, watch::Watch};
use crate::system::identity::serve::install::Roster;
use crate::system::identity::serve::names::Names;
use crate::system::life::{Phase, Status};
use crate::system::operator::serve::install::Tree;
use crate::system::{identity, operator};
use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use env::{PieToken, TaskId, Wait};
use protocol::debug;

pub fn start() -> Result<Arc<Status>, ()> {
    let status = Arc::new(Status {
        control: runtime::env::unit::self_id(),
        operator: AtomicUsize::new(0),
        identity: AtomicUsize::new(0),
        phase: AtomicU8::new(Phase::Starting as u8),
    });
    // Publish both task identities before either task is released.
    for (slot, operator) in [(&status.operator, true), (&status.identity, false)] {
        let state = status.clone();
        let body: Box<dyn FnOnce(usize) + Send> = Box::new(move |_| {
            let success = if operator {
                operator::serve::serve(state.clone()).is_ok()
            } else {
                identity::serve::serve(state.clone()).is_ok()
            };
            if !success || state.phase.load(Ordering::Acquire) != Phase::Stopping as u8 {
                debug::put("system: internal task failed; terminating team");
                let _ = runtime::env::room::doom(runtime::env::unit::self_id());
            }
        });
        let ptr = Box::into_raw(Box::new(body));
        let task = match runtime::env::unit::spawn(
            env::TeamId::new(0),
            runtime::core::task::join::trampoline as *const () as usize,
            &[ptr as usize],
            0,
        ) {
            Ok(task) => task,
            Err(_) => {
                // SAFETY: Spawn failed; no task can consume this closure.
                unsafe {
                    drop(Box::from_raw(ptr));
                }
                let _ = runtime::env::room::doom(status.control);
                return Err(());
            }
        };
        slot.store(task.get(), Ordering::Release);
    }
    let operator = env::TaskId::new(status.operator.load(Ordering::Acquire));
    let identity = env::TaskId::new(status.identity.load(Ordering::Acquire));
    if runtime::env::unit::embark(operator).is_err() || runtime::env::unit::embark(identity).is_err()
    {
        let _ = runtime::env::room::doom(status.control);
        return Err(());
    }
    Ok(status)
}

pub fn install(
    status: &Status,
    roster: &mut Roster,
    tree: &mut Tree,
    publications: &mut Publications,
    entry: PieToken,
    names: &mut Names,
    watch: &mut Watch,
) -> Result<(), &'static str> {
    use protocol::system::operator::Permit;
    use protocol::system::{identity as id, operator as op};
    let operator = TaskId::new(status.operator.load(Ordering::Acquire));
    let authority = TaskId::new(status.identity.load(Ordering::Acquire));
    tree.adopt(operator, Wait::AtMost(BOOT_MS))?;
    let faces = identity::serve::install::install(roster, operator, authority)?;
    tree.wire(
        authority,
        faces[id::Grant::Resolve.index()],
        faces[id::Grant::Matches.index()],
        faces[id::Grant::Same.index()],
    )?;
    let principal = roster.control().ok_or("Control identity missing")?;
    for grant in id::Grant::ALL {
        let permit = match grant.mount() {
            id::Mount::Public => Permit::Public,
            id::Mount::Bound => Permit::Bound,
            id::Mount::Installer => Permit::Identity(id::Selector::Exact(principal)),
        };
        let road = id::DIR.try_join(grant.name()).ok_or("Identity path")?;
        publications.internal(tree, &road, faces[grant.index()], permit, authority)?;
    }
    names.register(
        roster,
        tree,
        "control",
        protocol::system::control::publication::Object::Principal(principal),
        None,
    )?;
    for grant in op::Grant::ALL {
        let (entry, _) = mount::entry(grant.mark(), grant.name())?;
        let road = op::DIR.try_join(grant.name()).ok_or("Operator path")?;
        let permit = if matches!(grant, op::Grant::Part | op::Grant::Land | op::Grant::Trim) {
            Permit::Bound
        } else {
            Permit::Public
        };
        publications.internal(tree, &road, entry, permit, status.control)?;
    }
    publications.internal(
        tree,
        protocol::common::path::Path::new("svc/sys/control/publish"),
        entry,
        Permit::Public,
        status.control,
    )?;
    for grant in protocol::system::control::Grant::ALL {
        let permit = if grant == protocol::system::control::Grant::State {
            Permit::Public
        } else {
            Permit::Identity(id::Selector::Exact(principal))
        };
        let (entry, _) = mount::entry(grant.mark(), grant.name())?;
        let road = protocol::system::control::DIR
            .try_join(grant.name())
            .ok_or("Control path")?;
        publications.internal(tree, &road, entry, permit, status.control)?;
        watch.attach_face(grant, entry);
    }
    status.phase.store(Phase::Running as u8, Ordering::Release);
    debug::put("system: Control, Operator and Identity ready in one team");
    Ok(())
}
