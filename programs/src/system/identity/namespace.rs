//! System retains the actual Identity face bindings and their retirement receipt.

use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::{Access, Wait};
use system_api::{
    identity::{Grant, Mount},
    operator::{PathBuf, Permit, ResourceView},
};

#[derive(Default)]
pub(crate) struct Endpoints(Option<crate::system::app::namespace::ResourcePreparation>);

pub(crate) fn prepare(
    layout: Res<crate::system::app::operator::Layout>,
    faces: Res<crate::system::app::boot::Faces>,
    mut endpoints: ResMut<Endpoints>,
) -> Result<Progress, &'static str> {
    let system = layout.as_ref().ok_or("Identity namespace layout")?;
    if endpoints.0.is_none() {
        let view = ResourceView::new(Access::STORE).ok_or("Identity endpoint view")?;
        let mut bindings = Vec::new();
        bindings
            .try_reserve_exact(Grant::ALL.len())
            .map_err(|_| "Identity endpoint capacity")?;
        for grant in Grant::ALL {
            let resource = *faces
                .roots
                .get(grant.index())
                .ok_or("Identity face missing")?;
            bindings.push(crate::system::app::namespace::BindingInput {
                name: grant.name().into(),
                mark: grant.mark(),
                resource,
                view,
                permit: match grant.mount() {
                    Mount::Public => Permit::Public,
                    Mount::Bound => Permit::Bound,
                    Mount::Installer => Permit::Identity(system_api::identity::Selector::Exact(
                        faces.installer.ok_or("Identity installer principal")?,
                    )),
                },
            });
        }
        let plan = crate::system::app::namespace::Prepare::resources(
            crate::system::app::namespace::ResourceInput {
                base: system
                    .resolve(
                        system_api::namespace::IDENTITY,
                        Wait::AtMost(crate::support::timing::BOOT_MS),
                    )
                    .map_err(|_| "Identity namespace root")?,
                path: PathBuf::try_new(system_api::identity::DIR)
                    .ok_or("Identity endpoint path")?,
                retain_prefix: 0,
                bindings,
                wait: Wait::AtMost(crate::support::timing::BOOT_MS),
            },
        )
        .ok_or("Identity endpoint plan")?;
        endpoints.0 = Some(crate::system::app::namespace::ResourcePreparation::new(
            plan,
        ));
    }
    system
        .prepare_resources(endpoints.0.as_mut().ok_or("Identity endpoint receipt")?)
        .map_err(|_| "Identity endpoint preparation")?;
    Ok(Progress::Done)
}

pub(crate) fn retire(
    layout: Res<crate::system::app::operator::Layout>,
    mut endpoints: ResMut<Endpoints>,
) -> Result<Progress, &'static str> {
    if let Some(receipt) = endpoints.0.as_mut() {
        let system = layout.as_ref().ok_or("Identity namespace layout")?;
        if !system
            .retire_resources(receipt)
            .map_err(|_| "Identity endpoint retirement")?
        {
            return Ok(Progress::Pending);
        }
        endpoints.0 = None;
    }
    Ok(Progress::Done)
}
