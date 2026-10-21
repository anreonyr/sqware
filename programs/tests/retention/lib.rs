//! Hub 那一本账上的 **DMA 安全绑定**：不因主人死亡直接变为可重绑；
//! 同时保留旧的**持有账宿主安全套件**（`accessor` / `ordering` / `platform` /
//! `references` / `protocol`）——那套用例直接编译生产的 `driver/iommu/hold`，
//! 而新 IOMMU 核心正是用这本账（见 `driver/iommu/core.rs`），不是第二套死机制。
#![allow(dead_code)]
extern crate alloc;

#[cfg(test)]
#[path = "../../src/driver/iommu/hold/mod.rs"]
mod retention;

#[cfg(test)]
#[path = "../../src/service/hub/core/mod.rs"]
mod hub_core;

#[cfg(test)]
mod accessor;
#[cfg(test)]
mod ordering;
#[cfg(test)]
mod platform;
#[cfg(test)]
mod protocol;
#[cfg(test)]
mod references;
#[cfg(test)]
mod execution;
#[cfg(test)]
mod enrollment;

#[cfg(test)]
mod binding {
    use env::{PieToken, TaskId, TeamId};
    use hub_api::{BindingState, Fail};

    use crate::hub_core::{ClaimRequest, Entry, ExecutionStep, Ledger};

    const HUB: usize = 1;
    const PCI: usize = 2;
    const RECOVERY: usize = 3;

    fn ledger() -> Ledger {
        let mut ledger = Ledger::new().with_hub(TaskId::new(HUB));
        ledger
            .enroll(Entry {
                name: "virtio-blk-pci".into(),
                class: "virtio-blk".into(),
                line: 0,
                page: PieToken::NONE,
                door: PieToken::NONE,
                dma: true,
                requester: 0,
                pci: TaskId::new(PCI),
            })
            .expect("device enrollment");
        ledger
    }

    fn claim(ledger: &mut Ledger, from: usize, id: u64) -> Result<u64, Fail> {
        let task = TaskId::new(from);
        if ledger.execution(task).is_none() && ledger.launch(task, u64::MAX).is_ok() {
            let execution = ledger.execution(task).unwrap();
            execution.team = Some(TeamId::new(from));
            execution.step = ExecutionStep::Running;
        }
        ledger
            .claim(ClaimRequest {
                from: TaskId::new(from),
                id,
                requester: 0,
                sensor: PieToken::mint(from),
                recovery: TaskId::new(RECOVERY),
            })
            .map(|id| id.serial)
    }

    #[test]
    fn owner_death_closes_but_does_not_free_the_binding() {
        let mut ledger = ledger();
        let serial = claim(&mut ledger, 9, 1).expect("first claim");
        assert_eq!(ledger.open_bindings(0), 1);
        // 活绑定期：另一个执行实例不能认领同一台。
        assert_eq!(claim(&mut ledger, 10, 1), Err(Fail::Taken));
        // 主人死亡：只转收场（状态变了、责任还在）。
        assert_eq!(ledger.owner_gone(|_| false), 1);
        assert_eq!(
            ledger.binding(serial).map(|binding| binding.state),
            Some(BindingState::Closing)
        );
        assert_eq!(ledger.open_bindings(0), 1);
        // 收场责任未结清，不允许重绑。
        assert_eq!(ledger.open_bindings(0), 1);
        assert_eq!(claim(&mut ledger, 11, 1), Err(Fail::Taken));
        // 真结清之后才可重绑，且绑定号不复用。
        ledger.finish(serial);
        assert_eq!(ledger.open_bindings(0), 0);
        let again = claim(&mut ledger, 11, 1).expect("rebind after settlement");
        assert_ne!(again, serial);
    }

    #[test]
    fn claim_ids_are_idempotent_and_stale_ids_do_not_revive() {
        let mut ledger = ledger();
        let serial = claim(&mut ledger, 9, 7).expect("first claim");
        // 同调用者同编号同参：返原绑定。
        assert_eq!(claim(&mut ledger, 9, 7), Ok(serial));
        // 同编号异参：冲突。
        assert_eq!(
            ledger.claim(ClaimRequest {
                from: TaskId::new(9),
                id: 7,
                requester: 1,
                sensor: PieToken::mint(9),
                recovery: TaskId::new(RECOVERY),
            }),
            Err(Fail::Conflict)
        );
        // 旧编号不得复活。
        assert_eq!(claim(&mut ledger, 9, 6), Err(Fail::Conflict));
    }

