use crate::boot::Accounts;
use crate::system::control::unit::Service;
use crate::system::control::unit::start::Error;
use env::{Call, Mark, Name, Page, Permission, Wait};
use resource::retention::{Failure, Holds, Registration, RetentionId, ScopeId};
use system_api::supply::{Key, LeaseId, LeaseIds, State};

struct Provider {
    source: crate::system::supply::Source,
    ids: LeaseIds,
}
#[derive(Clone, Copy)]
struct LeaseOwner {
    task: env::TaskId,
    source: Key,
    id: LeaseId,
    binding: crate::unit::Binding,
    delivered: bool,
}

struct InitReceipt {
    source: Option<crate::system::supply::Source>,
    authority: Option<env::PieToken>,
    roots: alloc::vec::Vec<env::PieToken>,
}

impl Drop for InitReceipt {
    fn drop(&mut self) {
        if let Some(source) = self.source.as_mut() {
            while source.shutdown().is_err() {
                env::room::starve();
            }
        }
        if let Some(authority) = self.authority {
            while resource::raw::release(authority).is_err() {
                env::room::starve();
            }
            self.authority = None;
        }
        while let Some(root) = self.roots.last().copied() {
            if resource::raw::release(root).is_ok() {
                self.roots.pop();
            } else {
                env::room::starve();
            }
        }
    }
}

