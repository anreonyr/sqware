use super::supplies::Supplies;
use super::{boot, bootstrap::Boot, life, policy, wait};
use crate::system::{control, identity, launch, loader, publication};
use ::schedule::Resources;
use alloc::vec::Vec;
pub(crate) fn resources(boot: Boot) -> Result<Resources<'static>, &'static str> {
    let status = boot::status();
    let mut resources = Resources::new();
    publication::register(&mut resources)?;
    resources
        .insert(super::config::namespaces(boot.machine))
        .map_err(|_| "publication configuration")?;
    resources
        .insert(Supplies::new(boot.machine, boot.accounts, boot.catalog))
        .map_err(|_| "assembly supplies")?;
    control::install(
        &mut resources,
        status.clone(),
        control::Configuration {
            inputs: crate::unit::PROGRAMS
                .iter()
                .copied()
                .filter(|p| p.relation.after.is_some())
                .map(|program| crate::system::control::unit::start::Input {
                    program,
                    image: boot
                        .catalog
                        .find(program.name())
                        .map(|entry| (entry.elf, entry.kind)),
                })
                .collect(),
        },
    )?;
    identity::install(&mut resources)?;
    loader::install(&mut resources)?;
    launch::install(&mut resources)?;
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
