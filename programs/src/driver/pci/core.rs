//! PCI 那一域的核心：枚举、一根 BAR 的落位，以及 MASTER 的开启与清除。
//!
//! 三件事各是各的独立效果：落位只开内存空间，`enable` 才让端点能发起；
//! `disable` 清 MASTER 并回读，但它**不等于**端点已无在途（那一档归设备恢复方）。
//! 本域不解释设备协议（virtio 寄存器归设备那一侧），只读设备自己的能力表。

use env::{GrantRange, Permission, PieToken, TaskId};
use resource::dock::Dock;
use resource::raw::Capability;

use crate::driver::pci::api;
use crate::driver::pci::ecam::{self, Ecam};
use crate::driver::pci::endpoint::Endpoint;
use crate::driver::pci::placement::Placement;
use crate::driver::pci::virtio::Caps;
use crate::driver::shared::platform::Window;

/// 端点：virtio 的供应商号与块设备号（过渡态 / 纯 1.x 两个号都收）。
const VENDOR: u16 = 0x1AF4;
const BLOCK: [u16; 2] = [0x1001, 0x1042];
/// 交给恢复方那一枚窗口的形态：读写 ＋ 跟源（aperture 那一枚带 `ONLY`）。
const BAR_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::VEST)
    .union(Permission::ONLY);

pub struct Core {
    /// ECAM 段在本域的起址（已授权、已映射）。
    ecam: usize,
    /// 发现的那一台在 PCI 上的号（也是 IOMMU 的设备号）。
    requester: u16,
    /// 能力表说的那一根 BAR。
    bar: u8,
    caps: Caps,
    /// 从引导账交来的 PCI 窗口：BAR 就从这段里落位。
    aperture: Window,
    /// 当前 BAR 的本地精确派生根；aperture 比 BAR 宽，远端只收这一段。
    local: Option<Capability>,
    /// 本地派生已成功、远端交付尚未成功的那一笔；重试复用同一句柄。
    pending: Option<Placement>,
    placement: Option<Placement>,
    floor: u64,
}

/// 一次落位请求（协调入口核过之后转给核心）。
pub struct PlaceRequest {
    pub serial: u64,
    pub requester: u32,
    pub recovery: TaskId,
    pub bar: u8,
}

