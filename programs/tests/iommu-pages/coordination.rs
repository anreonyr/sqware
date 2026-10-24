//! Actual Core/Holds/queue code against retained-page and MMIO fixtures; no DMA hardware.
use crate::{PieToken, TaskId, api::Fail, core_fixture, iommu_core::{AcquireRequest, Core},
    platform::Window, queue::FENCE_DATA, regs, state};
use schedule::Progress::{Done, Pending};

fn start() -> Core {
    state::STATE.with_borrow_mut(|state| { state.data_alive = true; state.window_serial = 1; });
    Core::start(Window { token: PieToken::mint(1) }, PieToken::mint(7)).unwrap()
}

fn request(serial: u64) -> AcquireRequest {
    AcquireRequest { serial, requester: 8, execution: TaskId::new(9),
        iova: hub_api::iova_base(serial).unwrap(), access: crate::Access::FETCH_STORE.bits().bits() }
}

fn finish(core: &mut Core, serial: u64) {
    assert_eq!(core.quiesce(serial), Ok(Done));
    assert_eq!(core.release(serial, hub_api::Close::Normal.code()), Ok(Done));
    assert_eq!(core.held(), 0);
    assert_eq!(state::counts(), (1, 1, 1));
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn acquired_only_close_never_submits_a_fence_and_releases_its_reference() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    assert_eq!(core.held(), 2);
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    finish(&mut core, 1);
}

#[test]
fn command_queue_enable_failure_closes_unpublished_data_without_an_unexecutable_fence() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    state::STATE.with_borrow_mut(|state| state.blocked_queue = Some(regs::CQCSR));
    assert_eq!(core.publish(1), Err(Fail::Unavailable));
    assert_ne!(regs::read32(core.base(), regs::FQCSR) & regs::QUEUE_ON, 0);
    assert_eq!(regs::read32(core.base(), regs::CQCSR) & regs::QUEUE_ON, 0);
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    assert_eq!(core.held(), 2, "quiesce must still stop the partially raised queues");
    finish(&mut core, 1);
}

#[test]
fn fault_queue_enable_failure_also_closes_without_publishing_or_submitting_commands() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    state::STATE.with_borrow_mut(|state| state.blocked_queue = Some(regs::FQCSR));
    assert_eq!(core.publish(1), Err(Fail::Unavailable));
    assert_eq!(regs::read32(core.base(), regs::CQCSR), 0);
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    assert_eq!(core.held(), 2);
    finish(&mut core, 1);
}

#[test]
fn failed_engine_start_closes_partial_unpublished_pool_resources() {
    for operation in ["pole", "acquire", "attach", "handle", "traverse"] {
        state::fail(Some(operation));
        assert!(Core::start(Window { token: PieToken::mint(1) }, PieToken::mint(7)).is_err());
        assert_eq!(state::counts(), (0, 0, 0), "partial startup at {operation}");
        assert_eq!(regs::read64(core_fixture::base(), regs::DDTP), regs::DDTP_MODE_OFF);
        assert_eq!(regs::read32(core_fixture::base(), regs::CQT), 0);
    }
    state::fail(None);
    let mut core = start();
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn closing_a_pending_publish_requires_the_separate_retire_completion() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    core_fixture::hold(true);
    assert_eq!(core.publish(1), Ok(Pending));
    assert_eq!(core.invalidate(1), Ok(Pending));
    assert_eq!(regs::read32(core.base(), regs::CQT), 6);
    core_fixture::consume(3);
    assert_eq!(core_fixture::completion(0), FENCE_DATA);
    assert_eq!(core_fixture::completion(1), 0);
    assert_eq!(core.invalidate(1), Ok(Pending), "publish completion cannot retire");
    assert_eq!(core.held(), 2);
    assert_eq!(core.quiesce(1), Err(Fail::Conflict));
    core_fixture::consume(3);
    assert_eq!(core_fixture::completion(1), FENCE_DATA + 1);
    assert_eq!(core.invalidate(1), Ok(Done));
    finish(&mut core, 1);
}

#[test]
fn full_publish_and_invalidate_wait_for_capacity_without_losing_the_reference() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    core_fixture::hold(true);
    regs::write32(core.base(), regs::CQT, 255);
    assert_eq!(core.publish(1), Ok(Pending));
    assert_eq!(core.invalidate(1), Ok(Pending));
    assert_eq!(regs::read32(core.base(), regs::CQT), 255);
    assert_eq!(core.held(), 2);
    assert_eq!(core.quiesce(1), Err(Fail::Conflict));
    core_fixture::consume(255);
    core_fixture::hold(false);
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(core_fixture::completion(1), FENCE_DATA + 1);
    finish(&mut core, 1);
}

