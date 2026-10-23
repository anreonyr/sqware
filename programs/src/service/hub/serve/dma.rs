//! DMA 安全绑定那一面的招待：发现、认领、关闭、查看、整机结清，
//! 以及两条链（准备／收场）的逐步推进。
//!
//! 两条链的次序是合同，不因拆分而改变：
//!
//! ```text
//! 准备：PCI 落位 → 恢复方取得恢复映射并交子授权 → IOMMU 取得帧引用
//!       → IOMMU 发布翻译 → PCI 开 MASTER ⇒ Active
//! 收场：整域停执行方 CPU 控制 → IOMMU 撤翻译 → PCI 清 MASTER → 设备恢复方 reset/drain
//!       → IOMMU 关队列 → IOMMU 解除引用 → 设备恢复方放下映射 ⇒ Closed
//! ```
//!
//! 每一步没完成就留在那一步（重试同一责任，不重置超时也不重开 MASTER）；
//! 旧编号不能推进新绑定；各方清理须覆盖部分取得与未取得的责任。

use alloc::vec::Vec;
use env::{Access, PieToken, TaskId, Wait};
use hub_api::{
    BindingState, Close, DmaSaid, DmaWire, Fail, Failure, PARTY_HUB, PARTY_IOMMU,
    PARTY_PCI, PARTY_RECOVERY,
};
use resource::raw::Hole;
use wire::Message;

use crate::driver::iommu::api as iommu;
use crate::driver::pci::api as pci;
use crate::driver::recovery::api as recovery;
use crate::driver::shared::dma;
use crate::service::hub::core::{ClaimRequest, Ledger, Step};
use crate::service::hub::serve::directory::Directory;
use crate::service::hub::serve::peers::Peers;
use crate::service::hub::serve::{claim, sweep};

/// 一次硬件域调用的上限（毫秒）：驱动器内部的握手各自有界。
const MS: usize = 3000;

fn fail_to_code(fail: Fail) -> u8 {
    hub_api::fail_to_code(Some(fail))
}

fn failure(party: u8, op: u8, code: u8) -> Failure {
    Failure { party, op, code }
}

/// 招待一问（门上的那一枚已由调用方核过是 DMA 那一面）。
pub fn turn(
    ledger: &mut Ledger,
    peers: Option<&Peers>,
    directory: &Directory<'_>,
    from: TaskId,
    wire: DmaWire,
) -> DmaSaid {
    match wire {
        DmaWire::Enroll(enroll) => match peers {
            Some(peers) => enroll_device(ledger, peers, directory, from, enroll),
            None => DmaSaid::of(fail_to_code(Fail::Unknown)),
        },
        // 认领只能走**设备门牌**（那里才有类联盟的 permit）；DMA 面上来的当场拒。
        DmaWire::Claim(_) => DmaSaid::of(fail_to_code(Fail::Denied)),
        DmaWire::Close(close)
            if !ledger
                .binding(close.serial)
                .is_some_and(|binding| binding.execution == from) =>
        {
            DmaSaid::of(fail_to_code(Fail::Denied))
        }
        DmaWire::Close(close) => match Close::of_code(close.reason) {
            Some(reason) => match ledger.close(close.serial, reason) {
                Ok(state) => DmaSaid::bound(close.serial, state),
                Err(fail) => DmaSaid::of(fail_to_code(fail)),
            },
            None => DmaSaid::of(fail_to_code(Fail::Bad)),
        },
        DmaWire::Inspect(inspect) => match ledger.binding(inspect.serial) {
            Some(binding) => DmaSaid::progress(binding.id.serial, binding.state, binding.failure),
            None => DmaSaid::of(fail_to_code(Fail::Unknown)),
        },
        DmaWire::Settle(_) if from != env::unit::sire() => DmaSaid::of(fail_to_code(Fail::Denied)),
        DmaWire::Settle(_) => {
            let open = ledger.settle();
            let mut said = DmaSaid::of(wire::OK);
            said.count = u8::try_from(open).unwrap_or(u8::MAX);
            said
        }
    }
}

