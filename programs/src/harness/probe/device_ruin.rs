//! 在真实设备请求未完成时，经生产 Control 退役静态恢复驱动。

mod caller;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use schedule::{Progress, Res, ResMut, Schedule};

use crate::system::app;
use crate::system::control::{self, identity::Roster, unit::Control};
use caller::Caller;

pub(crate) const REQUEST: env::Mark = env::Mark::of("device-ruin/request");

pub fn run() -> Result<(), env::Reason> {
    let boot = app::bootstrap::take().map_err(|why| why.code())?;
    let programs = app::scene::programs(&boot.catalog).map_err(|_| 9usize)?;
    let mut resources = app::install::resources(boot).map_err(|_| 9usize)?;
    resources
        .write::<control::Startup>()
        .map_err(|_| 9usize)?
        .configure(programs);
    let root = env::unit::self_id();
    let ready = Arc::new(AtomicBool::new(false));
    let start = ready.clone();
    let worker = execution::unit::task::spawn(move || {
        let until = env::chrono::clock().saturating_add(5_000_000_000);
        while !start.load(Ordering::Acquire) {
            assert!(
                env::chrono::clock() < until,
                "ruin: Control capability not delivered"
            );
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
        let ruin = system_client::control::Face::of(
            ipc::session::establish::find(root, system_api::control::Grant::Ruin.mark()).unwrap(),
        )
        .unwrap();
        crate::debug::put("ruin: requested virtio-blk");
        ruin.service("virtio-blk".into())
            .ruin(env::Wait::AtMost(5000))
            .expect("ruin: static Driver retirement");
        let state = system_client::control::Face::of(
            ipc::session::establish::find(root, system_api::control::Grant::State.mark()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            state
                .service("virtio-blk".into())
                .state(env::Wait::AtMost(1000)),
            Ok(system_api::control::State::Dead),
            "ruin: successful reply without Dead state",
        );
        crate::debug::put("ruin: virtio-blk dead after reply");
    });
    resources
        .insert(Caller {
            task: worker.id(),
            ready,
        })
        .map_err(|_| 9usize)?;
    let mut frame = Schedule::sequence();
    frame.system("ruin.caller", prepare).map_err(|_| 9usize)?;
    frame
        .plan("system", app::schedule::frame().map_err(|_| 9usize)?)
        .map_err(|_| 9usize)?;
    let result = app::supervise(resources, frame.build().map_err(|_| 9usize)?);
    worker.join();
    result
}

fn prepare(
    caller: Res<Caller>,
    endpoints: Res<control::namespace::Endpoints>,
    mut roster: ResMut<Roster>,
    mut control: ResMut<Control>,
) -> Result<Progress, app::Fault> {
    if caller.ready.load(Ordering::Acquire) {
        return Ok(Progress::Done);
    }
    let grants = [
        system_api::control::Grant::Ruin,
        system_api::control::Grant::State,
    ];
    if grants.iter().any(|grant| endpoints.face(*grant).is_none()) {
        return Ok(Progress::Done);
    }
    let Some(controller) = control.task("device-ctl-ruin") else {
        return Ok(Progress::Done);
    };
    let Ok(request) = ipc::session::establish::find(controller, REQUEST) else {
        return Ok(Progress::Done);
    };
    let mut bytes = [0u8; 1];
    let Ok((len, from)) = resource::raw::Hole::from_raw(request).pull(&mut bytes, env::Wait::POLL)
    else {
        return Ok(Progress::Done);
    };
    if (len, from, bytes) != (1, controller, [1]) {
        return Err(app::Fault::Room);
    }
    roster
        .inherit(caller.task, env::unit::self_id())
        .map_err(|_| app::Fault::Room)?;
    control
        .register_internal(caller.task)
        .map_err(|_| app::Fault::Room)?;
    for grant in grants {
        env::pie::accord(
            endpoints.face(grant).ok_or(app::Fault::Room)?,
            caller.task,
            env::Permission::STORE,
            grant.mark(),
        )
        .map_err(|_| app::Fault::Room)?;
    }
    caller.ready.store(true, Ordering::Release);
    Ok(Progress::Done)
}