#[test]
fn reference_release_failure_keeps_the_same_reference_without_reissuing_fences() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    assert_eq!(core.publish(1), Ok(Done));
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(core.quiesce(1), Ok(Done));
    let tail = regs::read32(core.base(), regs::CQT);
    state::fail(Some("release data"));
    for _ in 0..2 {
        assert_eq!(core.release(1, hub_api::Close::Fault.code()), Err(Fail::Unavailable));
        assert_eq!(core.held(), 2);
        assert_eq!(state::counts(), (1, 2, 1));
        assert_eq!(regs::read32(core.base(), regs::CQT), tail);
    }
    state::fail(None);
    assert_eq!(core.release(1, hub_api::Close::Fault.code()), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), tail);
    assert_eq!(state::events().iter().filter(|event| **event == "acquire data").count(), 1);
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn sole_binding_owns_its_pair_until_release_and_conflicts_consume_no_cookies() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    core.acquire(request(1)).unwrap();
    assert_eq!(core.acquire(request(2)), Err(Fail::Conflict));
    assert_eq!(state::counts(), (1, 2, 1));
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(core.quiesce(1), Ok(Done));
    assert_eq!(core.release(1, hub_api::Close::Normal.code()), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    state::STATE.with_borrow_mut(|state| { state.data_alive = true; state.window_serial = 2; });
    core.acquire(request(2)).unwrap();
    assert_eq!(core.publish(2), Ok(Done));
    assert_eq!(core_fixture::completion(0), FENCE_DATA + 2);
    assert_eq!(core.invalidate(1), Err(Fail::Unknown));
    assert_eq!(core.release(1, hub_api::Close::Fault.code()), Err(Fail::Unknown));
    assert_eq!(core.held(), 2);
    assert_eq!(core.invalidate(2), Ok(Done));
    assert_eq!(core_fixture::completion(1), FENCE_DATA + 3);
    finish(&mut core, 2);
}

#[test]
fn failed_reference_acquisition_issues_no_command_and_next_attempt_remains_closable() {
    let mut core = start();
    state::fail(Some("acquire data"));
    assert_eq!(core.acquire(request(1)), Err(Fail::Denied));
    assert_eq!(core.held(), 0);
    assert_eq!(state::counts(), (1, 1, 1));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    state::fail(None);
    core.acquire(request(1)).unwrap();
    assert_eq!(core.publish(1), Ok(Done));
    assert_eq!(core_fixture::completion(0), FENCE_DATA + 2);
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(core_fixture::completion(1), FENCE_DATA + 3);
    finish(&mut core, 1);
}

#[test]
fn requester_outside_the_actual_ddt_page_is_rejected_before_any_data_reference_or_write() {
    let mut core = start();
    assert_eq!(crate::context::offset(63), Some(4032));
    assert_eq!(crate::context::offset(64), None);
    for requester in [64, 255, u16::MAX as u32, u32::MAX] {
        let mut request = request(1);
        request.requester = requester;
        assert_eq!(core.acquire(request), Err(Fail::Denied));
        assert_eq!(core.held(), 0);
        assert_eq!(state::counts(), (1, 1, 1));
        assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    }
    let mut request = request(1);
    request.requester = 63;
    core.acquire(request).unwrap();
    assert_eq!(core.publish(1), Ok(Done));
    assert_eq!(core.invalidate(1), Ok(Done));
    finish(&mut core, 1);
}

#[test]
fn frame_release_rechecks_the_actual_retire_cookie_while_ddtp_is_still_on() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    assert_eq!(core.publish(1), Ok(Done));
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(core.quiesce(1), Ok(Done));
    let cookie = core_fixture::completion(1);
    assert_eq!(cookie, FENCE_DATA + 1);
    assert_ne!(regs::read64(core.base(), regs::DDTP) & 0xf, regs::DDTP_MODE_OFF);
    core_fixture::complete(1, FENCE_DATA);
    assert_eq!(core.release(1, hub_api::Close::Normal.code()), Err(Fail::Unavailable));
    assert_eq!(core.held(), 2);
    assert_eq!(state::counts(), (1, 2, 1));
    core_fixture::complete(1, cookie);
    assert_eq!(core.release(1, hub_api::Close::Normal.code()), Ok(Done));
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn published_frames_need_the_retire_cookie_even_when_globally_off_and_other_stop_checks_hold() {
    let mut core = start();
    core.acquire(request(1)).unwrap();
    assert_eq!(core.publish(1), Ok(Done));
    assert_eq!(core.invalidate(1), Ok(Done));
    assert_eq!(core.quiesce(1), Ok(Done));
    let cookie = core_fixture::completion(1);
    regs::write64(core.base(), regs::DDTP, regs::DDTP_MODE_OFF);
    let ddt = state::STATE.with_borrow(|state| state.view_address) as *mut u64;
    for word in 0..8 {
        unsafe { core::ptr::write_volatile(ddt.add(8 * 8 + word), 1) };
        assert_eq!(core.release(1, hub_api::Close::Normal.code()), Err(Fail::Unavailable));
        assert_eq!(core.held(), 2);
        unsafe { core::ptr::write_volatile(ddt.add(8 * 8 + word), 0) };
    }
    for (csr, value) in [
        (regs::CQCSR, regs::QUEUE_ENABLE), (regs::CQCSR, regs::QUEUE_ON),
        (regs::CQCSR, regs::QUEUE_BUSY), (regs::CQCSR, regs::QUEUE_CMD_ILL),
        (regs::FQCSR, regs::QUEUE_MEM_FAULT), (regs::PQCSR, regs::QUEUE_ON),
    ] {
        regs::write32(core.base(), csr, value);
        assert_eq!(core.release(1, hub_api::Close::Normal.code()), Err(Fail::Unavailable));
        assert_eq!(core.held(), 2);
        assert_eq!(state::counts(), (1, 2, 1));
        regs::write32(core.base(), csr, 0);
    }
    core_fixture::complete(1, 0);
    assert_eq!(core.release(1, hub_api::Close::Normal.code()), Err(Fail::Unavailable));
    assert_eq!(core.held(), 2);
    assert_eq!(state::counts(), (1, 2, 1));
    core_fixture::complete(1, cookie);
    assert_eq!(core.release(1, hub_api::Close::Normal.code()), Ok(Done));
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}
