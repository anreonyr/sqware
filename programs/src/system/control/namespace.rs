//! System-owned Control endpoints installed below the shared service namespace.

use crate::system::app::namespace::{BindingInput, ResourceInput};
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::{Permission, PieToken, Wait, unit};
use system_api::operator::{PathBuf, Permit, ResourceView};

pub(crate) struct Endpoints {
    preparation: Option<crate::system::app::namespace::ResourcePreparation>,
    roots: [PieToken; system_api::control::Grant::ALL.len() + 1],
    entries: super::endpoint::Entries,
}

impl Endpoints {
    fn new() -> Self {
        Self {
            preparation: None,
            roots: [PieToken::NONE; system_api::control::Grant::ALL.len() + 1],
            entries: super::endpoint::Entries::new(),
        }
    }

    pub(crate) fn attach_face(&mut self, grant: system_api::control::Grant, token: PieToken) {
        self.entries.attach_face(grant, token);
    }

    pub(crate) fn attach_instance(&mut self, token: PieToken) {
        self.entries.attach_instance(token);
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = PieToken> + '_ {
        self.entries.entries()
    }

    pub(crate) fn face(&self, grant: system_api::control::Grant) -> Option<PieToken> {
        self.entries.face(grant)
    }

    pub(crate) fn instance(&self) -> Option<PieToken> {
        self.entries.instance()
    }

    fn release_roots(&mut self) -> Result<(), &'static str> {
        let mut failed = false;
        for token in &mut self.roots {
            if *token != PieToken::NONE {
                if resource::raw::release(*token).is_ok() {
                    *token = PieToken::NONE;
                } else {
                    failed = true;
                }
            }
        }
        if failed {
            Err("Control endpoint root release")
        } else {
            Ok(())
        }
    }

    fn plan(
        &mut self,
        principal: system_api::identity::PrincipalId,
        base: system_api::operator::Entry,
    ) -> Result<(), &'static str> {
        if self.preparation.is_some() {
            return Ok(());
        }
        let view = ResourceView::new(env::Access::STORE).ok_or("Control endpoint view")?;
        let mut bindings = Vec::new();
        bindings
            .try_reserve_exact(self.roots.len())
            .map_err(|_| "Control endpoint bindings")?;
        for grant in system_api::control::Grant::ALL {
            let token = if self.roots[grant.index()] == PieToken::NONE {
                match root(&mut self.roots[grant.index()], grant.mark()) {
                    Ok(token) => token,
                    Err(error) => {
                        let _ = self.release_roots();
                        return Err(error);
                    }
                }
            } else {
                let token = self.roots[grant.index()];
                validate_root(token, grant.mark())?;
                token
            };
            bindings.push(BindingInput {
                name: grant.name().into(),
                mark: grant.mark(),
                resource: token,
                view,
                permit: if grant == system_api::control::Grant::State {
                    Permit::Public
                } else {
                    Permit::Identity(system_api::identity::Selector::Exact(principal))
                },
            });
        }
        let instance_index = system_api::control::Grant::ALL.len();
        let instance = if self.roots[instance_index] == PieToken::NONE {
            match root(&mut self.roots[instance_index], system_api::control::ASK_MARK) {
                Ok(token) => token,
                Err(error) => {
                    let _ = self.release_roots();
                    return Err(error);
                }
            }
        } else {
            let token = self.roots[instance_index];
            validate_root(token, system_api::control::ASK_MARK)?;
            token
        };
        bindings.push(BindingInput {
            name: "instance".into(),
            mark: system_api::control::ASK_MARK,
            resource: instance,
            view,
            permit: Permit::Bound,
        });
        let Some(plan) = crate::system::app::namespace::Prepare::resources(ResourceInput {
            base,
            path: PathBuf::try_new(system_api::control::DIR.as_str()).ok_or("Control endpoint path")?,
            retain_prefix: 0,
            bindings,
            wait: Wait::AtMost(crate::support::timing::BOOT_MS),
        }) else {
            let _ = self.release_roots();
            return Err("Control endpoint plan");
        };
        self.preparation = Some(crate::system::app::namespace::ResourcePreparation::new(
            plan,
        ));
        Ok(())
    }
}

fn root(slot: &mut PieToken, mark: env::Mark) -> Result<PieToken, &'static str> {
    let token =
        env::pie::unseal(env::UnsealArgs::hole(mark)).map_err(|_| "Control endpoint Hole")?;
    *slot = token;
    validate_root(token, mark)?;
    Ok(token)
}

fn validate_root(token: PieToken, mark: env::Mark) -> Result<(), &'static str> {
    let info = resource::raw::inspect(token).map_err(|_| "Control endpoint inspection")?;
    let owner = unit::self_id();
    if !info.alive
        || info.owner != owner
        || info.vestor != env::TaskId::new(0)
        || info.kind != env::PieKind::Hole
        || info.mark != mark
        || !info.permission.contains(Permission::STORE)
    {
        return Err("Control endpoint provenance");
    }
    Ok(())
}

pub(crate) fn install(
    resources: &mut ::schedule::Resources<'static>,
) -> Result<(), crate::system::app::InstallError> {
    resources.insert(Endpoints::new())?;
    Ok(())
}

pub(crate) fn prepare(
    layout: Res<crate::system::app::operator::Layout>,
    roster: Res<crate::system::control::identity::Roster>,
    mut endpoints: ResMut<Endpoints>,
) -> Result<Progress, &'static str> {
    let principal = roster.control().ok_or("Control identity missing")?;
    let system = layout.as_ref().ok_or("Operator layout unavailable")?;
        endpoints.plan(
            principal,
            system
                .resolve(system_api::namespace::SERVICE, Wait::AtMost(crate::support::timing::BOOT_MS))
                .map_err(|_| "Service namespace root")?,
        )?;
    let preparation = endpoints
        .preparation
        .as_mut()
        .ok_or("Control endpoint receipt")?;
    system
        .prepare_resources(preparation)
        .map_err(|_| "Control endpoint preparation")?;
    for grant in system_api::control::Grant::ALL {
        let root = endpoints.roots[grant.index()];
        endpoints.attach_face(grant, root);
    }
    let instance = endpoints.roots[system_api::control::Grant::ALL.len()];
    endpoints.attach_instance(instance);
    Ok(Progress::Done)
}

pub(crate) fn retire(
    layout: Res<crate::system::app::operator::Layout>,
    mut endpoints: ResMut<Endpoints>,
) -> Result<Progress, &'static str> {
    if let Some(preparation) = endpoints.preparation.as_mut() {
        let system = layout.as_ref().ok_or("Operator layout unavailable")?;
        if !system
            .retire_resources(preparation)
            .map_err(|_| "Control endpoint retirement")?
        {
            return Ok(Progress::Pending);
        }
        endpoints.preparation = None;
    }
    endpoints.release_roots()?;
    endpoints.entries.clear();
    Ok(Progress::Done)
}
