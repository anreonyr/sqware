use ::schedule::{Progress, ResMut};
use crate::system::{
    control::serve::unit::Control,
    loader::{Image, serve::build::Spawn},
};
use alloc::vec::Vec;
use env::wire::Span as _;
use env::{PieToken, TaskId, Wait, pie};
use protocol::{
    system::{
        control::Fail,
        identity::Install,
        loader::{Built, frame::Said},
    },
};
#[derive(Default)]
pub struct Pending(pub Vec<Launch>);
pub struct Launch {
    pub task: TaskId,
    pub identity: Install,
    back: PieToken,
}
pub struct Build<'a> {
    pub image: Image<'a>,
    pub spawn: Spawn<'a>,
    pub delivery: Delivery,
}
pub struct Delivery {
    pub owner: TaskId,
    pub identity: Install,
    pub back: PieToken,
}
pub fn construct(
    control: &mut Control,
    pending: &mut Pending,
    build: Build<'_>,
) -> Result<Built, Fail> {
    pending.0.try_reserve(1).map_err(|_| Fail::Full)?;
    control.reserve_instance()?;
    let built = crate::system::loader::serve::build::construct(
        &mut control.loader,
        build.image,
        build.spawn,
    )?;
    control.register_instance(built, build.delivery.owner);
    pending.0.push(Launch {
        task: built.task,
        identity: build.delivery.identity,
        back: build.delivery.back,
    });
    Ok(built)
}
pub(super) fn reply(back: PieToken, result: Result<Built, Fail>) -> bool {
    let value = match result {
        Ok(built) => Said {
            status: protocol::wire::OK,
            task: built.task,
            team: built.team.get() as u64,
        },
        Err(fail) => Said {
            status: protocol::system::control::frame::fail_to_code(Some(fail)),
            task: TaskId::new(0),
            team: 0,
        },
    };
    let mut bytes = [0; Said::LEN];
    let sent = value.store_at(&mut bytes, 0).is_some_and(|n| {
        ::resource::raw::Hole::from_raw(back)
            .push(&bytes[..n], Wait::POLL)
            .is_ok()
    });
    let _ = pie::release(back);
    sent
}
pub fn completed(
    mut pending: ResMut<Pending>,
    mut control: ResMut<Control>,
) -> Result<Progress, crate::system::control::serve::Fail> {
    use crate::system::control::core::unit::State;
    let mut index = 0;
    while index < pending.0.len() {
        let launch = &pending.0[index];
        let item = control
            .instances
            .iter()
            .find(|item| item.task == launch.task);
        let result = match item {
            Some(item) if item.state == State::Debarked => Ok(Built {
                task: item.task,
                team: item.team.ok_or(crate::system::control::serve::Fail::Room)?,
            }),
            Some(item) if item.state == State::Starting => {
                index += 1;
                continue;
            }
            _ => Err(Fail::NotReady),
        };
        if !reply(launch.back, result) {
            control.stop_instance(launch.task);
        }
        pending.0.remove(index);
    }
    Ok(Progress::Done)
}
