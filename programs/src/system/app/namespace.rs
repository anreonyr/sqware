//! System-owned Operator namespace layout and per-task installation receipts.

use alloc::vec::Vec;
use core::cell::Cell;
use env::{PieToken, TaskId, Wait};
use system_api::operator::{
    self as api, Entry, GrantId, GrantSpec, NamespaceKey, OperationId, PathBuf, Permit, ResourceView,
    Scope,
};
use system_client::operator::{Capability, PartResult};

mod installed;
mod retire;

pub use installed::Installed;

pub struct BindingInput {
    pub name: alloc::string::String,
    pub mark: env::Mark,
    pub resource: PieToken,
    pub view: ResourceView,
    pub permit: Permit,
}

pub struct Prepare {
    base: Entry,
    /// App-selected relative path below `base`.
    path: PathBuf,
    /// Created prefix entries before this index are shared placement containers.
    retain_prefix: usize,
    grant: Option<GrantSpec>,
    bindings: Vec<BindingInput>,
    wait: Wait,
}

/// A System-owned resource publication plan that needs no child authority.
pub struct ResourceInput {
    pub base: Entry,
    pub path: PathBuf,
    pub retain_prefix: usize,
    pub bindings: Vec<BindingInput>,
    pub wait: Wait,
}

pub struct PrepareInput {
    pub base: Entry,
    /// App-selected prefix below `base`; empty for shared named service locations.
    pub prefix: PathBuf,
    /// Number of leading prefix segments owned by the longer-lived product layout.
    pub retain_prefix: usize,
    pub declaration: crate::unit::operator::Namespace,
    pub grant_id: GrantId,
    pub holder: TaskId,
    pub source: Option<TaskId>,
    pub bindings: Vec<BindingInput>,
    pub wait: Wait,
}

pub struct AuthorityInput {
    pub declaration: crate::unit::operator::Namespace,
    pub id: GrantId,
    pub holder: TaskId,
    pub anchor: Entry,
    pub source: Option<TaskId>,
}

pub struct Preparation {
    plan: Prepare,
    installed: Installed,
    path_cursor: usize,
    binding_cursor: usize,
    operations: Vec<api::OperationId>,
    path_attempted: bool,
    binding_attempted: bool,
    ready: bool,
    retired: bool,
}

/// Stable, retryable receipt for a shared resource publication without a holder grant.
pub type ResourcePreparation = Preparation;

pub struct System {
    capability: Capability,
    root: Entry,
    next_operation: Cell<u64>,
}

pub struct PartRequest<'a> {
    pub parent: Entry,
    pub name: &'a str,
    pub operation: OperationId,
    pub wait: Wait,
}

pub struct AssociateRequest {
    pub key: NamespaceKey,
    pub entry: Entry,
    pub wait: Wait,
}

impl System {
    pub fn new(info: crate::system::operator::management::RootInfo) -> Result<Self, api::Fail> {
        let capability = Capability::direct_at(info.entry.operator, info.entrance, info.entry)?;
        Ok(Self {
            capability,
            root: info.entry,
            next_operation: Cell::new(1),
        })
    }

    pub fn part(&self, request: PartRequest<'_>) -> Result<PartResult, api::Fail> {
        self.capability.part(
            request.parent,
            request.name,
            request.operation,
            request.wait,
        )
    }

    pub fn associate(&self, request: AssociateRequest) -> Result<(), api::Fail> {
        self.capability
            .associate(request.key, request.entry, request.wait)
    }

    pub fn resolve(&self, key: NamespaceKey, wait: Wait) -> Result<Entry, api::Fail> {
        self.capability.resolve(key, wait)
    }

    pub fn root(&self) -> Entry {
        self.root
    }

    /// Reserves caller-owned IDs in the same sequence as namespace preparation.
    pub fn reserve_operations(&self, count: usize) -> Result<api::OperationId, api::Fail> {
        let start = self.next_operation.get();
        let end = start
            .checked_add(u64::try_from(count).map_err(|_| api::Fail::Full)?)
            .ok_or(api::Fail::Full)?;
        self.next_operation.set(end);
        Ok(api::OperationId(start))
    }

