use crate::system::{
    control::{
        core::publication::Publications,
        serve::{publication::Internal, start::BOOT_MS},
    },
    identity,
    identity::serve::{
        install::Roster,
        names::{Names, Registration},
    },
    life::{Phase, Status},
    operator,
    operator::serve::install::{Tree, Wiring},
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use env::{PieToken, TaskId, Wait};
use protocol::{
    common::schedule::{Progress, Res, ResMut},
    debug,
    system::identity as id,
};
use runtime::core::res::bell::Bell;
pub struct Faces(pub Vec<PieToken>);
pub struct Mounts(pub Vec<Internal>);
pub fn status() -> Arc<Status> {
    Arc::new(Status {
        control: env::unit::self_id(),
        operator: AtomicUsize::new(0),
        identity: AtomicUsize::new(0),
        phase: AtomicU8::new(Phase::Starting as u8),
    })
}
pub fn spawn(
    status: Res<Arc<Status>>,
    epoch: Res<identity::serve::revision::Epoch>,
    changed: Res<identity::serve::revision::Changed>,
) -> Result<Progress, &'static str> {
    let signal = Arc::new(AtomicUsize::new(0));
    // Publish both task identities before either task is released.
    for (slot, operator) in [(&status.operator, true), (&status.identity, false)] {
        let state = (*status).clone();
        let version = (*epoch).clone();
        let bell = signal.clone();
        let body: Box<dyn FnOnce(usize) + Send> = Box::new(move |_| {
            let success = if operator {
                operator::serve::run::serve(state.clone()).is_ok()
            } else {
                identity::serve::run::serve(
                    state.clone(),
                    version,
                    identity::serve::revision::Changed(Bell::new(
                        PieToken::from_bytes(&(bell.load(Ordering::Acquire) as u64).to_le_bytes())
                            .unwrap(),
                    )),
                )
                .is_ok()
            };
            if !success || state.phase.load(Ordering::Acquire) != Phase::Stopping as u8 {
                debug::put("system: internal task failed; terminating team");
                let _ = env::room::doom(env::unit::self_id());
            }
        });
        let ptr = Box::into_raw(Box::new(body));
        let task = match runtime::core::task::spawn(
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
                let _ = env::room::doom(status.control);
                return Err("internal task spawn");
            }
        };
        slot.store(task.get(), Ordering::Release);
        if !operator {
            let seed = runtime::core::res::port::ship(changed.0.token(),
                task,
                env::Access::STORE,
                env::Policy::NONE,
            )
            .map_err(|_| "identity change signal")?
            .seed();
            signal.store(seed.get(), Ordering::Release);
        }
    }
    Ok(Progress::Done)
}
pub fn embark(status: Res<Arc<Status>>) -> Result<Progress, &'static str> {
    for slot in [&status.operator, &status.identity] {
        env::unit::embark(TaskId::new(slot.load(Ordering::Acquire)))
            .map_err(|_| "internal task embark")?;
    }
    Ok(Progress::Done)
}
pub fn adopt(status: Res<Arc<Status>>, mut tree: ResMut<Tree>) -> Result<Progress, &'static str> {
    tree.adopt(
        TaskId::new(status.operator.load(Ordering::Acquire)),
        Wait::AtMost(BOOT_MS),
    )?;
    Ok(Progress::Done)
}
pub fn identity(
    status: Res<Arc<Status>>,
    mut roster: ResMut<Roster>,
    mut faces: ResMut<Faces>,
) -> Result<Progress, &'static str> {
    faces.0 = identity::serve::install::install(
        &mut roster,
        TaskId::new(status.operator.load(Ordering::Acquire)),
        TaskId::new(status.identity.load(Ordering::Acquire)),
    )?
    .to_vec();
    Ok(Progress::Done)
}
pub fn wire(
    roster: Res<Roster>,
    faces: Res<Faces>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    tree.wire(Wiring {
        authority: identity::serve::query::current_authority(&roster)
            .ok_or("identity authority")?,
        faces: [
            faces.0[id::Grant::Resolve.index()],
            faces.0[id::Grant::Matches.index()],
            faces.0[id::Grant::Same.index()],
        ],
    })?;
    Ok(Progress::Done)
}

pub fn name(
    roster: Res<Roster>,
    mut names: ResMut<Names>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    let object = protocol::system::control::publication::Object::Principal(
        roster.control().ok_or("Control identity missing")?,
    );
    identity::serve::query::validate(&roster, object).map_err(|_| "Control identity source")?;
    names.register(
        &mut tree,
        Registration {
            name: "control".into(),
            object,
            lifetime: None,
        },
    )?;
    Ok(Progress::Done)
}

pub fn publish(
    mut mounts: ResMut<Mounts>,
    mut publications: ResMut<Publications>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    for publication in mounts.0.drain(..) {
        publications.internal(&mut tree, &publication)?;
    }
    Ok(Progress::Done)
}
pub fn await_running(status: Res<Arc<Status>>) -> Result<Progress, &'static str> {
    Ok(
        if status.phase.load(Ordering::Acquire) == Phase::Running as u8 {
            Progress::Done
        } else {
            Progress::Pending
        },
    )
}