/// 设备门牌上的 DMA 认领。
///
/// “哪一台”由**哪一枚门响了**回答（调用方给的就是那一枚）；准入由门牌上的类联盟
/// permit 回答（认领者先经 `bond` 进类，否则 `find` 都拿不到这一枚）——
/// 不看报文里的自报设备号，也不开放设备清单。执行实例是否真实存在、是否属于 Hub
/// 已由 launch 队列经 Control 实例面核验并记入准入；这里只接受那道准入。
pub fn claim_at_door(
    ledger: &mut Ledger,
    peers: &Peers,
    door: PieToken,
    from: TaskId,
    frame: &[u8],
) -> Option<DmaSaid> {
    let (claim, end) = <hub_api::DmaClaim as env::wire::Span>::fetch_at(frame, 0)?;
    if claim.op != hub_api::DMA_CLAIM || end != frame.len() {
        return None;
    }
    let said = match ledger.device_by_door(door) {
        None => DmaSaid::of(fail_to_code(Fail::Unknown)),
        // 报活孔必须由认领者自己转授。
        Some(_) if !claim::sensor_from(from, claim.sensor) => {
            DmaSaid::of(fail_to_code(Fail::Denied))
        }
        Some(device) => {
            let request = ClaimRequest {
                from,
                id: claim.id,
                requester: device.requester,
                sensor: claim.sensor,
                recovery: peers.recovery.host(),
            };
            match ledger.claim(request) {
                Ok(id) => {
                    let binding = ledger.binding(id.serial).expect("claim receipt");
                    DmaSaid::progress(id.serial, binding.state, binding.failure)
                }
                Err(fail) => DmaSaid::of(fail_to_code(fail)),
            }
        }
    };
    let mut tx = ipc::hand::Sender::<DmaSaid>::from_raw(claim.back);
    let _ = tx.send(said);
    Some(said)
}

fn enroll_device(
    ledger: &mut Ledger,
    peers: &Peers,
    directory: &Directory<'_>,
    from: TaskId,
    enroll: hub_api::Enroll,
) -> DmaSaid {
    // 发现结果只接受**Control 登记的那一台 PCI 实例**（安装策略，不看自报）。
    if peers.registered("pci", Wait::AtMost(MS)) != Some(from) {
        return DmaSaid::of(fail_to_code(Fail::Denied));
    }
    let requester = enroll.requester;
    if let Err(fail) = ledger.prepare_enrollment(from, enroll) {
        return DmaSaid::of(fail_to_code(fail));
    }
    match ledger.advance_enrollment(requester, &mut &*directory) {
        Some(Ok(())) => DmaSaid::of(wire::OK),
        Some(Err(fail)) => {
            crate::debug::put(&alloc::format!(
                "hub: enrollment requester={requester} failed {fail:?} (holding)"
            ));
            DmaSaid::of(fail_to_code(Fail::Denied))
        }
        None => DmaSaid::of(fail_to_code(Fail::Unknown)),
    }
}

/// 推进所有开着的绑定：先放行构造方报来的执行实例，再逐笔推一格。
///
/// 调用方（常驻那一圈）每一格叫一次；没推进就下一格再来。
pub(crate) fn advance(ledger: &mut Ledger, peers: &Peers, executions: &mut super::execution::Executions) {
    let gone = ledger.owner_gone(sweep::alive);
    if gone > 0 {
        crate::debug::put(&alloc::format!(
            "hub: {gone} binding(s) closing (owner gone)"
        ));
    }
    let serials: Vec<u64> = ledger
        .open_bindings_all()
        .map(|binding| binding.id.serial)
        .collect();
    for serial in serials {
        if let Some(task) = ledger.binding(serial)
            .filter(|binding| binding.state == BindingState::Closing)
            .map(|binding| binding.execution)
        {
            executions.closed(ledger, task);
        }
        step(ledger, peers, executions, serial);
    }
}

