use super::supplies::Supplies;
use super::{boot, bootstrap::Boot, life, policy, wait};
use crate::system::{control, identity, launch, loader, publication};
use ::schedule::Resources;
use alloc::vec::Vec;
pub(crate) fn resources(boot: Boot) -> Result<Resources<'static>, crate::system::app::InstallError> {
    let status = boot::status();
    let mut resources = Resources::new();
    publication::register(&mut resources)?;
    resources
        .insert(super::config::namespaces(boot.machine))?
        .insert(Supplies::new(boot.machine, boot.accounts, boot.catalog))?;
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

    resources
        .insert(status)?
        .insert(boot::Faces(Vec::new()))?
        .insert(boot.machine)?
        .insert(policy::Flow {
            settling: false,
            forced: false,
            done: false
        })?
        .insert(policy::Activity {
            owed: 0,
            quiet: env::chrono::clock(),
            walking: false
        })?
        .insert(policy::Bound(env::Wait::POLL))?
        .insert(policy::Shutoff(None))?
        .insert(wait::Waiting::new()?)?
        .insert(wait::Interests {
            tokens: Vec::new(),
            writes: Vec::new(),
            subs: Vec::new(),
            armed: false
        })?
        .insert(life::Deadline(0))?;
    Ok(resources)
}