    #[test]
    fn ordinary_vacate_does_not_touch_a_dma_binding() {
        let mut ledger = ledger();
        let serial = claim(&mut ledger, 9, 1).expect("first claim");
        // 普通设备账那一手（主人没了就空格）不碰 DMA 绑定那一笔。
        assert_eq!(ledger.vacate(|_| false), 0);
        assert_eq!(
            ledger.binding(serial).map(|binding| binding.state),
            Some(BindingState::Preparing)
        );
        assert!(ledger.device(0).is_some());
    }

    #[test]
    fn close_is_idempotent_and_fault_dominates() {
        let mut ledger = ledger();
        let serial = claim(&mut ledger, 9, 1).expect("first claim");
        assert_eq!(
            ledger.close(serial, hub_api::Close::Normal),
            Ok(BindingState::Closing)
        );
        assert_eq!(
            ledger.close(serial, hub_api::Close::Fault),
            Ok(BindingState::Closing)
        );
        ledger.finish(serial);
        assert_eq!(
            ledger.close(serial, hub_api::Close::Fault),
            Ok(BindingState::Closed)
        );
    }

    #[test]
    fn settlement_permanently_closes_admission() {
        let mut ledger = ledger();
        let serial = claim(&mut ledger, 9, 1).expect("first claim");
        assert_eq!(ledger.settle(), 2);
        assert_eq!(claim(&mut ledger, 9, 1), Ok(serial));
        assert_eq!(claim(&mut ledger, 10, 1), Err(Fail::Denied));
        ledger.finish(serial);
        assert_eq!(ledger.settle(), 1);
        ledger.execution(TaskId::new(9)).unwrap().step = ExecutionStep::Done;
        assert_eq!(ledger.settle(), 0);
        assert_eq!(claim(&mut ledger, 10, 1), Err(Fail::Denied));
    }

    #[test]
    fn launch_replay_keeps_deadline_and_settlement_closes_new_launches() {
        let mut ledger = ledger();
        let task = TaskId::new(9);
        assert_eq!(ledger.launch(task, 100), Ok(()));
        assert_eq!(ledger.launch(task, 200), Ok(()));
        assert_eq!(ledger.execution(task).unwrap().deadline, 100);
        assert_eq!(ledger.settle(), 1);
        assert_eq!(ledger.launch(task, 300), Ok(()));
        assert_eq!(ledger.execution(task).unwrap().deadline, 100);
        assert_eq!(
            ledger.launch(TaskId::new(10), 300),
            Err(Fail::Denied)
        );
        assert!(ledger.execution(TaskId::new(10)).is_none());
    }

    #[test]
    fn repeated_close_does_not_restart_the_stop_chain() {
        let mut ledger = ledger();
        let serial = claim(&mut ledger, 9, 1).expect("first claim");
        ledger.close(serial, hub_api::Close::Normal).unwrap();
        ledger.binding_mut(serial).unwrap().step = crate::hub_core::Step::Release;
        assert_eq!(ledger.owner_gone(|_| false), 0);
        ledger.close(serial, hub_api::Close::Fault).unwrap();
        assert_eq!(ledger.settle(), 2);
        assert_eq!(ledger.execution(TaskId::new(9)).unwrap().step, ExecutionStep::Stop);
        let binding = ledger.binding(serial).unwrap();
        assert_eq!(binding.step, crate::hub_core::Step::Release);
        assert_eq!(binding.reason, hub_api::Close::Fault);
    }