/// 推一格。
fn step(
    ledger: &mut Ledger,
    peers: &Peers,
    executions: &mut super::execution::Executions,
    serial: u64,
) {
    let Some(binding) = ledger.binding(serial) else {
        return;
    };
    let (state, step) = (binding.state, binding.step);
    match state {
        BindingState::Preparing => match step {
            Step::Place => place(ledger, peers, serial),
            Step::Attach => attach(ledger, peers, serial),
            Step::Acquire => acquire(ledger, peers, serial),
            Step::Publish => publish(ledger, peers, serial),
            Step::Enable => enable(ledger, peers, serial),
            _ => {}
        },
        BindingState::Closing => match step {
            // Control's per-instance target is the only proof that crosses StopCpu.
            Step::StopCpu => {}
            Step::Invalidate => invalidate(ledger, peers, serial),
            Step::Disable => disable(ledger, peers, serial),
            Step::DeviceStop => device_stop(ledger, peers, serial),
            Step::Quiesce => quiesce(ledger, peers, serial),
            Step::Release => release(ledger, peers, serial),
            Step::Detach => detach(ledger, peers, serial),
            Step::Unplace => unplace(ledger, peers, executions, serial),
            _ => finish(ledger, executions, serial),
        },
        BindingState::Active | BindingState::Closed => {}
    }
}

/// 记一笔失败并留在原步（重试同一责任）。
fn note(ledger: &mut Ledger, serial: u64, fail: Failure) {
    let first = ledger
        .binding(serial)
        .and_then(|binding| binding.failure)
        .is_none();
    if first {
        crate::debug::put(&alloc::format!(
            "hub: binding {serial} step failed party={} op={} code={} (holding)",
            fail.party,
            fail.op,
            fail.code,
        ));
    }
    ledger.note_failure(serial, fail);
}

fn finish(
    ledger: &mut Ledger,
    executions: &mut super::execution::Executions,
    serial: u64,
) {
    let requester = ledger
        .binding(serial)
        .map(|binding| binding.device.requester)
        .unwrap_or(0);
    let execution = ledger.binding(serial).map(|binding| binding.execution);
    ledger.finish(serial);
    if let Some(execution) = execution {
        executions.closed(ledger, execution);
    }
    crate::debug::put(&alloc::format!(
        "hub: binding {serial} closed and released device {requester}"
    ));
}

fn next(ledger: &mut Ledger, serial: u64, step: Step) {
    if let Some(binding) = ledger.binding_mut(serial) {
        binding.step = step;
    }
}

fn place(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    let Some(binding) = ledger.binding(serial) else {
        return;
    };
    let (requester, recovery) = (binding.device.requester, binding.recovery);
    let said = match peers.pci.call(Wait::AtMost(MS), |back| {
        pci::Wire::Place(pci::Place {
            op: pci::PLACE,
            serial,
            requester,
            recovery,
            bar: pci::BAR_ANY,
            back,
        })
    }) {
        Ok(said) => said,
        Err(_) => {
            note(ledger, serial, failure(PARTY_PCI, pci::PLACE, 0));
            return;
        }
    };
    if said.status != wire::OK {
        note(ledger, serial, failure(PARTY_PCI, pci::PLACE, said.status));
        return;
    }
    if let Some(binding) = ledger.binding_mut(serial) {
        binding.pages = said.pages;
        binding.bar = said.bar;
        binding.caps = (said.common, said.notify, said.multiplier, said.device);
        binding.step = Step::Attach;
    }
}

fn attach(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    let Some(binding) = ledger.binding(serial) else {
        return;
    };
    let (execution, bar, pages) = (binding.execution, binding.bar, binding.pages);
    let (common, notify, multiplier, device) = binding.caps;
    let said = match peers.recovery.call(Wait::AtMost(MS), |back| {
        recovery::Wire::Attach(recovery::Attach {
            op: recovery::ATTACH,
            serial,
            execution,
            bar,
            pages,
            common,
            notify,
            multiplier,
            device,
            back,
        })
    }) {
        Ok(said) => said,
        Err(_) => {
            note(ledger, serial, failure(PARTY_RECOVERY, recovery::ATTACH, 0));
            return;
        }
    };
    if said.status != wire::OK {
        note(
            ledger,
            serial,
            failure(PARTY_RECOVERY, recovery::ATTACH, said.status),
        );
        return;
    }
    if let Some(binding) = ledger.binding_mut(serial) {
        binding.step = Step::Acquire;
    }
}

