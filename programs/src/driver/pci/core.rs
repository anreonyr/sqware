//! Generic configuration effects and exact BAR grants for mapped PCI functions.

use alloc::vec::Vec;
use env::{GrantRange, Permission, PieToken, TaskId};
use resource::{dock::Dock, raw::Capability};
use crate::driver::shared::platform::Window;
use super::{api, bar::Bar, ecam::{self, Ecam}, endpoint::Endpoint,
    function_state::FunctionState, placement::Placement, reader::MAX_FUNCTIONS};
pub use super::place_request::PlaceRequest;

const BAR_SUBSET: Permission = Permission::FETCH.union(Permission::STORE)
    .union(Permission::VEST).union(Permission::ONLY);

pub struct Core {
    ecam: usize,
    limit: u32,
    aperture: Window,
    functions: Vec<FunctionState>,
}

impl Core {
    pub fn new(ecam: usize, limit: u32, aperture: Window) -> Result<Self, &'static str> {
        if ecam % env::PAGE_SIZE != 0 || limit > MAX_FUNCTIONS
            || ecam.checked_add(limit as usize * 4096).is_none()
            || aperture.size == 0 || aperture.base % env::PAGE_SIZE as u64 != 0
            || aperture.size % env::PAGE_SIZE as u64 != 0
            || aperture.size / env::PAGE_SIZE as u64 > u64::from(u32::MAX)
            || aperture.base.checked_add(aperture.size).is_none()
        {
            return Err("pci: mapped geometry");
        }
        Ok(Self { ecam, limit, aperture, functions: Vec::new() })
    }

    fn requester(&self, raw: u32) -> Result<u16, api::Fail> {
        if raw >= self.limit { return Err(api::Fail::Denied); }
        u16::try_from(raw).map_err(|_| api::Fail::Denied)
    }

    pub fn place(&mut self, request: PlaceRequest) -> Result<&Placement, api::Fail> {
        let requester = self.requester(request.requester)?;
        if request.serial == 0 || request.bar > 5 || request.recipient == TaskId::new(0) {
            return Err(api::Fail::Denied);
        }
        let at = match self.functions.iter().position(|function| function.requester == requester) {
            Some(at) => at,
            None => {
                self.functions.try_reserve(1).map_err(|_| api::Fail::Unavailable)?;
                self.functions.push(FunctionState { requester, recipient: request.recipient,
                    serial: None, floor: 0, master: false, disabled: false, bars: Vec::new() });
                self.functions.len() - 1
            }
        };
        let function = &mut self.functions[at];
        if function.recipient != request.recipient || request.serial <= function.floor
            || function.serial.is_some_and(|serial| serial != request.serial)
        {
            return Err(api::Fail::Conflict);
        }
        let new_binding = function.serial.is_none();
        let bar_at = match function.bars.iter().position(|bar| bar.index == request.bar) {
            Some(at) => at,
            None => {
                function.bars.try_reserve(1).map_err(|_| api::Fail::Unavailable)?;
                function.bars.push(Bar { index: request.bar, local: None, placement: None });
                function.bars.len() - 1
            }
        };
        function.serial = Some(request.serial);
        if new_binding { function.disabled = false; }
        if !new_binding && function.bars[bar_at].placement.as_ref()
            .is_some_and(|placement| placement.bar != PieToken::NONE)
        {
            return Ok(self.functions[at].bars[bar_at].placement.as_ref().unwrap());
        }
        let scan = Ecam::new(self.ecam, requester);
        if scan.read16(ecam::COMMAND) & ecam::COMMAND_MASTER != 0 {
            return Err(api::Fail::Denied);
        }
        if self.functions[at].bars[bar_at].placement.is_none() {
            let start = self.functions.iter().flat_map(|function| &function.bars)
                .filter_map(|bar| bar.placement.as_ref())
                .try_fold(self.aperture.base, |frontier, placement| {
                    placement.address.checked_add(u64::from(placement.pages) * env::PAGE_SIZE as u64)
                        .map(|end| frontier.max(end))
                }).ok_or(api::Fail::Denied)?;
            let remaining = self.aperture.base.checked_add(self.aperture.size)
                .and_then(|end| end.checked_sub(start)).ok_or(api::Fail::Denied)?;
            let endpoint = Endpoint::place(&scan, request.bar, (start, remaining))
                .map_err(|_| api::Fail::Denied)?;
            let pages = u32::try_from(endpoint.size().div_ceil(env::PAGE_SIZE as u64))
                .map_err(|_| api::Fail::Denied)?;
            // Geometry is retained before either derivation or delivery can fail.
            self.functions[at].bars[bar_at].placement = Some(Placement {
                address: endpoint.address(), size: endpoint.size(), pages, bar: PieToken::NONE,
            });
            crate::debug::put(&alloc::format!(
                "pci: endpoint requester={} bar={} address={:#x} size={:#x} bus-master off={:#x}",
                requester, request.bar, endpoint.address(), endpoint.size(),
                scan.read16(ecam::COMMAND) & ecam::COMMAND_MASTER,
            ));
        }
        let bar = &mut self.functions[at].bars[bar_at];
        if bar.local.is_none() {
            let placement = bar.placement.as_ref().unwrap();
            let offset = placement.address.checked_sub(self.aperture.base)
                .and_then(|bytes| usize::try_from(bytes / env::PAGE_SIZE as u64).ok())
                .ok_or(api::Fail::Denied)?;
            bar.local = Some(Capability::derive(self.aperture.token, BAR_SUBSET, api::BAR_MARK,
                GrantRange::Pages { offset, count: placement.pages as usize })
                .map_err(|_| api::Fail::Denied)?);
        }
        scan.command_or(ecam::COMMAND_MEMORY);
        if bar.placement.as_ref().unwrap().bar == PieToken::NONE {
            let token = bar.local.as_ref().unwrap()
                .grant(request.recipient, BAR_SUBSET, api::BAR_MARK)
                .map_err(|_| api::Fail::Denied)?.keep();
            bar.placement.as_mut().unwrap().bar = token;
        }
        Ok(bar.placement.as_ref().unwrap())
    }

    pub fn enable(&mut self, serial: u64, raw: u32) -> Result<(), api::Fail> {
        let requester = self.requester(raw)?;
        let function = self.functions.iter_mut().find(|function| function.requester == requester)
            .ok_or(api::Fail::Unknown)?;
        if function.serial != Some(serial) || serial <= function.floor || function.disabled
            || function.bars.is_empty()
            || function.bars.iter().any(|bar| bar.placement.as_ref().is_none_or(|placement| placement.bar == PieToken::NONE))
        {
            return Err(api::Fail::Conflict);
        }
        let scan = Ecam::new(self.ecam, requester);
        let master = scan.read16(ecam::COMMAND) & ecam::COMMAND_MASTER != 0;
        if function.master { return if master { Ok(()) } else { Err(api::Fail::Unavailable) }; }
        if master { return Err(api::Fail::Denied); }
        scan.command_or(ecam::COMMAND_MASTER);
        let seen = scan.read16(ecam::COMMAND);
        if seen & ecam::COMMAND_MASTER == 0 { return Err(api::Fail::Unavailable); }
        function.master = true;
        crate::debug::put(&alloc::format!("pci: endpoint bus-master on={:#x}", seen & ecam::COMMAND_MASTER));
        Ok(())
    }

    pub fn disable(&mut self, serial: u64, raw: u32) -> Result<(), api::Fail> {
        let requester = self.requester(raw)?;
        let Some(function) = self.functions.iter_mut().find(|function| function.requester == requester)
        else { return Ok(()); };
        if serial <= function.floor { return Ok(()); }
        if function.serial.is_some_and(|current| current != serial) { return Err(api::Fail::Conflict); }
        let scan = Ecam::new(self.ecam, requester);
        crate::debug::put(&alloc::format!("pci: binding {serial} disabling bus-master"));
        scan.command_and(ecam::COMMAND_MASTER);
        let seen = scan.read16(ecam::COMMAND);
        if seen & ecam::COMMAND_MASTER != 0 { return Err(api::Fail::Unavailable); }
        function.master = false;
        function.disabled = true;
        function.floor = function.floor.max(serial);
        crate::debug::put(&alloc::format!("pci: endpoint bus-master off={:#x}", seen & ecam::COMMAND_MASTER));
        Ok(())
    }

    pub fn release(&mut self, serial: u64, raw: u32) -> Result<(), api::Fail> {
        let requester = self.requester(raw)?;
        let Some(function) = self.functions.iter_mut().find(|function| function.requester == requester)
        else { return Ok(()); };
        if function.serial != Some(serial) {
            return if serial <= function.floor { Ok(()) } else { Err(api::Fail::Conflict) };
        }
        if !function.disabled || function.master { return Err(api::Fail::Conflict); }
        for bar in &mut function.bars {
            if bar.placement.as_ref().is_some_and(|placement| placement.bar == PieToken::NONE) {
                if let Some(local) = bar.local.as_mut() {
                    local.release().map_err(|_| api::Fail::Denied)?;
                    bar.local = None;
                }
                // A programmed BAR still decodes its address after its unpublished
                // capability is released. Its address reservation must remain.
            }
        }
        function.bars.retain(|bar| bar.placement.is_some() || bar.local.is_some());
        function.serial = None;
        function.floor = function.floor.max(serial);
        Ok(())
    }
}

pub fn open_ecam(window: &Window) -> Result<Dock, &'static str> {
    Dock::open(window.token).map_err(|_| "pci: ecam mapping")
}