pub struct Supplies {
    pub(crate) accounts: Accounts,
    providers: alloc::vec::Vec<Provider>,
    leases: alloc::vec::Vec<LeaseOwner>,
    pub(crate) retention: Holds,
    source_authority: Option<env::PieToken>,
    inventory_roots: alloc::vec::Vec<env::PieToken>,
}
impl Supplies {
    pub fn new(
        machine: crate::support::machine::Machine,
        accounts: Accounts,
        catalog: crate::boot::Catalog<'static>,
    ) -> Result<Self, &'static str> {
        let actor = env::unit::self_id();
        let authority = env::pie::unseal(env::UnsealArgs::hole(system_api::supply::ASK))
            .map_err(|_| "image supply authority")?;
        let images = match crate::system::supply::Source::new(
            system_api::supply::Key::of("system/initrd"),
            authority,
        ) {
            Ok(source) => source,
            Err(_) => {
                while resource::raw::release(authority).is_err() {
                    env::room::starve();
                }
                return Err("image supply source");
            }
        };
        let mut receipt = InitReceipt {
            source: Some(images),
            authority: Some(authority),
            roots: alloc::vec::Vec::new(),
        };
        for package in crate::unit::PACKAGES {
            let Some(entry) = catalog.find(package.name) else {
                continue;
            };
            let length = (entry.elf.len() as u64).to_le_bytes();
            let resource = system_api::supply::Key::of(package.name);
            let definition = match crate::system::supply::PoleResource::from_bytes(
                crate::system::supply::PoleSpec {
                    key: resource,
                    bytes: entry.elf,
                    description: &length,
                    admission: move |request: &crate::system::supply::LeaseRequest| {
                        request.origin.requester == actor
                            && request.owner == actor
                            && request.target.get() != 0
                            && request.permission.contains(env::Permission::FETCH)
                            && !request.permission.contains(env::Permission::STORE)
                            && !env::unit::join_task(request.target, Wait::POLL).unwrap_or(true)
                    },
                },
            ) {
                Ok(definition) => definition,
                Err(mut failure) => {
                    while failure.retry_cleanup().is_err() {
                        env::room::starve();
                    }
                    return Err("image package registration");
                }
            };
            if let Err(mut failure) = receipt
                .source
                .as_mut()
                .ok_or("image supply source")?
                .register(alloc::boxed::Box::new(definition))
            {
                while failure.retry_cleanup().is_err() {
                    env::room::starve();
                }
                return Err("image package registration");
            }
        }
        let devices = machine.devices().ok_or("machine inventory capacity")?;
        let count = devices
            .len()
            .checked_add(2)
            .ok_or("machine inventory capacity")?;
        if hub_api::Inventory::header(count).is_none() {
            return Err("machine inventory exceeds lease roots");
        }
        let mut items: alloc::vec::Vec<crate::system::supply::CollectionItem> =
            alloc::vec::Vec::new();
        let mut inventory_items: alloc::vec::Vec<hub_api::InventoryItem> = alloc::vec::Vec::new();
        items
            .try_reserve(count)
            .map_err(|_| "machine inventory allocation")?;
        inventory_items
            .try_reserve(count)
            .map_err(|_| "machine inventory allocation")?;
        receipt
            .roots
            .try_reserve_exact(count)
            .map_err(|_| "machine inventory allocation")?;
        let mut add_item = |name: Name, index: usize| -> Result<(), &'static str> {
            let token = accounts.token(name).ok_or("machine resource missing")?;
            let info = resource::raw::inspect(token).map_err(|_| "machine resource unavailable")?;
            let slot = hub_api::inventory_slot(index).ok_or("machine inventory slot capacity")?;
            let mut permission = info.permission & (Permission::FETCH | Permission::STORE);
            permission |= Permission::VEST;
            if info.permission.contains(Permission::ONLY) {
                permission |= Permission::ONLY;
            }
            let kernel_boot_resource =
                info.owner == env::TaskId::new(0) && accounts.token(name) == Some(token);
            let authority_holder = info.owner == actor || info.vestor == actor;
            if !info.alive
                || info.kind != name.kind()
                || !info.permission.contains(Permission::VEST)
                || (!authority_holder && !kernel_boot_resource)
                || permission.is_empty()
                || (items
                    .iter()
                    .any(|item: &crate::system::supply::CollectionItem| item.slot == slot))
            {
                return Err("machine resource authority");
            }
            let shared = env::pie::accord(token, actor, permission, info.mark)
                .map_err(|_| "machine resource local authority")?;
            receipt.roots.push(shared);
            items.push(crate::system::supply::CollectionItem {
                shared,
                slot,
                kind: info.kind,
                permission,
            });
            inventory_items.push(hub_api::InventoryItem {
                resource: name,
                owner: info.owner,
                kind: info.kind,
                permission,
            });
            Ok(())
        };
        add_item(Name::Page(Page::Dtb), 0)?;
        add_item(Name::Trap(env::Trap::SupervisorExternal), 1)?;
        for (index, device) in devices.iter().enumerate() {
            add_item(device.resource, index + 2)?;
        }
        drop(add_item);
        let inventory =
            hub_api::Inventory::of(&inventory_items).ok_or("machine inventory validation")?;
        let mut encoded_inventory = alloc::vec::Vec::new();
        encoded_inventory
            .try_reserve_exact(inventory.encoded_len())
            .map_err(|_| "machine inventory allocation")?;
        encoded_inventory.resize(inventory.encoded_len(), 0);
        inventory
            .store(&mut encoded_inventory)
            .ok_or("machine inventory encoding")?;
        let count_description = [count as u8];
        let inventory_resource = system_api::supply::Key::of("machine/inventory");
        let manifest =
            match crate::system::supply::PoleResource::from_bytes(crate::system::supply::PoleSpec {
                key: inventory_resource,
                bytes: &encoded_inventory,
                description: &count_description,
                admission: move |request: &crate::system::supply::LeaseRequest| {
                    request.origin.requester == actor
                        && request.owner == actor
                        && request.target.get() != 0
                        && request.permission.contains(Permission::FETCH)
                        && !request.permission.contains(Permission::STORE)
                        && !env::unit::join_task(request.target, Wait::POLL).unwrap_or(true)
                },
            }) {
                Ok(manifest) => manifest,
                Err(mut failure) => {
                    while failure.retry_cleanup().is_err() {
                        env::room::starve();
                    }
                    return Err("machine inventory initialization");
                }
            };
        let collection = match crate::system::supply::CollectionResource::new(
            crate::system::supply::CollectionSpec {
                key: inventory_resource,
                manifest,
                items,
                admission: move |request: &crate::system::supply::LeaseRequest| {
                    request.origin.requester == actor
                        && request.owner == actor
                        && request.target.get() != 0
                        && request.permission.contains(Permission::FETCH)
                        && !request.permission.contains(Permission::STORE)
                        && !env::unit::join_task(request.target, Wait::POLL).unwrap_or(true)
                },
            },
        ) {
            Ok(collection) => collection,
            Err(mut failure) => {
                while failure.retry_cleanup().is_err() {
                    env::room::starve();
                }
                return Err("machine inventory definition");
            }
        };
        if let Err(mut failure) = receipt
            .source
            .as_mut()
            .ok_or("image supply source")?
            .register(alloc::boxed::Box::new(collection))
        {
            while failure.retry_cleanup().is_err() {
                env::room::starve();
            }
            return Err("machine inventory resource registration");
        }
        let mut providers = alloc::vec::Vec::new();
        providers
            .try_reserve_exact(1)
            .map_err(|_| "supply provider capacity")?;
        let source = receipt.source.take().ok_or("image supply source")?;
        let source_authority = receipt.authority.take().ok_or("image supply authority")?;
        let inventory_roots = core::mem::take(&mut receipt.roots);
        let mut result = Self {
            accounts,
            providers,
            leases: alloc::vec::Vec::new(),
            retention: Holds::new(),
            source_authority: Some(source_authority),
            inventory_roots,
        };
        result.register(source)?;
        Ok(result)
    }

    pub(crate) fn retain_memory(
        &mut self,
        scope: ScopeId,
        source: env::AcquireSource,
    ) -> Result<RetentionId, Failure> {
        let authority = self
            .accounts
            .token(Name::Call(Call::Retain))
            .ok_or(Failure::Denied)?;
        self.retention.register(
            Registration {
                scope,
                source,
                retention: authority,
            },
            &mut super::retention::References,
        )
    }

    pub fn register(&mut self, source: crate::system::supply::Source) -> Result<(), &'static str> {
        if self
            .providers
            .iter()
            .any(|provider| provider.source.key().mark() == source.key().mark())
        {
            return Err("duplicate supply source key");
        }
        self.providers
            .try_reserve(1)
            .map_err(|_| "supply provider capacity")?;
        self.providers.push(Provider {
            source,
            ids: LeaseIds::new(),
        });
        Ok(())
    }
}
impl Supplies {
    pub fn supply(
        &mut self,
        service: &mut Service,
        binding: crate::unit::Binding,
    ) -> Result<(), Error> {
        use wire::Message;
        let Some(at) = self.providers.iter().position(|provider| {
            provider.source.key().mark() == binding.source.mark()
                && provider.source.key().name() == binding.source.name()
        }) else {
            return Err(Error::Step("supply source missing"));
        };
        let task = service.task();
        let prior = self.leases.iter().position(|record| {
            record.task == task && record.source == binding.source && record.binding == binding
        });
        let lease = if let Some(prior) = prior {
            if self.leases[prior].delivered {
                return Ok(());
            }
            self.leases[prior].id
        } else {
            self.leases
                .try_reserve(1)
                .map_err(|_| Error::Step("supply lease capacity"))?;
            let lease = self.providers[at]
                .ids
                .reserve()
                .map_err(|_| Error::Step("supply lease id"))?;
            self.leases.push(LeaseOwner {
                task,
                source: binding.source,
                id: lease,
                binding,
                delivered: false,
            });
            lease
        };
        let acquired = self.providers[at].source.acquire(
            lease,
            crate::system::supply::Parameters {
                source: binding.source,
                resource: binding.resource,
                target: service.task(),
                slot: binding.slot,
                permission: binding.permission,
            },
        );
        let (state, delivery) = match acquired {
            Ok(result) => result,
            Err(failure) => {
                crate::debug::put(&alloc::format!(
                    "supply: acquire {} failed {failure:?}",
                    binding.resource.name()
                ));
                if matches!(
                    self.providers[at].source.inspect(lease),
                    Ok((State::Unseen | State::Closed | State::Released, _))
                ) {
                    self.leases.retain(|record| {
                        record.id != lease || record.source.mark() != binding.source.mark()
                    });
                }
                return Err(Error::Step("supply lease acquisition"));
            }
        };
        if state != system_api::supply::State::Granted {
            return Err(Error::Step("supply lease not granted"));
        }
        let delivery = delivery.ok_or(Error::Step("supply delivery missing"))?;
        if delivery.seed == env::PieToken::NONE {
            self.release_lease(binding.source, lease)?;
            return Err(Error::Step("supply delivery missing"));
        }
        let frame = crate::unit::SupplyFrame {
            length: delivery.len,
            description: delivery.description,
        };
        let mut buffer = crate::unit::SupplyFrame::EMPTY;
        let Some(length) = frame.store(&mut buffer) else {
            self.release_lease(binding.source, lease)?;
            return Err(Error::Step("supply descriptor frame"));
        };
        if let Err(error) = service.send_descriptor(binding.slot, &buffer[..length]) {
            self.release_lease(binding.source, lease)?;
            return Err(error);
        }
        if let Some(record) = self.leases.iter_mut().find(|record| {
            record.task == task && record.source == binding.source && record.id == lease
        }) {
            record.delivered = true;
        }
        Ok(())
    }