fn acquire(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    let Some(binding) = ledger.binding(serial) else {
        return;
    };
    let requester = binding.device.requester;
    let execution = binding.execution;
    let iova = match dma::iova_base(serial) {
        Some(iova) => iova,
        None => {
            note(ledger, serial, failure(PARTY_HUB, iommu::ACQUIRE, 0));
            return;
        }
    };
    let access = Access::FETCH_STORE.bits().bits();
    let said = match peers.iommu.call(Wait::AtMost(MS), |back| {
        iommu::Wire::Acquire(iommu::Acquire {
            op: iommu::ACQUIRE,
            serial,
            requester,
            execution,
            iova,
            access,
            back,
        })
    }) {
        Ok(said) => said,
        Err(_) => {
            note(ledger, serial, failure(PARTY_IOMMU, iommu::ACQUIRE, 0));
            return;
        }
    };
    if said.status != wire::OK {
        // 执行方还没把窗交到 IOMMU 表里 = 正常等下一步，不算失败。
        if said.status != iommu::fail_to_code(Some(iommu::Fail::NotReady)) {
            note(
                ledger,
                serial,
                failure(PARTY_IOMMU, iommu::ACQUIRE, said.status),
            );
        }
        return;
    }
    if let Some(binding) = ledger.binding_mut(serial) {
        binding.step = Step::Publish;
    }
}

fn publish(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    match iommu_step(peers, serial, iommu::PUBLISH, Close::Normal) {
        Ok(said) if said.status == wire::OK && said.pending == 0 => {
            if let Some(binding) = ledger.binding_mut(serial) {
                binding.step = Step::Enable;
            }
        }
        Ok(said) if said.status == wire::OK => {}
        Ok(said) => note(
            ledger,
            serial,
            failure(PARTY_IOMMU, iommu::PUBLISH, said.status),
        ),
        Err(fail) => note(ledger, serial, fail),
    }
}

fn enable(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    let Some(binding) = ledger.binding(serial) else {
        return;
    };
    let execution = binding.execution;
    let (pages, caps, iova) = (
        binding.pages,
        binding.caps,
        dma::iova_base(serial).unwrap_or(0),
    );
    let said = match peers.pci.call(Wait::AtMost(MS), |back| {
        pci::Wire::Enable(pci::Toggle {
            op: pci::ENABLE,
            serial,
            back,
        })
    }) {
        Ok(said) => said,
        Err(_) => {
            note(ledger, serial, failure(PARTY_PCI, pci::ENABLE, 0));
            return;
        }
    };
    if said.status != wire::OK {
        note(ledger, serial, failure(PARTY_PCI, pci::ENABLE, said.status));
        return;
    }
    // 放行业务：把设备坐标推给执行方（它按记号认回 BAR 与 CPU 窗）。
    // **发送成功才算放行**：推不进去就留在这一步重试，不把 Active 先标上。
    let coords = dma::Coords {
        pages,
        common: caps.0,
        notify: caps.1,
        multiplier: caps.2,
        device: caps.3,
        iova,
    };
    let mut buffer = [0u8; dma::Coords::LEN];
    let stored = coords.store(&mut buffer);
    let sent = match (stored, peers.release_business(execution, serial)) {
        (Some(len), Ok(writer)) => Hole::from_raw(writer)
            .push(&buffer[..len], Wait::AtMost(MS))
            .is_ok(),
        _ => false,
    };
    if !sent {
        note(ledger, serial, failure(PARTY_HUB, pci::ENABLE, 0));
        return;
    }
    if let Some(binding) = ledger.binding_mut(serial) {
        binding.state = BindingState::Active;
        binding.step = Step::StopCpu;
    }
    crate::debug::put(&alloc::format!(
        "hub: binding {serial} active (business released) iova={iova:#x}"
    ));
}

fn iommu_step(peers: &Peers, serial: u64, op: u8, reason: Close) -> Result<iommu::Said, Failure> {
    peers
        .iommu
        .call(Wait::AtMost(MS), |back| match op {
            iommu::PUBLISH => iommu::Wire::Publish(iommu::Step {
                op,
                serial,
                reason: 0,
                back,
            }),
            iommu::INVALIDATE => iommu::Wire::Invalidate(iommu::Step {
                op,
                serial,
                reason: 0,
                back,
            }),
            iommu::QUIESCE => iommu::Wire::Quiesce(iommu::Step {
                op,
                serial,
                reason: 0,
                back,
            }),
            _ => iommu::Wire::Release(iommu::Step {
                op,
                serial,
                reason: reason.code(),
                back,
            }),
        })
        .map_err(|_| failure(PARTY_IOMMU, op, 0))
}