    #[test]
    fn closed_history_is_bounded_and_evicted_claims_cannot_revive() {
        let mut ledger = ledger();
        let first = claim(&mut ledger, 9, 1).expect("first claim");
        ledger.finish(first);
        for id in 2..=32 {
            let serial = claim(&mut ledger, 9, id).unwrap();
            ledger.finish(serial);
        }
        assert_eq!(claim(&mut ledger, 9, 1), Ok(first));
        let next = claim(&mut ledger, 9, 33).unwrap();
        assert_eq!(next, 33);
        ledger.finish(next);
        assert_eq!(claim(&mut ledger, 9, 1), Err(Fail::Conflict));
        assert!(ledger.binding(first).is_none());
        assert_eq!(claim(&mut ledger, 9, 33), Ok(next));
        assert_eq!(ledger.open_bindings(0), 0);
    }

    #[test]
    fn rebindings_use_each_iova_once_until_the_actual_address_space_is_full() {
        let mut ledger = ledger();
        let mut previous = 0;
        for id in 1..hub_api::BINDING_MAX {
            let serial = claim(&mut ledger, 9, id).unwrap();
            assert_eq!(serial, id);
            let iova = hub_api::iova_base(serial).unwrap();
            assert!(iova > previous);
            previous = iova;
            ledger.finish(serial);
        }
        assert_eq!(previous + hub_api::IOVA_SPAN, hub_api::IOVA_LIMIT);
        assert_eq!(claim(&mut ledger, 9, hub_api::BINDING_MAX), Err(Fail::Full));
        assert_eq!(claim(&mut ledger, 9, hub_api::BINDING_MAX), Err(Fail::Full));
        assert!(ledger.binding(hub_api::BINDING_MAX).is_none());
        assert_eq!(claim(&mut ledger, 9, 1), Err(Fail::Conflict));
        assert_eq!(claim(&mut ledger, 9, hub_api::BINDING_MAX - 1), Ok(hub_api::BINDING_MAX - 1));
        assert_eq!((1..hub_api::BINDING_MAX).filter(|serial| ledger.binding(*serial).is_some()).count(), 32);
        assert_eq!(ledger.executions().len(), 1);
        assert_eq!(ledger.open_bindings(0), 0);
    }

    #[test]
    fn history_never_evicts_an_open_binding_or_immediately_discards_its_completion() {
        let mut ledger = ledger();
        let pinned = claim(&mut ledger, 9, 1).unwrap();
        ledger.enroll(Entry {
            name: "other".into(), class: "virtio-blk".into(), line: 0,
            page: PieToken::NONE, door: PieToken::mint(1),
            dma: true, requester: 1, pci: TaskId::new(PCI),
        }).unwrap();
        for id in 2..=100 {
            let serial = ledger.claim(ClaimRequest {
                from: TaskId::new(9), id, requester: 1,
                sensor: PieToken::mint(9), recovery: TaskId::new(RECOVERY),
            }).unwrap().serial;
            ledger.finish(serial);
            assert!(ledger.binding(pinned).unwrap().open());
        }
        ledger.finish(pinned);
        assert_eq!(ledger.binding(pinned).unwrap().state, BindingState::Closed);
        assert_eq!(claim(&mut ledger, 9, 1), Ok(pinned));
        assert_eq!(ledger.open_bindings_all().count(), 0);
        assert_eq!((1..=100).filter(|serial| ledger.binding(*serial).is_some()).count(), 32);
    }

    #[test]
    fn denied_claims_do_not_advance_the_watermark_and_retired_instances_cannot_reenter() {
        let mut ledger = ledger();
        assert_eq!(claim(&mut ledger, 9, 0), Err(Fail::Bad));
        let first = claim(&mut ledger, 9, 1).unwrap();
        assert_eq!(claim(&mut ledger, 9, 100), Err(Fail::Taken));
        ledger.finish(first);
        assert_eq!(claim(&mut ledger, 9, 2), Ok(2));
        ledger.finish(2);
        ledger.execution(TaskId::new(9)).unwrap().step = ExecutionStep::Done;
        ledger.prune_executions();
        assert!(ledger.execution(TaskId::new(9)).is_none());
        assert_eq!(ledger.claim(ClaimRequest {
            from: TaskId::new(9), id: 3, requester: 0,
            sensor: PieToken::mint(9), recovery: TaskId::new(RECOVERY),
        }), Err(Fail::Denied));
        assert!(ledger.binding(3).is_none());
    }
}
