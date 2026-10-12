//! Assembly receipts for declared Operator namespaces.

use alloc::vec::Vec;
use env::wire::Span as _;
use env::{Permission, PieToken, TaskId, TeamId, Wait, pie};
use resource::raw::{Hole, inspect};
use system_api::operator::{self as api, PathBuf};
use system_client::operator::GrantInfo;

const TASK_CAP: usize = 256;
const NAMESPACE_CAP: usize = 32;

/// Coordinates taken from Control's live slot or instance record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Coordinates {
    pub task: TaskId,
    pub team: Option<TeamId>,
}

pub struct PrepareTask {
    pub task: TaskId,
    pub program: Option<&'static crate::unit::UnitFile>,
    pub wait: Wait,
}

enum Receipt {
    Vacant,
    Preparing(crate::system::app::namespace::Preparation),
    Installed(crate::system::app::namespace::Installed),
}

struct Delivery {
    root: Option<PieToken>,
    entrance: Option<PieToken>,
    sent: bool,
}

struct DeliveryRequest {
    task: TaskId,
    namespace: usize,
    grant: GrantInfo,
    wait: Wait,
}

struct TaskReceipt {
    coordinates: Coordinates,
    program: Option<&'static str>,
    items: Vec<Receipt>,
    placements: Vec<(system_api::operator::NamespaceKey, Receipt)>,
    delivery: Vec<Delivery>,
    planned: bool,
    complete: bool,
}

/// Bounded, persistent prepare/retire state owned by Assembly.
pub struct NamespaceInstallations {
    tasks: Vec<TaskReceipt>,
    next_grant: u64,
}

