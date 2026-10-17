use super::supplies::Supplies;
use super::{boot, bootstrap::Boot, life, policy, wait};
use crate::system::{assembly, control, identity, loader, operator};
use ::schedule::Resources;
use alloc::vec::Vec;

pub(crate) fn resources(
    boot: Boot,
) -> Result<Resources<'static>, crate::system::app::InstallError> {
    super::config::validate().map_err(crate::system::app::InstallError::Configuration)?;
    let status = boot::status();
    let deployment = crate::unit::select_deployment(|image| boot.catalog.find(image).is_some())
        .map_err(|_| {
            crate::system::app::InstallError::Configuration("deployment entry missing or ambiguous")
        })?;
    let inputs = deployment
        .start
        .iter()
        .map(|name| {
            let program = crate::unit::PROGRAMS
                .iter()
                .copied()
                .find(|program| program.name() == *name)
                .ok_or(crate::system::app::InstallError::Configuration(
                    "deployment start UnitFile missing",
                ))?;
            let image = boot
                .catalog
                .find(program.loader.image)
                .map(|entry| entry.elf)
                .ok_or(crate::system::app::InstallError::Configuration(
                    "deployment start image missing",
                ))?;
            Ok(crate::system::control::unit::start::Input {
                program,
                image: Some(image),
            })
        })
        .collect::<Result<Vec<_>, crate::system::app::InstallError>>()?;
    let mut resources = Resources::new();
    resources.insert(boot.catalog)?.insert(
        Supplies::new(boot.machine, boot.accounts, boot.catalog)
            .map_err(crate::system::app::InstallError::Configuration)?,
    )?;
    resources.insert(super::operator::Layout::default())?;
    resources.insert(super::operator::BootstrapReceipt::default())?;
    // 端点服务那一台镜像：装配期就从清单里取好，运行期不再重查。
    let service = crate::unit::PROGRAMS
        .iter()
        .copied()
        .find(|program| program.name() == "system-dma");
    let service_image = service.and_then(|program| {
        boot.catalog
            .find(program.loader.image)
            .map(|entry| (entry.elf, program.loader.space))
    });
    resources.insert(crate::system::device::dma::Dma::new(
        service_image.map(|(elf, _)| elf),
        service_image.map(|(_, space)| space),
    ))?;
    control::install(
        &mut resources,
        status.clone(),
        control::Configuration {
            inputs,
            targets: deployment.targets,
        },
    )?;
    identity::install(&mut resources)?;
    operator::install(&mut resources)?;
    resources.insert(identity::export::Exports::default())?;
    loader::install(&mut resources)?;
    assembly::install(&mut resources)?;
    let namespaces = crate::system::assembly::namespace::NamespaceInstallations::new();
    resources.insert(namespaces)?;

    resources
        .insert(status)?
        .insert(boot::Faces::default())?
        .insert(boot.machine)?
        .insert(policy::Flow {
            settling: false,
            forced: false,
            done: false,
        })?
        .insert(policy::Activity {
            owed: 0,
            quiet: env::chrono::clock(),
            walking: false,
        })?
        .insert(policy::Bound(env::Wait::POLL))?
        .insert(policy::Shutoff(None))?
        .insert(wait::Waiting::new()?)?
        .insert(wait::Interests {
            tokens: Vec::new(),
            writes: Vec::new(),
            subs: Vec::new(),
            armed: false,
        })?
        .insert(life::Deadline(0))?;
    Ok(resources)
}
