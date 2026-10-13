use super::{answer::Inbox, claims::Claims};
use crate::system::{
    app::{namespace as layout, operator::Layout},
    control::identity::Roster,
};
use ::schedule::{Progress, Res, ResMut};
use alloc::string::String;
use env::{PieToken, Wait};
use system_api::operator::{PathBuf, Permit, ResourceView};

struct NamespaceReceipt {
    preparation: Option<layout::ResourcePreparation>,
    installed: Option<layout::Installed>,
    entry: Option<PieToken>,
}
impl NamespaceReceipt {
    fn new() -> Self {
        Self {
            preparation: None,
            installed: None,
            entry: None,
        }
    }
}

pub(crate) struct EndpointState {
    pub(crate) inbox: Inbox,
    namespace: NamespaceReceipt,
    claims: Claims,
}
impl EndpointState {
    pub(crate) fn new() -> Self {
        Self {
            inbox: Inbox::new(),
            namespace: NamespaceReceipt::new(),
            claims: Claims::default(),
        }
    }

    pub(crate) fn grant_claim(
        &mut self,
        task: env::TaskId,
        construction: &[crate::unit::Construction],
    ) -> Result<(), &'static str> {
        if construction.is_empty() {
            return Ok(());
        }
        let source = self
            .inbox
            .entry
            .ok_or("Loader Claim entry is not installed")?;
        self.claims.grant(source, task)
    }

    pub(crate) fn retire_claim(&mut self, task: env::TaskId) -> Result<(), &'static str> {
        self.claims.retire(task)
    }

    pub(crate) fn retire_claims(&mut self) -> Result<(), &'static str> {
        self.claims.retire_all()
    }
}

pub(crate) fn faces(
    layout_system: Res<Layout>,
    roster: Res<Roster>,
    mut endpoints: ResMut<EndpointState>,
) -> Result<Progress, &'static str> {
    let EndpointState {
        inbox,
        namespace: receipt,
        ..
    } = &mut *endpoints;
    if receipt.installed.is_some() {
        inbox.entry = receipt.entry;
        return Ok(Progress::Done);
    }
    let system = layout_system.as_ref().ok_or("Operator namespace missing")?;
    if receipt.preparation.is_none() {
        let principal = roster.control().ok_or("Control identity missing")?;
        let grant = system_api::loader::Grant::Claim;
        let entry = match receipt.entry {
            Some(entry) => entry,
            None => {
                crate::support::face::mount::entry(grant.mark(), grant.name())
                    .map_err(|_| "Loader Claim endpoint")?
                    .0
            }
        };
        receipt.entry = Some(entry);
        let view = ResourceView::new(env::Access::STORE).ok_or("Loader Claim view")?;
        let input = layout::ResourceInput {
            base: system
                .resolve(
                    system_api::namespace::SERVICE,
                    Wait::AtMost(crate::support::timing::BOOT_MS),
                )
                .map_err(|_| "Loader namespace root")?,
            path: PathBuf::try_new(system_api::loader::DIR).ok_or("Loader namespace path")?,
            retain_prefix: 0,
            bindings: alloc::vec![layout::BindingInput {
                name: String::from(grant.name()),
                mark: grant.mark(),
                resource: entry,
                view,
                permit: Permit::Identity(system_api::identity::Selector::Exact(principal)),
            }],
            wait: Wait::AtMost(crate::support::timing::BOOT_MS),
        };
        let plan = layout::Prepare::resources(input).ok_or("Loader namespace plan")?;
        receipt.preparation = Some(layout::ResourcePreparation::new(plan));
    }
    let preparation = receipt
        .preparation
        .as_mut()
        .ok_or("Loader namespace plan")?;
    if system.prepare_resources(preparation).is_err() {
        let cleanup = system.retire_resources(preparation);
        if matches!(cleanup, Ok(true)) {
            receipt.preparation = None;
        }
        return Err(if cleanup.is_err() {
            "Loader namespace cleanup"
        } else {
            "Loader namespace binding"
        });
    }
    receipt.installed = receipt
        .preparation
        .take()
        .and_then(layout::Preparation::into_installed);
    inbox.entry = receipt.entry;
    if receipt.installed.is_none() {
        return Err("Loader namespace receipt");
    }
    Ok(Progress::Done)
}

pub(crate) fn withdraw(
    layout_system: Res<Layout>,
    mut endpoints: ResMut<EndpointState>,
    waiting: Res<crate::system::app::wait::Waiting>,
) -> Result<Progress, crate::system::app::Fault> {
    let Some(system) = layout_system.as_ref() else {
        return Err(crate::system::app::Fault::Dead);
    };
    endpoints
        .retire_claims()
        .map_err(|_| crate::system::app::Fault::Shutdown)?;
    let EndpointState {
        inbox,
        namespace: receipt,
        ..
    } = &mut *endpoints;
    if let Some(installed) = receipt.installed.as_mut() {
        if !system
            .retire(installed)
            .map_err(|_| crate::system::app::Fault::Room)?
        {
            return Ok(Progress::Pending);
        }
        receipt.installed = None;
    }
    if let Some(entry) = receipt.entry {
        waiting.detach(entry);
        env::pie::release(entry, env::ReleaseMode::Revoke)
            .map_err(|_| crate::system::app::Fault::Shutdown)?;
        receipt.entry = None;
        inbox.entry = None;
    }
    Ok(Progress::Done)
}