impl Core {
    /// 枚举：ECAM 上找那台端点，读它自己的能力表。
    pub fn discover(ecam: usize, aperture: Window) -> Result<Self, &'static str> {
        let requester = ecam::find(ecam, VENDOR, &BLOCK).ok_or("pci: endpoint absent")?;
        let scan = Ecam::new(ecam, requester);
        let (bar, caps) = Caps::read(&scan).map_err(|_| "pci: virtio caps")?;
        Ok(Self {
            ecam,
            requester,
            bar,
            caps,
            aperture,
            local: None,
            pending: None,
            placement: None,
            floor: 0,
        })
    }

    pub fn requester(&self) -> u16 {
        self.requester
    }

    pub fn bar(&self) -> u8 {
        self.bar
    }

    /// 落位：把那一根 BAR 落进 aperture，**只开内存空间**，再把精确窗口交给恢复方。
    ///
    /// 同一号绑定同参数重试复用原记录（不重复落位、不重复交付）；异参冲突。
    pub fn place(&mut self, request: PlaceRequest) -> Result<&Placement, api::Fail> {
        if request.requester != u32::from(self.requester)
            || request.bar != api::BAR_ANY && request.bar != self.bar
        {
            return Err(api::Fail::Denied);
        }
        if request.serial <= self.floor {
            return Err(api::Fail::Conflict);
        }
        if let Some(prior) = self.placement.as_mut() {
            if prior.recovery != request.recovery
                || (prior.serial != 0 && prior.serial != request.serial)
            {
                return Err(api::Fail::Conflict);
            }
            if prior.serial == 0 {
                prior.serial = request.serial;
                prior.disabled = false;
            }
        }
        if self.placement.is_some() {
            return Ok(self.placement.as_ref().unwrap());
        }
        if let Some(pending) = self.pending.as_ref() {
            if pending.serial != request.serial || pending.recovery != request.recovery {
                return Err(api::Fail::Conflict);
            }
            return self.deliver();
        }
        let scan = Ecam::new(self.ecam, self.requester);
        // 落位之前先核 MASTER 是关的（回读，不拿复位默认值当前提）。
        let before = scan.read16(ecam::COMMAND);
        if before & ecam::COMMAND_MASTER != 0 {
            return Err(api::Fail::Denied);
        }
        let endpoint = Endpoint::place(&scan, self.bar, (self.aperture.base, self.aperture.size))
            .map_err(|_| api::Fail::Denied)?;
        let first = ((endpoint.address() - self.aperture.base) / env::PAGE_SIZE as u64) as usize;
        let count = (endpoint.size() as usize).div_ceil(env::PAGE_SIZE);
        let pages = u32::try_from(count).map_err(|_| api::Fail::Denied)?;
        // aperture 比 BAR 宽：先把精确 BAR 派生成本地子能力，再交给恢复方。
        let local = Capability::derive(
            self.aperture.token,
            BAR_SUBSET,
            api::BAR_MARK,
            GrantRange::Pages {
                offset: first,
                count,
            },
        )
        .map_err(|_| api::Fail::Denied)?;
        // 本地派生根留到这一根 BAR 的实际清理；失败重试不重派、不丢句柄。
        self.local = Some(local);
        self.pending = Some(Placement {
            serial: request.serial,
            address: endpoint.address(),
            size: endpoint.size(),
            pages,
            caps: self.caps,
            recovery: request.recovery,
            bar: PieToken::NONE,
            master: false,
            disabled: false,
        });
        let after = scan.read16(ecam::COMMAND);
        crate::debug::put(&alloc::format!(
            "pci: endpoint requester={} bar={} address={:#x} size={:#x} bus-master off={:#x}",
            endpoint.requester(),
            self.bar,
            endpoint.address(),
            endpoint.size(),
            after & ecam::COMMAND_MASTER,
        ));
        self.deliver()
    }

    /// 把当前本地精确根交给恢复方；失败保留同一派生根与待交付记录供原样重试。
    fn deliver(&mut self) -> Result<&Placement, api::Fail> {
        let local = self.local.as_ref().ok_or(api::Fail::Denied)?;
        let pending = self.pending.as_ref().ok_or(api::Fail::Denied)?;
        let bar = local
            .grant(pending.recovery, BAR_SUBSET, api::BAR_MARK)
            .map_err(|_| api::Fail::Denied)?
            .keep();
        let mut placement = self.pending.take().ok_or(api::Fail::Denied)?;
        placement.bar = bar;
        self.placement = Some(placement);
        self.placement.as_ref().ok_or(api::Fail::Denied)
    }

    /// 开启 MASTER：写后回读确认；只在本次绑定尚未被关过、且此刻还是关的时候执行。
    pub fn enable(&mut self, serial: u64) -> Result<(), api::Fail> {
        let placement = self
            .placement
            .as_mut()
            .filter(|placement| placement.serial == serial && serial > self.floor)
            .ok_or(api::Fail::Unknown)?;
        if placement.disabled {
            return Err(api::Fail::Conflict);
        }
        let scan = Ecam::new(self.ecam, self.requester);
        if placement.master {
            return if scan.read16(ecam::COMMAND) & ecam::COMMAND_MASTER != 0 {
                Ok(())
            } else {
                Err(api::Fail::Unavailable)
            };
        }
        if scan.read16(ecam::COMMAND) & ecam::COMMAND_MASTER != 0 {
            return Err(api::Fail::Denied);
        }
        scan.command_or(ecam::COMMAND_MASTER);
        let seen = scan.read16(ecam::COMMAND);
        if seen & ecam::COMMAND_MASTER == 0 {
            return Err(api::Fail::Unavailable);
        }
        placement.master = true;
        crate::debug::put(&alloc::format!(
            "pci: endpoint bus-master on={:#x}",
            seen & ecam::COMMAND_MASTER,
        ));
        Ok(())
    }

    /// 清除 MASTER：写后回读确认；关过就记下来，不再重开。
    pub fn disable(&mut self, serial: u64) -> Result<(), api::Fail> {
        if serial <= self.floor {
            return Ok(());
        }
        if self
            .placement
            .as_ref()
            .or(self.pending.as_ref())
            .is_some_and(|placement| placement.serial != 0 && placement.serial != serial)
        {
            return Err(api::Fail::Conflict);
        }
        let scan = Ecam::new(self.ecam, self.requester);
        crate::debug::put(&alloc::format!(
            "pci: binding {serial} disabling bus-master"
        ));
        scan.command_and(ecam::COMMAND_MASTER);
        let seen = scan.read16(ecam::COMMAND);
        if seen & ecam::COMMAND_MASTER != 0 {
            return Err(api::Fail::Unavailable);
        }
        if let Some(placement) = self.placement.as_mut().or(self.pending.as_mut()) {
            placement.master = false;
            placement.disabled = true;
        }
        self.floor = self.floor.max(serial);
        crate::debug::put(&alloc::format!(
            "pci: endpoint bus-master off={:#x}",
            seen & ecam::COMMAND_MASTER,
        ));
        Ok(())
    }

    /// 这一号绑定的落位事实（读数用）。
    pub fn placement(&self, serial: u64) -> Option<&Placement> {
        self.placement
            .as_ref()
            .filter(|placement| placement.serial == serial)
    }

    /// 已交付的 BAR 根跨绑定复用；未交付根在确认禁用后释放。
    pub fn release(&mut self, serial: u64) -> Result<(), api::Fail> {
        if let Some(pending) = self.pending.as_ref() {
            if pending.serial != serial {
                return if serial <= self.floor {
                    Ok(())
                } else {
                    Err(api::Fail::Conflict)
                };
            }
            if !pending.disabled || pending.master {
                return Err(api::Fail::Conflict);
            }
            self.local
                .as_mut()
                .ok_or(api::Fail::Denied)?
                .release()
                .map_err(|_| api::Fail::Denied)?;
            self.local = None;
            self.pending = None;
        }
        if let Some(placement) = self.placement.as_mut() {
            if placement.serial == serial {
                if !placement.disabled || placement.master {
                    return Err(api::Fail::Conflict);
                }
                placement.serial = 0;
            } else if placement.serial != 0 && serial > self.floor {
                return Err(api::Fail::Conflict);
            }
        }
        self.floor = self.floor.max(serial);
        Ok(())
    }
}

/// ECAM 那一扇窗的映射：返本域起址；映射活到本对象最后一次访问之后。
pub fn open_ecam(window: &Window) -> Result<Dock, &'static str> {
    Dock::open(window.token).map_err(|_| "pci: ecam mapping")
}
