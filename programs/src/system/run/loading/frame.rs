use super::answer::{self, Inbox};
use crate::system::control::serve::watch::Watch;
use crate::system::control::serve::{Fail, unit::Control};
use ::schedule::{Progress, Res, ResMut};

pub fn settle(
    flow: Res<crate::system::run::frame::Flow>,
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
        let super::answer::Incoming { ask, from, back } = incoming;
        let result = (|| {
            let bytes = match crate::system::loader::serve::build::snapshot(
                crate::system::loader::serve::build::Source { from, ask: &ask },
            ) {
                Ok(bytes) => bytes,
                Err(fail) => return Err((fail.into(), back)),
            };
            let count = ask.count as usize;
            if count > system_api::loader::MAX_ARGS {
                return Err((system_api::control::Fail::Bad, back));
            }
            let mut args = [0; system_api::loader::MAX_ARGS];
            for (to, from) in args.iter_mut().zip(&ask.args[..count]) {
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
                        stack: ask.stack as usize,
                    },
                    delivery: super::super::launch::Delivery {
                        owner: from,
                        identity: system_api::identity::Install::Inherit { parent: from },
                        back,
                    },
                },
            )
            .map(|_| ())
        })();
        answer::release_image(&ask, from);
        if let Err((fail, back)) = result {
            super::super::launch::reply(back, Err(fail));
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
    control.clear_images();
    Ok(Progress::Done)
}
