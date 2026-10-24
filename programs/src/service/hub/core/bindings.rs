//! 册上的 DMA 安全绑定那一半：认领记录、进度推进、主人死亡与整机结清。
//!
//! 与普通设备账共用同一本 `Ledger`（不新增第二本设备账）：普通设备的主人一没格子就空，
//! DMA 绑定在本次责任结清前**不**因主人死亡变为可重绑。

use env::PieToken;
use hub_api::{BindingId, BindingState, Close, DeviceId, Fail, Failure};

use super::binding::Binding;
use super::ledger::Ledger;
use super::ClaimRequest;

/// Recently completed receipts; open bindings are never evicted.
const CLOSED_HISTORY: usize = 32;

impl Ledger {
    /// Find a published device by its full source-qualified coordinate.
    pub fn device(&self, id: DeviceId) -> Option<DeviceId> {
        self.cells
            .iter()
            .filter(|cell| cell.ready())
            .filter_map(|cell| cell.device())
            .find(|device| *device == id)
    }

    /// 这台设备此刻开着几笔绑定（未结清）。
    pub fn open_bindings(&self, device: DeviceId) -> usize {
        self.bindings
            .iter()
            .filter(|binding| binding.open() && binding.device == device)
            .count()
    }

    /// 一格一格地看：这一格是谁的（DMA 那一档“哪一台”由门牌回答，见 `device_by_door`）。
    pub fn device_by_door(&self, door: PieToken) -> Option<DeviceId> {
        self.cells
            .iter()
            .filter(|cell| cell.ready() && cell.entry().door == door)
            .find_map(|cell| cell.device())
    }

    /// 已登记设备的坐标窗口（**不对外开**：认领走门牌，清单不开放）。
    #[allow(dead_code, reason = "diagnostic window for the ledger itself")]
    pub fn devices(&self) -> impl Iterator<Item = DeviceId> + '_ {
        self.cells
            .iter()
            .filter(|cell| cell.ready())
            .filter_map(|cell| cell.device())
    }

    /// 认领一台：同调用者同编号同参幂等；异参或旧编号冲突；设备已有开着的一笔则 `Taken`。
    pub fn claim(&mut self, request: ClaimRequest) -> Result<BindingId, Fail> {
        if request.id == 0 {
            return Err(Fail::Bad);
        }
        if let Some(binding) = self
            .bindings
            .iter()
            .find(|binding| binding.execution == request.from && binding.claim == request.id)
        {
            if binding.device != request.device
                || binding.sensor != request.sensor
            {
                return Err(Fail::Conflict);
            }
            return Ok(binding.id);
        }
        if self.settling {
            return Err(Fail::Denied);
        }
        let Some(device) = self.device(request.device) else {
            return Err(Fail::Unknown);
        };
        let execution = self
            .admissions
            .iter()
            .position(|admission| admission.task == request.from && admission.claimable())
            .ok_or(Fail::Denied)?;
        if request.id <= self.admissions[execution].claim_high() {
            return Err(Fail::Conflict);
        }
        if self.open_bindings(request.device) > 0 {
            return Err(Fail::Taken);
        }
        let serial = self.serial.checked_add(1).ok_or(Fail::Full)?;
        self.bindings.try_reserve(1).map_err(|_| Fail::Full)?;
        self.serial = serial;
        self.admissions[execution].advance_claim(request.id);
        self.bindings.push(Binding::new(
            BindingId {
                hub: self.hub,
                serial,
            },
            request.id,
            device,
            request.from,
            request.sensor,
        ));
        Ok(BindingId {
            hub: self.hub,
            serial,
        })
    }

    /// 这一号绑定（旧号找不到 ⇒ `None`）。
    pub fn binding(&self, serial: u64) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|binding| binding.id.serial == serial)
    }

    /// 所有还开着（未结清）的绑定。
    pub fn open_bindings_all(&self) -> impl Iterator<Item = &Binding> + '_ {
        self.bindings.iter().filter(|binding| binding.open())
    }

    pub fn binding_mut(&mut self, serial: u64) -> Option<&mut Binding> {
        self.bindings
            .iter_mut()
            .find(|binding| binding.id.serial == serial)
    }

    /// 主人死了吗（报活孔那一手，与普通设备账同一把尺）。
    ///
    /// DMA 绑定**只转收场**：状态从 Preparing/Active 进 Closing，不从账上消失。
    pub fn owner_gone(&mut self, alive: impl Fn(PieToken) -> bool) -> usize {
        let mut gone = 0;
        for binding in &mut self.bindings {
            if binding.open() && binding.state != BindingState::Closing && !alive(binding.sensor) {
                binding.close(Close::Fault);
                gone += 1;
            }
        }
        gone
    }

    /// 要求关闭：重复请求继续同一责任，Fault 可覆盖 Normal，不反向恢复。
    pub fn close(&mut self, serial: u64, reason: Close) -> Result<BindingState, Fail> {
        let binding = self.binding_mut(serial).ok_or(Fail::Unknown)?;
        binding.close(reason);
        Ok(binding.state)
    }

    /// 封闭准入，退休执行实例并关闭绑定，返回未结清责任数。
    pub fn settle(&mut self) -> usize {
        self.settling = true;
        let open = self.bindings.iter().filter(|binding| binding.open()).count();
        for binding in &mut self.bindings {
            if binding.open() {
                binding.close(Close::Normal);
            }
        }
        for admission in &mut self.admissions {
            admission.close();
        }
        open.saturating_add(self.admissions.len())
    }

    /// 记一笔原始失败（失败方、操作、原失败码）。
    pub fn note_failure(&mut self, serial: u64, failure: Failure) {
        if let Some(binding) = self.binding_mut(serial) {
            binding.failure.get_or_insert(failure);
        }
    }

    /// 一笔结清：状态落 `Closed`，设备重新可认领。
    pub fn finish(&mut self, serial: u64) {
        let Some(at) = self.bindings.iter().position(|binding| binding.id.serial == serial && binding.open()) else {
            return;
        };
        let mut binding = self.bindings.remove(at);
        binding.state = BindingState::Closed;
        self.bindings.push(binding);
        if self.bindings.iter().filter(|binding| !binding.open()).count() > CLOSED_HISTORY {
            let oldest = self.bindings.iter().position(|binding| !binding.open())
                .expect("closed history exceeds its limit");
            self.bindings.remove(oldest);
        }
    }

}