    fn release_lease(&mut self, source: Key, id: LeaseId) -> Result<(), Error> {
        let at = self
            .providers
            .iter()
            .position(|provider| provider.source.key().mark() == source.mark())
            .ok_or(Error::Step("supply source missing"))?;
        match self.providers[at].source.release(id) {
            Ok(State::Released | State::Closed) => {
                self.leases
                    .retain(|record| record.id != id || record.source.mark() != source.mark());
                Ok(())
            }
            _ => Err(Error::Step("supply lease release pending")),
        }
    }

    pub fn release_target(&mut self, task: env::TaskId) -> Result<(), Error> {
        let mut at = 0;
        while at < self.leases.len() {
            let record = self.leases[at];
            if record.task != task {
                at += 1;
                continue;
            }
            self.release_lease(record.source, record.id)?;
        }
        Ok(())
    }

    pub fn sweep_images(&mut self) {
        let mut index = 0;
        while index < self.leases.len() {
            let record = self.leases[index];
            let task = record.task;
            if !matches!(env::unit::join_task(task, Wait::POLL), Ok(true)) {
                index += 1;
                continue;
            }
            if self.release_lease(record.source, record.id).is_err() {
                index += 1;
            }
        }
    }
    pub fn grant_call(&self, call: env::Call, task: env::TaskId) -> Result<(), &'static str> {
        let token = self
            .accounts
            .token(Name::Call(call))
            .ok_or("Call authority missing")?;
        env::pie::accord(token, task, env::Permission::FETCH, Mark::NONE)
            .map_err(|_| "Call authority grant")?;
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), system_api::supply::Fail> {
        for provider in &mut self.providers {
            provider.source.shutdown()?;
        }
        if let Some(authority) = self.source_authority {
            resource::raw::release(authority).map_err(|_| system_api::supply::Fail::SourceLost)?;
            self.source_authority = None;
        }
        while let Some(root) = self.inventory_roots.last().copied() {
            resource::raw::release(root).map_err(|_| system_api::supply::Fail::SourceLost)?;
            self.inventory_roots.pop();
        }
        Ok(())
    }
}

pub(crate) fn shutdown(
    mut supplies: ::schedule::ResMut<Supplies>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    match supplies.shutdown() {
        Ok(()) => Ok(::schedule::Progress::Done),
        Err(system_api::supply::Fail::Pending) => Ok(::schedule::Progress::Pending),
        Err(_) => Err(crate::system::app::Fault::Shutdown),
    }
}
