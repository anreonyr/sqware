use super::answer::{self, Inbox};
use crate::system::control::serve::watch::Watch;
use crate::system::control::serve::{Fail, unit::Control};
use protocol::common::schedule::{Progress, Res, ResMut};

pub fn settle(
    flow: Res<crate::system::control::serve::frame::Flow>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, Fail> {
    if flow.settling {
        answer::reject(&mut inbox);
    }
    Ok(Progress::Done)
}
pub fn build(
    mut control: ResMut<Control>,
    mut pending: ResMut<super::super::launch::Pending>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, Fail> {
    for incoming in inbox.requests.drain(..) {
        let result = (|| {
            let bytes = crate::system::loader::serve::build::snapshot(
                crate::system::loader::serve::build::Source {
                    from: incoming.from,
                    ask: &incoming.ask,
                },
            )?;
            let count = incoming.ask.count as usize;
            if count > protocol::system::loader::frame::MAX_ARGS {
                return Err(protocol::system::control::Fail::Bad);
            }
            let mut args = [0; protocol::system::loader::frame::MAX_ARGS];
            for (to, from) in args.iter_mut().zip(&incoming.ask.args[..count]) {
                *to = *from as usize;
            }
            super::super::launch::construct(
                &mut control,
                &mut pending,
                super::super::launch::Build {
                    image: crate::system::loader::Image {
                        bytes: &bytes,
                        kind: env::ProgramKind::User,
                    },
                    spawn: crate::system::loader::serve::build::Spawn {
                        args: &args[..count],
                        stack: incoming.ask.stack as usize,
                    },
                    delivery: super::super::launch::Delivery {
                        owner: incoming.from,
                        identity: protocol::system::identity::Install::Inherit {
                            parent: incoming.from,
                        },
                        back: incoming.ask.back,
                    },
                },
            )?;
            Ok::<_, protocol::system::control::Fail>(())
        })();
        answer::release_image(&incoming.ask, incoming.from);
        if let Err(fail) = result {
            super::super::launch::reply(incoming.ask.back, Err(fail));
        }
    }
    Ok(Progress::Done)
}

pub fn close(
    mut inbox: ResMut<Inbox>,
    mut control: ResMut<Control>,
    watch: Res<Watch>,
) -> Result<Progress, Fail> {
    if let Some(entry) = inbox.entry {
        let _ = watch.pile.detach(entry, env::HoleDir::Pull);
        env::pie::seal(entry).map_err(|_| Fail::Shutdown)?;
        env::pie::release(entry).map_err(|_| Fail::Shutdown)?;
        inbox.entry = None;
    }
    answer::reject(&mut inbox);
    control.loader.clear();
    Ok(Progress::Done)
}