fn invalidate(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    match iommu_step(peers, serial, iommu::INVALIDATE, Close::Normal) {
        Ok(said) if said.status == wire::OK && said.pending == 0 => {
            next(ledger, serial, Step::Disable)
        }
        Ok(said) if said.status == wire::OK => {}
        Ok(said) => note(
            ledger,
            serial,
            failure(PARTY_IOMMU, iommu::INVALIDATE, said.status),
        ),
        Err(fail) => note(ledger, serial, fail),
    }
}

fn disable(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    let said = match peers.pci.call(Wait::AtMost(MS), |back| {
        pci::Wire::Disable(pci::Toggle {
            op: pci::DISABLE,
            serial,
            back,
        })
    }) {
        Ok(said) => said,
        Err(_) => {
            note(ledger, serial, failure(PARTY_PCI, pci::DISABLE, 0));
            return;
        }
    };
    if said.status != wire::OK {
        note(
            ledger,
            serial,
            failure(PARTY_PCI, pci::DISABLE, said.status),
        );
        return;
    }
    next(ledger, serial, Step::DeviceStop);
}

fn device_stop(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    let said = match peers.recovery.call(Wait::AtMost(MS), |back| {
        recovery::Wire::Stop(recovery::Step {
            op: recovery::STOP,
            serial,
            back,
        })
    }) {
        Ok(said) => said,
        Err(_) => {
            note(ledger, serial, failure(PARTY_RECOVERY, recovery::STOP, 0));
            return;
        }
    };
    if said.status != wire::OK {
        note(
            ledger,
            serial,
            failure(PARTY_RECOVERY, recovery::STOP, said.status),
        );
        return;
    }
    next(ledger, serial, Step::Quiesce);
}

fn quiesce(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    match iommu_step(peers, serial, iommu::QUIESCE, Close::Normal) {
        Ok(said) if said.status == wire::OK && said.pending == 0 => {
            next(ledger, serial, Step::Release)
        }
        Ok(said) if said.status == wire::OK => {}
        Ok(said) => note(
            ledger,
            serial,
            failure(PARTY_IOMMU, iommu::QUIESCE, said.status),
        ),
        Err(fail) => note(ledger, serial, fail),
    }
}

fn release(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    let binding = ledger.binding(serial);
    let reason = binding
        .map(|binding| binding.reason)
        .unwrap_or(Close::Normal);
    match iommu_step(peers, serial, iommu::RELEASE, reason) {
        Ok(said) if said.status == wire::OK && said.pending == 0 => {
            next(ledger, serial, Step::Detach)
        }
        Ok(said) if said.status == wire::OK => {}
        Ok(said) => note(
            ledger,
            serial,
            failure(PARTY_IOMMU, iommu::RELEASE, said.status),
        ),
        Err(fail) => note(ledger, serial, fail),
    }
}

fn detach(ledger: &mut Ledger, peers: &Peers, serial: u64) {
    let said = match peers.recovery.call(Wait::AtMost(MS), |back| {
        recovery::Wire::Detach(recovery::Step {
            op: recovery::DETACH,
            serial,
            back,
        })
    }) {
        Ok(said) => said,
        Err(_) => {
            note(ledger, serial, failure(PARTY_RECOVERY, recovery::DETACH, 0));
            return;
        }
    };
    if said.status != wire::OK {
        note(
            ledger,
            serial,
            failure(PARTY_RECOVERY, recovery::DETACH, said.status),
        );
        return;
    }
    next(ledger, serial, Step::Unplace);
}

fn unplace(
    ledger: &mut Ledger,
    peers: &Peers,
    executions: &mut super::execution::Executions,
    serial: u64,
) {
    match peers.pci.call(Wait::AtMost(MS), |back| {
        pci::Wire::Release(pci::Toggle {
            op: pci::RELEASE,
            serial,
            back,
        })
    }) {
        Ok(said) if said.status == wire::OK => finish(ledger, executions, serial),
        Ok(said) => note(
            ledger,
            serial,
            failure(PARTY_PCI, pci::RELEASE, said.status),
        ),
        Err(_) => note(ledger, serial, failure(PARTY_PCI, pci::RELEASE, 0)),
    }
}