impl NamespaceInstallations {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            next_grant: 1,
        }
    }

    /// Reserves the task's installation receipt with coordinates read from Control.
    pub fn coordinates(&mut self, coordinates: Coordinates) -> Result<(), api::Fail> {
        if let Some(record) = self
            .tasks
            .iter()
            .find(|record| record.coordinates.task == coordinates.task)
        {
            return if record.coordinates == coordinates {
                Ok(())
            } else {
                Err(api::Fail::Denied)
            };
        }
        if self.tasks.len() >= TASK_CAP || self.tasks.try_reserve(1).is_err() {
            return Err(api::Fail::Full);
        }
        self.tasks.push(TaskReceipt {
            coordinates,
            program: None,
            items: Vec::new(),
            placements: Vec::new(),
            delivery: Vec::new(),
            planned: false,
            complete: false,
        });
        Ok(())
    }

    /// Creates the stable plan once, then resumes the same per-namespace receipts on retry.
    pub fn prepare_task(
        &mut self,
        system: &crate::system::app::namespace::System,
        request: PrepareTask,
    ) -> Result<bool, api::Fail> {
        let program_name = request.program.map_or("", crate::unit::UnitFile::name);
        let existing_index = self
            .tasks
            .iter()
            .position(|record| record.coordinates.task == request.task)
            .ok_or(api::Fail::Denied)?;
        let coordinates = self.tasks[existing_index].coordinates;
        let declarations = request
            .program
            .map_or(&[][..], |program| program.operator.namespace);
        if self.tasks[existing_index].program.is_some() {
            if self.tasks[existing_index].program != Some(program_name) {
                return Err(api::Fail::Denied);
            }
        } else {
            if declarations.len() > NAMESPACE_CAP {
                return Err(api::Fail::Full);
            }
            if declarations.iter().any(|decl| {
                decl.placement == crate::unit::operator::Placement::Instance
                    && coordinates.team.is_none()
            }) {
                return Err(api::Fail::Denied);
            }
            let mut placements = Vec::new();
            placements
                .try_reserve_exact(declarations.len())
                .map_err(|_| api::Fail::Full)?;
            for declaration in declarations.iter().copied() {
                if declaration.placement != crate::unit::operator::Placement::Instance
                    || placements.iter().any(|(key, _)| *key == declaration.key)
                {
                    continue;
                }
                let base = system.resolve(declaration.key, request.wait)?;
                let team = coordinates.team.ok_or(api::Fail::Denied)?;
                let prefix = PathBuf::try_new(&alloc::format!(
                    "{}/{}",
                    team.get(),
                    coordinates.task.get()
                ))
                .ok_or(api::Fail::Denied)?;
                let plan = crate::system::app::namespace::Prepare::resources(
                    crate::system::app::namespace::ResourceInput {
                        base,
                        path: prefix,
                        retain_prefix: 1,
                        bindings: Vec::new(),
                        wait: request.wait,
                    },
                )
                .ok_or(api::Fail::Denied)?;
                placements.push((
                    declaration.key,
                    Receipt::Preparing(crate::system::app::namespace::Preparation::new(plan)),
                ));
            }
            let mut delivery = Vec::new();
            delivery
                .try_reserve_exact(declarations.len())
                .map_err(|_| api::Fail::Full)?;
            delivery.resize_with(declarations.len(), || Delivery {
                root: None,
                entrance: None,
                sent: false,
            });
            let record = &mut self.tasks[existing_index];
            record.program = Some(program_name);
            record.items = Vec::new();
            record.placements = placements;
            record.delivery = delivery;
            record.planned = false;
        }

        {
            let record = &mut self.tasks[existing_index];
            for (_, item) in &mut record.placements {
                if let Receipt::Preparing(preparation) = item {
                    system.prepare(preparation)?;
                    let completed = core::mem::replace(item, Receipt::Vacant);
                    let Receipt::Preparing(preparation) = completed else {
                        return Err(api::Fail::Dead);
                    };
                    *item = Receipt::Installed(preparation.into_installed().ok_or(api::Fail::Dead)?);
                }
            }
            if !record.planned {
                let mut items = Vec::new();
                items
                    .try_reserve_exact(declarations.len())
                    .map_err(|_| api::Fail::Full)?;
                let mut next_grant = self.next_grant;
                for declaration in declarations.iter().copied() {
                    let base = match declaration.placement {
                        crate::unit::operator::Placement::Shared => {
                            system.resolve(declaration.key, request.wait)?
                        }
                        crate::unit::operator::Placement::Instance => record
                            .placements
                            .iter()
                            .find(|(key, _)| *key == declaration.key)
                            .and_then(|(_, receipt)| match receipt {
                                Receipt::Installed(installed) => Some(installed.anchor),
                                Receipt::Preparing(_) | Receipt::Vacant => None,
                            })
                            .ok_or(api::Fail::Dead)?,
                    };
                    let id = api::GrantId(next_grant);
                    next_grant = next_grant.checked_add(1).ok_or(api::Fail::Full)?;
                    let plan = crate::system::app::namespace::Prepare::declared(
                        crate::system::app::namespace::PrepareInput {
                            base,
                            prefix: PathBuf::root(),
                            retain_prefix: 0,
                            declaration,
                            grant_id: id,
                            holder: request.task,
                            source: Some(request.task),
                            bindings: Vec::new(),
                            wait: request.wait,
                        },
                    )
                    .ok_or(api::Fail::Denied)?;
                    items.push(Receipt::Preparing(
                        crate::system::app::namespace::Preparation::new(plan),
                    ));
                }
                record.items = items;
                record.planned = true;
                self.next_grant = next_grant;
            }
        }

        let record = self
            .tasks
            .iter_mut()
            .find(|record| record.coordinates.task == request.task)
            .ok_or(api::Fail::Unknown)?;
        for (index, item) in record.items.iter_mut().enumerate() {
            if let Receipt::Preparing(preparation) = item {
                system.prepare(preparation)?;
                let completed = core::mem::replace(item, Receipt::Vacant);
                let Receipt::Preparing(preparation) = completed else {
                    return Err(api::Fail::Dead);
                };
                *item = Receipt::Installed(preparation.into_installed().ok_or(api::Fail::Dead)?);
            }
            if let Receipt::Installed(installed) = item {
                if let Some(info) = installed.authority {
                    deliver(
                        &mut record.delivery[index],
                        DeliveryRequest {
                            task: request.task,
                            namespace: index,
                            grant: info,
                            wait: request.wait,
                        },
                    )?;
                }
            }
        }
        record.complete = true;
        Ok(true)
    }

    /// Retires only receipts owned by this task; failures retain remaining work for retry.
    pub fn retire_task(
        &mut self,
        system: &crate::system::app::namespace::System,
        task: TaskId,
    ) -> Result<bool, api::Fail> {
        let Some(index) = self.tasks.iter().position(|record| record.coordinates.task == task) else {
            return Ok(true);
        };
        let record = &mut self.tasks[index];
        while let Some(item) = record.items.last_mut() {
            match item {
                Receipt::Preparing(preparation) => {
                    if !system.retire_preparation(preparation)? {
                        return Ok(false);
                    }
                }
                Receipt::Installed(installed) => {
                    if !system.retire(installed)? {
                        return Ok(false);
                    }
                }
                Receipt::Vacant => return Err(api::Fail::Dead),
            }
            let delivery = record.delivery.last_mut().ok_or(api::Fail::Dead)?;
            if let Some(root) = delivery.root {
                pie::release(root, env::ReleaseMode::Revoke).map_err(|_| api::Fail::Dead)?;
                delivery.root = None;
                delivery.entrance = None;
            }
            record.items.pop();
            record.delivery.pop();
        }
        while let Some((_, item)) = record.placements.last_mut() {
            match item {
                Receipt::Preparing(preparation) => {
                    if !system.retire_preparation(preparation)? {
                        return Ok(false);
                    }
                }
                Receipt::Installed(installed) => {
                    if !system.retire(installed)? {
                        return Ok(false);
                    }
                }
                Receipt::Vacant => return Err(api::Fail::Dead),
            }
            record.placements.pop();
        }
        self.tasks.swap_remove(index);
        Ok(true)
    }

    pub fn grant(&self, task: TaskId, namespace: usize) -> Option<GrantInfo> {
        let record = self.tasks.iter().find(|record| record.coordinates.task == task)?;
        if !record.delivery.get(namespace)?.sent {
            return None;
        }
        match record.items.get(namespace)? {
            Receipt::Installed(installed) => installed.authority,
            Receipt::Preparing(_) | Receipt::Vacant => None,
        }
    }

    #[cfg(test)]
    pub fn prepared_task(
        &self,
        task: TaskId,
        program: Option<&'static crate::unit::UnitFile>,
    ) -> bool {
        let Some(record) = self.tasks.iter().find(|record| record.coordinates.task == task) else {
            return false;
        };
        let program_name = program.map_or("", crate::unit::UnitFile::name);
        let declarations = program.map_or(0, |program| program.operator.namespace.len());
        record.program == Some(program_name)
            && record.complete
            && record.planned
            && record.items.len() == declarations
            && record
                .items
                .iter()
                .all(|item| matches!(item, Receipt::Installed(_)))
            && record.delivery.iter().all(|delivery| delivery.sent)
    }

    /// True only after an explicit prepare call has completed, including empty plans.
    pub fn prepared(&self, task: TaskId) -> bool {
        self.tasks
            .iter()
            .find(|record| record.coordinates.task == task)
            .is_some_and(|record| record.complete)
    }
}