    /// Resumes the same part/bind/grant requests after errors or lost replies.
    pub fn prepare(&self, preparation: &mut Preparation) -> Result<(), api::Fail> {
        if preparation.retired {
            return Err(api::Fail::Denied);
        }
        if preparation.ready {
            return Ok(());
        }
        if preparation.operations.is_empty() {
            let count = preparation
                .plan
                .path
                .len()
                .checked_add(preparation.plan.bindings.len())
                .ok_or(api::Fail::Full)?;
            preparation
                .operations
                .try_reserve(count)
                .map_err(|_| api::Fail::Full)?;
            let start = self.reserve_operations(count)?.0;
            for offset in 0..count {
                preparation
                    .operations
                    .push(api::OperationId(start + offset as u64));
            }
        }
        let mut anchor = preparation.plan.base;
        for index in 0..preparation.path_cursor {
            let name = preparation
                .plan
                .path
                .iter()
                .nth(index)
                .ok_or(api::Fail::Unknown)?;
            anchor = self
                .capability
                .part(
                    anchor,
                    name,
                    preparation.operations[index],
                    preparation.plan.wait,
                )?
                .entry;
        }
        while let Some(name) = preparation.plan.path.iter().nth(preparation.path_cursor) {
            preparation
                .installed
                .entries
                .try_reserve(1)
                .map_err(|_| api::Fail::Full)?;
            preparation.path_attempted = true;
            let outcome = self.capability.part(
                anchor,
                name,
                preparation.operations[preparation.path_cursor],
                preparation.plan.wait,
            )?;
            if preparation.path_cursor >= preparation.plan.retain_prefix
                && outcome.created
                && !preparation.installed.entries.contains(&outcome.entry)
            {
                preparation.installed.entries.push(outcome.entry);
            }
            preparation.path_attempted = false;
            preparation.path_cursor += 1;
            anchor = outcome.entry;
        }
        preparation.installed.anchor = anchor;
        while let Some(input) = preparation.plan.bindings.get(preparation.binding_cursor) {
            preparation
                .installed
                .bindings
                .try_reserve(1)
                .map_err(|_| api::Fail::Full)?;
            let info = ::resource::raw::inspect(input.resource).map_err(|_| api::Fail::Dead)?;
            if !info.alive
                || info.mark != input.mark
                || !input.view.fits(info.permission)
                || preparation.plan.grant.as_ref().is_some_and(|grant| {
                    grant.source.is_some_and(|source| source != info.owner)
                        || grant.source_mark.is_some_and(|mark| mark != info.mark)
                })
            {
                return Err(api::Fail::Denied);
            }
            preparation.binding_attempted = true;
            let binding = self.capability.bind(
                anchor,
                &input.name,
                input.resource,
                input.view,
                input.permit,
                preparation.operations[preparation.plan.path.len() + preparation.binding_cursor],
                preparation.plan.wait,
            )?;
            if binding.created && !preparation.installed.bindings.contains(&binding.binding) {
                preparation.installed.bindings.push(binding.binding);
            }
            preparation.binding_attempted = false;
            preparation.binding_cursor += 1;
        }
        if let Some(mut grant) = preparation.plan.grant.clone() {
            grant.anchor = anchor;
            preparation.installed.grant = Some(grant.id);
            preparation.installed.authority =
                Some(self.capability.grant(grant, None, preparation.plan.wait)?);
        }
        preparation.ready = true;
        Ok(())
    }

    pub fn retire_preparation(&self, preparation: &mut Preparation) -> Result<bool, api::Fail> {
        if preparation.ready {
            return Err(api::Fail::Denied);
        }
        if preparation.path_attempted {
            let mut anchor = preparation.plan.base;
            for index in 0..preparation.path_cursor {
                let name = preparation
                    .plan
                    .path
                    .iter()
                    .nth(index)
                    .ok_or(api::Fail::Unknown)?;
                anchor = self
                    .capability
                    .part(
                        anchor,
                        name,
                        preparation.operations[index],
                        preparation.plan.wait,
                    )?
                    .entry;
            }
            let index = preparation.path_cursor;
            let name = preparation
                .plan
                .path
                .iter()
                .nth(index)
                .ok_or(api::Fail::Unknown)?;
            match self.capability.part(
                anchor,
                name,
                preparation.operations[index],
                preparation.plan.wait,
            ) {
                Ok(outcome) => {
                    if index >= preparation.plan.retain_prefix
                        && outcome.created
                        && !preparation.installed.entries.contains(&outcome.entry)
                    {
                        preparation.installed.entries.push(outcome.entry);
                    }
                }
                Err(_)
                    if self.capability.inspect_operation(
                        anchor,
                        preparation.operations[index],
                        preparation.plan.wait,
                    )? == api::OperationState::Failed => {}
                Err(fail) => return Err(fail),
            }
            preparation.path_attempted = false;
        }
        if preparation.binding_attempted {
            let mut anchor = preparation.plan.base;
            for index in 0..preparation.plan.path.len() {
                let name = preparation
                    .plan
                    .path
                    .iter()
                    .nth(index)
                    .ok_or(api::Fail::Unknown)?;
                anchor = self
                    .capability
                    .part(
                        anchor,
                        name,
                        preparation.operations[index],
                        preparation.plan.wait,
                    )?
                    .entry;
            }
            let index = preparation.binding_cursor;
            let input = preparation
                .plan
                .bindings
                .get(index)
                .ok_or(api::Fail::Unknown)?;
            match self.capability.bind(
                anchor,
                &input.name,
                input.resource,
                input.view,
                input.permit,
                preparation.operations[preparation.plan.path.len() + index],
                preparation.plan.wait,
            ) {
                Ok(outcome) => {
                    if outcome.created && !preparation.installed.bindings.contains(&outcome.binding)
                    {
                        preparation.installed.bindings.push(outcome.binding);
                    }
                }
                Err(_)
                    if self.capability.inspect_operation(
                        anchor,
                        preparation.operations[preparation.plan.path.len() + index],
                        preparation.plan.wait,
                    )? == api::OperationState::Failed => {}
                Err(fail) => return Err(fail),
            }
            preparation.binding_attempted = false;
        }
        let count = preparation.operations.len();
        for index in preparation.path_cursor + preparation.binding_cursor..count {
            let id = preparation.operations[index];
            let state = self.capability.inspect_operation(
                preparation.installed.anchor,
                id,
                preparation.plan.wait,
            )?;
            if state == api::OperationState::Unseen {
                self.capability.cancel_operation(
                    preparation.installed.anchor,
                    id,
                    preparation.plan.wait,
                )?;
            } else if state == api::OperationState::Pending {
                return Err(api::Fail::Dead);
            }
        }
        let done = self.retire(&mut preparation.installed)?;
        preparation.retired = done;
        Ok(done)
    }

