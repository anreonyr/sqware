use super::supplies::Supplies;
use super::{boot, bootstrap::Boot, life, policy, wait};
use crate::system::{assembly, control, identity, loader, operator};
use ::schedule::Resources;
use alloc::vec::Vec;

fn namespace_prefix(
    _: crate::system::assembly::namespace::Coordinates,
) -> Option<system_api::operator::PathBuf> {
    Some(system_api::operator::PathBuf::root())
}

const NAMESPACE_ROUTES: &[crate::system::assembly::namespace::Route] = &[
    crate::system::assembly::namespace::Route {
        program: "router",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Drivers,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "rtc",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Drivers,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "uart",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Drivers,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "account",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "hub",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "hub",
        namespace: 1,
        base: crate::system::assembly::namespace::Base::Root,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "pipe",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "terminal",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-lease",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-operator-land",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-bound",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-rack-mount",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-rule",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-watch",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-watch",
        namespace: 1,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-watch",
        namespace: 2,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
    crate::system::assembly::namespace::Route {
        program: "probe-watch-after",
        namespace: 0,
        base: crate::system::assembly::namespace::Base::Services,
        prefix: namespace_prefix,
    },
];
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
    let namespaces =
        crate::system::assembly::namespace::NamespaceInstallations::new(NAMESPACE_ROUTES).ok_or(
            crate::system::app::InstallError::Configuration("invalid namespace routes"),
        )?;
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