fn deliver(delivery: &mut Delivery, request: DeliveryRequest) -> Result<(), api::Fail> {
    if delivery.sent {
        return Ok(());
    }
    let DeliveryRequest {
        task,
        namespace,
        grant,
        wait,
    } = request;
    let mark = api::descriptor_mark(namespace).ok_or(api::Fail::Denied)?;
    let root = match delivery.root {
        Some(root) => root,
        None => {
            let root = Hole::unseal(mark).map_err(|_| api::Fail::Unknown)?.token();
            delivery.root = Some(root);
            root
        }
    };
    let facts = inspect(root).map_err(|_| api::Fail::Dead)?;
    if !facts.alive
        || facts.owner != env::unit::self_id()
        || facts.kind != env::PieKind::Hole
        || facts.mark != mark
        || !facts
            .permission
            .contains(Permission::FETCH | Permission::STORE | Permission::VEST)
    {
        return Err(api::Fail::Denied);
    }
    if delivery.entrance.is_none() {
        let entrance =
            pie::accord(root, task, Permission::FETCH, mark).map_err(|_| api::Fail::Denied)?;
        delivery.entrance = Some(entrance);
    }
    let mut bytes = [0; api::GrantDescriptor::LEN];
    let len = api::GrantDescriptor {
        id: grant.id.0,
        entry_operator: grant.entry.operator,
        entry_root: u8::from(matches!(grant.entry.target, api::EntryTarget::Root)),
        entry_id: match grant.entry.target {
            api::EntryTarget::Root => system_api::operator::EntryId::new(0),
            api::EntryTarget::Node(id) => id,
        },
        entrance: grant.entrance,
    }
    .store_at(&mut bytes, 0)
    .ok_or(api::Fail::Full)?;
    if len != bytes.len() {
        return Err(api::Fail::Full);
    }
    Hole::from_raw(root)
        .push(&bytes, wait)
        .map_err(|_| api::Fail::Dead)?;
    delivery.sent = true;
    Ok(())
}