    /// Runs or retries an app-owned namespace resource publication.
    pub fn prepare_resources(
        &self,
        preparation: &mut ResourcePreparation,
    ) -> Result<(), api::Fail> {
        self.prepare(preparation)
    }

    /// Cancels an incomplete resource plan or retires a completed plan's receipt.
    pub fn retire_resources(
        &self,
        preparation: &mut ResourcePreparation,
    ) -> Result<bool, api::Fail> {
        if preparation.ready {
            let done = self.retire(&mut preparation.installed)?;
            preparation.retired = done;
            Ok(done)
        } else {
            self.retire_preparation(preparation)
        }
    }

    pub fn capability(&self) -> &Capability {
        &self.capability
    }
}

impl Prepare {
    pub fn declared(input: PrepareInput) -> Option<Self> {
        let declaration = input.declaration;
        if !declaration.valid() {
            return None;
        }
        let path = if declaration.path.is_empty() {
            input.prefix.clone()
        } else {
            input.prefix.as_path().try_join(declaration.path)?
        };
        if input.retain_prefix > path.len() {
            return None;
        }
        let grant = authority(AuthorityInput {
            declaration,
            id: input.grant_id,
            holder: input.holder,
            anchor: input.base,
            source: input.source,
        })?;
        if input.bindings.len() > declaration.bindings.len() {
            return None;
        }
        for (index, binding_input) in input.bindings.iter().enumerate() {
            let declared = declaration
                .bindings
                .iter()
                .find(|declared| binding_input.name == declared.name)?;
            if input.bindings[..index]
                .iter()
                .any(|previous| previous.name == binding_input.name)
                || binding_input.name != declared.name
                || binding_input.mark != declared.mark
                || binding_input.view.access() != declared.view
                || binding_input.permit != declared.permit
            {
                return None;
            }
        }
        Some(Self {
            base: input.base,
            path,
            retain_prefix: input.retain_prefix,
            grant: Some(grant),
            bindings: input.bindings,
            wait: input.wait,
        })
    }

    /// Creates a receipt-tracked app resource publication without granting a namespace capability.
    pub fn resources(input: ResourceInput) -> Option<Self> {
        if input.retain_prefix > input.path.len() {
            return None;
        }
        if input.bindings.iter().enumerate().any(|(index, binding)| {
            !system_api::operator::name::valid(&binding.name)
                || binding.mark == env::Mark::NONE
                || input.bindings[..index]
                    .iter()
                    .any(|previous| previous.name == binding.name)
        }) {
            return None;
        }
        Some(Self {
            base: input.base,
            path: input.path,
            retain_prefix: input.retain_prefix,
            grant: None,
            bindings: input.bindings,
            wait: input.wait,
        })
    }
}

impl Preparation {
    pub fn new(plan: Prepare) -> Self {
        let installed = Installed {
            anchor: plan.base,
            authority: None,
            grant: None,
            bindings: Vec::new(),
            entries: Vec::new(),
            wait: plan.wait,
        };
        Self {
            plan,
            installed,
            path_cursor: 0,
            binding_cursor: 0,
            operations: Vec::new(),
            path_attempted: false,
            binding_attempted: false,
            ready: false,
            retired: false,
        }
    }

    pub fn into_installed(self) -> Option<Installed> {
        self.ready.then_some(self.installed)
    }
}

/// Converts a static unit declaration into a runtime grant anchored at its actual entry.
pub fn authority(input: AuthorityInput) -> Option<GrantSpec> {
    let AuthorityInput {
        declaration,
        id,
        holder,
        anchor,
        source,
    } = input;
    if !declaration.valid() {
        return None;
    }
    let scope = match declaration.scope {
        crate::unit::operator::Scope::Subtree => Scope::Subtree,
        crate::unit::operator::Scope::Names(names) => {
            let mut paths = Vec::new();
            paths.try_reserve(names.len()).ok()?;
            for name in names {
                paths.push(PathBuf::try_new(name)?);
            }
            Scope::Names(paths)
        }
    };
    Some(GrantSpec {
        id,
        holder,
        anchor,
        scope,
        ops: declaration.ops,
        permit: declaration.permit,
        source,
        source_mark: declaration.source_mark.map(env::Mark::new),
    })
}
