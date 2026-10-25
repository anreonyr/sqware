use crate::boot::Accounts;
use crate::system::control::unit::Service;
use crate::system::control::unit::start::Error;
use env::{Mark, Name, Page, Permission, Wait};
use system_api::supply::{Key, LeaseId, LeaseIds, State};

struct Provider {
    source: crate::system::supply::Source,
    ids: LeaseIds,
}

/// 设备树里 `riscv,iommu` 那段 `reg` 的起址（引导账里那一段的名字）。
const IOMMU_BASE: u64 = 0x301_0000;
/// 设备树里 `pci@…` 的 `ECAM` 起址。
const ECAM_BASE: u64 = 0x3000_0000;
/// 设备树 `ranges` 里那一段 64 位 PCI 窗口的父地址：精确 BAR 就从这段里取。
const APERTURE_BASE: u64 = 0x4_0000_0000;
/// 那一段窗口的字节数。
const APERTURE_SIZE: u64 = 0x4_0000_0000;
/// 寄存器与 ECAM 两段窗交出去时的形态（跟源：`ONLY` 必须一致）。
const REGION_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::ONLY);
/// aperture 那一扇还要能再授出（PCI 要从中派生那一根精确 BAR）。
const APERTURE_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::VEST)
    .union(Permission::ONLY);
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

/// 一扇受限平台窗的登记参数（引导账里的那一段 ＋ 交付形态）。
#[derive(Clone, Copy)]
struct PlatformSpec {
    resource: &'static str,
    slot: Mark,
    region: Name,
    permission: Permission,
    /// 声明长度（字节）；`None` = 接收方从自己那份映射里量。
    size: Option<u64>,
}

impl InitReceipt {
    /// 一扇**受限平台窗**：来源是引导账里那一段，只由可信装配选择的声明绑定交出去。
    ///
    /// 描述那一帧带本扇窗的几何（8 字节 = 物理起址；16 字节 = 起址 ＋ 长度）；
    /// 长度为零的那一档表示“接收方从自己那份映射里量”。
    fn platform(&mut self, accounts: &Accounts, spec: PlatformSpec) -> Result<(), &'static str> {
        use crate::system::supply::{
            CollectionItem, CollectionResource, CollectionSpec, PoleResource, PoleSpec,
        };
        let PlatformSpec {
            resource,
            slot,
            region,
            permission,
            size,
        } = spec;
        let actor = env::unit::self_id();
        let Some(token) = accounts.token(region) else {
            // Absent hardware has no supply definition; a required lease still fails normally.
            return Ok(());
        };
        let info = resource::raw::inspect(token).map_err(|_| "platform region unavailable")?;
        if !info.alive
            || info.kind != env::PieKind::Pole
            || !info.permission.contains(permission)
            || !info.permission.contains(Permission::VEST)
        {
            return Err("platform region authority");
        }
        let base = region.base().ok_or("platform region base")?;
        let shared = env::pie::accord(token, actor, permission | Permission::VEST, slot)
            .map_err(|_| "platform region local authority")?;
        self.roots
            .try_reserve(1)
            .map_err(|_| "platform root capacity")?;
        self.roots.push(shared);
        let mut items = alloc::vec::Vec::new();
        items
            .try_reserve_exact(1)
            .map_err(|_| "platform collection capacity")?;
        items.push(CollectionItem {
            shared,
            slot,
            kind: env::PieKind::Pole,
            permission,
        });
        let admission = move |request: &crate::system::supply::LeaseRequest| {
            request.origin.requester == actor
                && request.owner == actor
                && request.target.get() != 0
                && request.permission == Permission::FETCH
                && !env::unit::join_task(request.target, Wait::POLL).unwrap_or(true)
        };
        let key = Key::of(resource);
        let count = [1u8];
        let mut description = [0u8; 16];
        description[..8].copy_from_slice(&base.to_le_bytes());
        let length = match size {
            Some(size) => {
                description[8..].copy_from_slice(&size.to_le_bytes());
                16usize
            }
            None => 8usize,
        };
        let manifest = match PoleResource::from_bytes(PoleSpec {
            key,
            bytes: &count,
            description: &description[..length],
            admission,
        }) {
            Ok(manifest) => manifest,
            Err(mut failure) => {
                while failure.retry_cleanup().is_err() {
                    env::room::starve();
                }
                return Err("platform manifest");
            }
        };
        let collection = match CollectionResource::new(CollectionSpec {
            key,
            manifest,
            items,
            admission,
        }) {
            Ok(collection) => collection,
            Err(mut failure) => {
                while failure.retry_cleanup().is_err() {
                    env::room::starve();
                }
                return Err("platform collection");
            }
        };
        if let Err(mut failure) = self
            .source
            .as_mut()
            .ok_or("platform source")?
            .register(alloc::boxed::Box::new(collection))
        {
            while failure.retry_cleanup().is_err() {
                env::room::starve();
            }
            return Err("platform registration");
        }
        Ok(())
    }

    fn retention(&mut self, accounts: &Accounts) -> Result<(), &'static str> {
        use crate::system::supply::{
            CollectionItem, CollectionResource, CollectionSpec, PoleResource, PoleSpec,
        };
        let actor = env::unit::self_id();
        let authority = accounts
            .token(Name::Call(env::Call::Retain))
            .ok_or("retention authority missing")?;
        let permission = Permission::FETCH | Permission::VEST;
        self.roots
            .try_reserve(1)
            .map_err(|_| "retention source capacity")?;
        let mut items = alloc::vec::Vec::new();
        items
            .try_reserve_exact(1)
            .map_err(|_| "retention collection capacity")?;
        let shared = env::pie::accord(authority, actor, permission, Mark::of("memory/retention"))
            .map_err(|_| "retention local authority")?;
        self.roots.push(shared);
        items.push(CollectionItem {
            shared,
            slot: Mark::of("memory/retention"),
            kind: env::PieKind::Nole,
            permission,
        });
        let admission = move |request: &crate::system::supply::LeaseRequest| {
            request.origin.requester == actor
                && request.owner == actor
                && request.target.get() != 0
                && request.permission == Permission::FETCH
                && !env::unit::join_task(request.target, Wait::POLL).unwrap_or(true)
        };
        let key = Key::of("memory/retention");
        let count = [items.len() as u8];
        let manifest = match PoleResource::from_bytes(PoleSpec {
            key,
            bytes: &count,
            description: &count,
            admission,
        }) {
            Ok(manifest) => manifest,
            Err(mut failure) => {
                while failure.retry_cleanup().is_err() {
                    env::room::starve();
                }
                return Err("retention manifest");
            }
        };
        let collection = match CollectionResource::new(CollectionSpec {
            key,
            manifest,
            items,
            admission,
        }) {
            Ok(collection) => collection,
            Err(mut failure) => {
                while failure.retry_cleanup().is_err() {
                    env::room::starve();
                }
                return Err("retention collection");
            }
        };
        if let Err(mut failure) = self
            .source
            .as_mut()
            .ok_or("retention source")?
            .register(alloc::boxed::Box::new(collection))
        {
            while failure.retry_cleanup().is_err() {
                env::room::starve();
            }
            return Err("retention registration");
        }
        Ok(())
    }
}

