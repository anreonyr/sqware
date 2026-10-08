use super::{boot, bootstrap::Boot, life, policy, wait};
use crate::system::control::unit::{material::Supplies, start::Images};
use crate::system::{account, control, identity, launch, loader, publication};
use ::schedule::Resources;
use alloc::vec::Vec;
pub(crate) fn resources(boot: Boot) -> Result<Resources<'static>, &'static str> {
    let status = boot::status();
    let mut resources = Resources::new();
    let entry = publication::register(&mut resources)?;
    control::install(
        &mut resources,
        status.clone(),
        control::Configuration {
            images: Images {
                catalog: boot.catalog,
                entry,
            },
            supplies: Supplies::new(boot.machine, boot.accounts),
        },
    )?;
    identity::install(&mut resources)?;
    loader::install(&mut resources)?;
    launch::install(&mut resources)?;
    account::install(&mut resources, super::config::account(boot.catalog))?;
    macro_rules! put {
        ($value:expr) => {
            resources
                .insert($value)
                .map_err(|_| "app resource capacity")?
        };
    }
    put!(status);
    put!(boot::Faces(Vec::new()));
    put!(boot.machine);

    put!(policy::Flow {
        settling: false,
        forced: false,
        done: false
    });
    put!(policy::Activity {
        owed: 0,
        quiet: env::chrono::clock(),
        walking: false
    });
    put!(policy::Bound(env::Wait::POLL));
    put!(policy::Shutoff(None));
    put!(wait::Waiting::new().map_err(|_| "app waiting pile")?);
    put!(wait::Interests {
        tokens: Vec::new(),
        writes: Vec::new(),
        subs: Vec::new(),
        armed: false
    });
    put!(life::Deadline(0));
    Ok(resources)
}