pub struct Supplies {
    pub(crate) accounts: Accounts,
    providers: alloc::vec::Vec<Provider>,
    leases: alloc::vec::Vec<LeaseOwner>,
    source_authority: Option<env::PieToken>,
    source_roots: alloc::vec::Vec<env::PieToken>,
}
impl Supplies {
    pub fn new(
        machine: crate::support::machine::Machine,
        accounts: Accounts,
        catalog: crate::boot::Catalog<'static>,
    ) -> Result<Self, &'static str> {
        let actor = env::unit::self_id();
        let authority = env::hole::unseal(system_api::supply::ASK, env::HoleLimits::default())
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
        for entry in catalog.entries() {
            let entry = entry.map_err(|_| "image manifest entry")?;
            let length = (entry.elf.len() as u64).to_le_bytes();
            let resource = system_api::supply::Key::of(entry.name);
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
        receipt.retention(&accounts)?;
        // 受限平台窗：三扇各自一条集合，只由可信装配选择的声明绑定交出去。
        // 普通设备库存（`machine/inventory`）不因此获得 IOMMU/ECAM/aperture 的写权。
        receipt.platform(
            &accounts,
            PlatformSpec {
                resource: "platform/iommu",
                slot: Mark::of("iommu/region"),
                region: Name::Page(Page::Region(IOMMU_BASE)),
                permission: REGION_SUBSET,
                size: None,
            },
        )?;
        receipt.platform(
            &accounts,
            PlatformSpec {
                resource: "platform/ecam",
                slot: Mark::of("pci/ecam"),
                region: Name::Page(Page::Region(ECAM_BASE)),
                permission: REGION_SUBSET,
                size: None,
            },
        )?;
        receipt.platform(
            &accounts,
            PlatformSpec {
                resource: "platform/aperture",
                slot: Mark::of("pci/aperture"),
                region: Name::Page(Page::Region(APERTURE_BASE)),
                permission: APERTURE_SUBSET,
                size: Some(APERTURE_SIZE),
            },
        )?;
        let mut providers = alloc::vec::Vec::new();
        providers
            .try_reserve_exact(1)
            .map_err(|_| "supply provider capacity")?;
        let source = receipt.source.take().ok_or("image supply source")?;
        let source_authority = receipt.authority.take().ok_or("image supply authority")?;
        let source_roots = core::mem::take(&mut receipt.roots);
        let mut result = Self {
            accounts,
            providers,
            leases: alloc::vec::Vec::new(),
            source_authority: Some(source_authority),
            source_roots,
        };
        result.register(source)?;
        Ok(result)
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
        let task = service.task();
        if self.leases.iter().any(|record| {
            record.task == task
                && record.source == binding.source
                && record.binding == binding
                && record.delivered
        }) {
            return Ok(());
        }
        let (lease, delivery) = self.acquire(task, binding)?;
        let frame = system_api::supply::SupplyFrame {
            length: delivery.len,
            description: delivery.description,
        };
        let mut buffer = system_api::supply::SupplyFrame::EMPTY;
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

    pub(crate) fn acquire(
        &mut self,
        task: env::TaskId,
        binding: crate::unit::Binding,
    ) -> Result<(LeaseId, system_api::supply::Delivery), Error> {
        let Some(at) = self.providers.iter().position(|provider| {
            provider.source.key().mark() == binding.source.mark()
                && provider.source.key().name() == binding.source.name()
        }) else {
            return Err(Error::Step("supply source missing"));
        };
        let prior = self.leases.iter().position(|record| {
            record.task == task && record.source == binding.source && record.binding == binding
        });
        let lease = if let Some(prior) = prior {
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
                target: task,
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
        Ok((lease, delivery))
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
        while let Some(root) = self.source_roots.last().copied() {
            resource::raw::release(root).map_err(|_| system_api::supply::Fail::SourceLost)?;
            self.source_roots.pop();
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
