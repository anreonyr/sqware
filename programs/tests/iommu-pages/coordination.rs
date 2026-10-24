//! Actual Core/Holds/queue code against retained-page and MMIO fixtures; no DMA hardware.
use crate::{
    Access, PAGE_SIZE, PieToken, TaskId, acquire::AcquireRequest, api::Fail, core_fixture,
    domain_id::DomainId, iommu_core::Core, layout, platform::Window, queue::FENCE_DATA, regs,
    state,
};
use hub_api::BindingId;
use schedule::Progress::{Done, Pending};

const DRIVER: TaskId = TaskId::new(5);
const HUB: TaskId = TaskId::new(3);
const WORK: TaskId = TaskId::new(9);
const ACCESS: u32 = Access::FETCH_STORE.bits().bits();

fn core() -> Core {
    state::reset();
    Core::start(
        Window {
            token: PieToken::mint(1),
        },
        PieToken::mint(7),
    )
    .unwrap()
}

fn source(token: usize, pages: usize, serial: u64) {
    state::add_source(token, pages, WORK, crate::driver::shared::dma::window_mark(serial));
}

fn establish(core: &mut Core, requester: u32) -> DomainId {
    core.establish(DRIVER, requester).unwrap()
}

fn binding(serial: u64) -> BindingId {
    BindingId { hub: HUB, serial }
}

fn request(domain: DomainId, serial: u64, token: usize) -> AcquireRequest {
    AcquireRequest {
        domain,
        binding: binding(serial),
        execution: WORK,
        source: PieToken::mint(token),
        access: ACCESS,
    }
}

fn acquire(core: &mut Core, domain: DomainId, serial: u64, token: usize) -> Result<(), Fail> {
    core.acquire(DRIVER, request(domain, serial, token))
}

fn publish(core: &mut Core, domain: DomainId, serial: u64) -> Result<schedule::Progress, Fail> {
    core.publish(DRIVER, domain, binding(serial))
}

fn invalidate(core: &mut Core, domain: DomainId, serial: u64) -> Result<schedule::Progress, Fail> {
    core.invalidate(DRIVER, domain, binding(serial))
}

fn quiesce(core: &mut Core, domain: DomainId, serial: u64) -> Result<schedule::Progress, Fail> {
    core.quiesce(DRIVER, domain, binding(serial))
}

fn release(core: &mut Core, domain: DomainId, serial: u64) -> Result<schedule::Progress, Fail> {
    core.release(
        DRIVER,
        domain,
        binding(serial),
        hub_api::Close::Normal.code(),
    )
}

fn finish(core: &mut Core, domain: DomainId, serial: u64) {
    assert_eq!(invalidate(core, domain, serial), Ok(Done));
    assert_eq!(quiesce(core, domain, serial), Ok(Done));
    assert_eq!(release(core, domain, serial), Ok(Done));
    assert_eq!(core.held(), 0);
    assert_eq!(state::counts(), (1, 1, 1));
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

fn slot(requester: usize) -> usize {
    2 * requester
}

fn ddt_word(requester: usize, word: usize) -> u64 {
    let address = state::STATE
        .with_borrow(|state| state.view_address + layout::DDT * PAGE_SIZE)
        + (requester * 8 + word) * 8;
    unsafe { core::ptr::read_volatile(address as *const u64) }
}

fn write_ddt_word(requester: usize, word: usize, value: u64) {
    let address = state::STATE
        .with_borrow(|state| state.view_address + layout::DDT * PAGE_SIZE)
        + (requester * 8 + word) * 8;
    unsafe { core::ptr::write_volatile(address as *mut u64, value) };
}

#[test]
fn acquired_only_close_never_submits_a_fence_and_releases_its_reference() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    assert_eq!(core.held(), 2);
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    finish(&mut core, domain, 1);
}

#[test]
fn command_queue_enable_failure_closes_unpublished_data_without_an_unexecutable_fence() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    state::STATE.with_borrow_mut(|state| state.blocked_queue = Some(regs::CQCSR));
    assert_eq!(publish(&mut core, domain, 1), Err(Fail::Unavailable));
    assert_ne!(regs::read32(core.base(), regs::FQCSR) & regs::QUEUE_ON, 0);
    assert_eq!(regs::read32(core.base(), regs::CQCSR) & regs::QUEUE_ON, 0);
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    assert_eq!(
        core.held(),
        2,
        "quiesce must still stop the partially raised queues"
    );
    finish(&mut core, domain, 1);
}

#[test]
fn fault_queue_enable_failure_also_closes_without_publishing_or_submitting_commands() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    state::STATE.with_borrow_mut(|state| state.blocked_queue = Some(regs::FQCSR));
    assert_eq!(publish(&mut core, domain, 1), Err(Fail::Unavailable));
    assert_eq!(regs::read32(core.base(), regs::CQCSR), 0);
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    assert_eq!(core.held(), 2);
    finish(&mut core, domain, 1);
}

#[test]
fn failed_engine_start_closes_partial_unpublished_pool_resources() {
    state::reset();
    for operation in ["pole", "acquire", "attach", "handle", "traverse"] {
        state::fail(Some(operation));
        assert!(
            Core::start(
                Window {
                    token: PieToken::mint(1)
                },
                PieToken::mint(7)
            )
            .is_err()
        );
        assert_eq!(state::counts(), (0, 0, 0), "partial startup at {operation}");
        assert_eq!(
            regs::read64(core_fixture::base(), regs::DDTP),
            regs::DDTP_MODE_OFF
        );
        assert_eq!(regs::read32(core_fixture::base(), regs::CQT), 0);
    }
    state::fail(None);
    state::reset();
    let mut core = Core::start(
        Window {
            token: PieToken::mint(1),
        },
        PieToken::mint(7),
    )
    .unwrap();
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn startup_requires_actual_off_queues_and_no_sticky_error() {
    state::reset();
    let (base, _) = crate::raw::open(PieToken::mint(1)).unwrap();
    for (csr, value) in [
        (regs::CQCSR, regs::QUEUE_ENABLE),
        (regs::CQCSR, regs::QUEUE_ON),
        (regs::CQCSR, regs::QUEUE_BUSY),
        (regs::CQCSR, regs::QUEUE_CMD_ILL),
        (regs::FQCSR, regs::QUEUE_MEM_FAULT),
        (regs::PQCSR, regs::QUEUE_ON),
    ] {
        regs::write32(base, csr, value);
        assert!(Core::start(
            Window {
                token: PieToken::mint(1)
            },
            PieToken::mint(7)
        )
        .is_err());
        regs::write32(base, csr, 0);
    }
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn closing_a_pending_publish_requires_the_separate_retire_completion() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    core_fixture::hold(true);
    assert_eq!(publish(&mut core, domain, 1), Ok(Pending));
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Pending));
    assert_eq!(regs::read32(core.base(), regs::CQT), 6);
    core_fixture::consume(3);
    assert_eq!(core_fixture::completion(slot(8)), FENCE_DATA);
    assert_eq!(core_fixture::completion(slot(8) + 1), 0);
    assert_eq!(
        invalidate(&mut core, domain, 1),
        Ok(Pending),
        "publish completion cannot retire"
    );
    assert_eq!(core.held(), 2);
    assert_eq!(quiesce(&mut core, domain, 1), Err(Fail::Conflict));
    core_fixture::consume(3);
    assert_eq!(core_fixture::completion(slot(8) + 1), FENCE_DATA + 1);
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    finish(&mut core, domain, 1);
}

#[test]
fn full_publish_and_invalidate_wait_for_capacity_without_losing_the_reference() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    core_fixture::hold(true);
    regs::write32(core.base(), regs::CQT, 255);
    assert_eq!(publish(&mut core, domain, 1), Ok(Pending));
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Pending));
    assert_eq!(regs::read32(core.base(), regs::CQT), 255);
    assert_eq!(core.held(), 2);
    assert_eq!(quiesce(&mut core, domain, 1), Err(Fail::Conflict));
    core_fixture::consume(255);
    core_fixture::hold(false);
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(core_fixture::completion(slot(8) + 1), FENCE_DATA + 1);
    finish(&mut core, domain, 1);
}

#[test]
fn reference_release_failure_keeps_the_same_reference_without_reissuing_fences() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    assert_eq!(publish(&mut core, domain, 1), Ok(Done));
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(quiesce(&mut core, domain, 1), Ok(Done));
    let tail = regs::read32(core.base(), regs::CQT);
    state::fail(Some("release data"));
    for _ in 0..2 {
        assert_eq!(
            core.release(
                DRIVER,
                domain,
                binding(1),
                hub_api::Close::Fault.code()
            ),
            Err(Fail::Unavailable)
        );
        assert_eq!(core.held(), 2);
        assert_eq!(state::counts(), (1, 2, 1));
        assert_eq!(regs::read32(core.base(), regs::CQT), tail);
    }
    state::fail(None);
    assert_eq!(
        core.release(
            DRIVER,
            domain,
            binding(1),
            hub_api::Close::Fault.code()
        ),
        Ok(Done)
    );
    assert_eq!(regs::read32(core.base(), regs::CQT), tail);
    assert_eq!(
        state::events()
            .iter()
            .filter(|event| **event == "acquire data")
            .count(),
        1
    );
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn exact_retries_reuse_the_pair_and_a_new_binding_does_not_reset_the_cookie_issuer() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    let first = core.range(domain).unwrap();
    acquire(&mut core, domain, 1, 42).unwrap();
    assert_eq!(core.range(domain), Some(first));
    assert_eq!(state::counts(), (1, 2, 1));
    assert_eq!(acquire(&mut core, domain, 2, 42), Err(Fail::Conflict));

    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(quiesce(&mut core, domain, 1), Ok(Done));
    assert_eq!(release(&mut core, domain, 1), Ok(Done));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);

    source(43, 2, 2);
    acquire(&mut core, domain, 2, 43).unwrap();
    assert_eq!(core.range(domain).unwrap().0, first.0 + crate::domain::IOVA_STRIDE);
    assert_eq!(publish(&mut core, domain, 2), Ok(Done));
    assert_eq!(core_fixture::completion(slot(8)), FENCE_DATA + 2);
    assert_eq!(invalidate(&mut core, domain, 1), Err(Fail::Unknown));
    assert_eq!(release(&mut core, domain, 1), Err(Fail::Unknown));
    assert_eq!(core.held(), 2);
    assert_eq!(invalidate(&mut core, domain, 2), Ok(Done));
    assert_eq!(core_fixture::completion(slot(8) + 1), FENCE_DATA + 3);
    finish(&mut core, domain, 2);
}

#[test]
fn failed_reference_acquisition_issues_no_command_and_exact_retry_reuses_the_reservation() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    state::fail(Some("acquire data"));
    assert_eq!(acquire(&mut core, domain, 1, 42), Err(Fail::Denied));
    let reserved = core
        .range(domain)
        .expect("range persisted before the reference");
    assert_eq!(state::counts(), (1, 1, 1));
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    state::fail(None);
    acquire(&mut core, domain, 1, 42).unwrap();
    assert_eq!(core.range(domain), Some(reserved));
    assert_eq!(publish(&mut core, domain, 1), Ok(Done));
    assert_eq!(core_fixture::completion(slot(8)), FENCE_DATA);
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(core_fixture::completion(slot(8) + 1), FENCE_DATA + 1);
    finish(&mut core, domain, 1);
}

#[test]
fn requester_outside_the_actual_ddt_page_is_rejected_before_any_data_reference_or_write() {
    let mut core = core();
    for requester in [64, 255, u16::MAX as u32, u32::MAX] {
        assert_eq!(core.establish(DRIVER, requester), Err(Fail::Denied));
    }
    assert_eq!(crate::context::offset(63), Some(4032));
    assert_eq!(crate::context::offset(64), None);
    source(42, 2, 1);
    let domain = establish(&mut core, 63);
    acquire(&mut core, domain, 1, 42).unwrap();
    assert_eq!(publish(&mut core, domain, 1), Ok(Done));
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    finish(&mut core, domain, 1);
}

#[test]
fn frame_release_rechecks_the_actual_retire_cookie_after_engine_off() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    assert_eq!(publish(&mut core, domain, 1), Ok(Done));
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(release(&mut core, domain, 1), Err(Fail::Conflict));
    assert_eq!(quiesce(&mut core, domain, 1), Ok(Done));
    let cookie = core_fixture::completion(slot(8) + 1);
    assert_eq!(cookie, FENCE_DATA + 1);
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    core_fixture::complete(slot(8) + 1, FENCE_DATA);
    assert_eq!(release(&mut core, domain, 1), Err(Fail::Unavailable));
    assert_eq!(core.held(), 2);
    assert_eq!(state::counts(), (1, 2, 1));
    core_fixture::complete(slot(8) + 1, cookie);
    assert_eq!(release(&mut core, domain, 1), Ok(Done));
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn published_frames_need_the_retire_cookie_and_engine_readbacks_even_when_ddtp_is_off() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    assert_eq!(publish(&mut core, domain, 1), Ok(Done));
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(release(&mut core, domain, 1), Err(Fail::Conflict));
    assert_eq!(quiesce(&mut core, domain, 1), Ok(Done));
    let cookie = core_fixture::completion(slot(8) + 1);
    regs::write64(core.base(), regs::DDTP, regs::DDTP_MODE_OFF);
    for word in 0..8 {
        write_ddt_word(8, word, 1);
        assert_eq!(release(&mut core, domain, 1), Err(Fail::Unavailable));
        assert_eq!(core.held(), 2);
        assert_eq!(ddt_word(8, word), 1);
        write_ddt_word(8, word, 0);
    }
    core_fixture::complete(slot(8) + 1, 0);
    assert_eq!(release(&mut core, domain, 1), Err(Fail::Unavailable));
    assert_eq!(core.held(), 2);
    assert_eq!(state::counts(), (1, 2, 1));
    core_fixture::complete(slot(8) + 1, cookie);
    assert_eq!(release(&mut core, domain, 1), Ok(Done));
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn global_pool_release_rejects_any_remaining_queue_error_or_activity() {
    let mut core = core();
    source(42, 2, 1);
    let domain = establish(&mut core, 8);
    acquire(&mut core, domain, 1, 42).unwrap();
    assert_eq!(publish(&mut core, domain, 1), Ok(Done));
    assert_eq!(invalidate(&mut core, domain, 1), Ok(Done));
    assert_eq!(quiesce(&mut core, domain, 1), Ok(Done));
    assert_eq!(release(&mut core, domain, 1), Ok(Done));
    assert_eq!(core.held(), 0);
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    for (csr, value) in [
        (regs::CQCSR, regs::QUEUE_ENABLE),
        (regs::CQCSR, regs::QUEUE_ON),
        (regs::CQCSR, regs::QUEUE_BUSY),
        (regs::CQCSR, regs::QUEUE_CMD_ILL),
        (regs::FQCSR, regs::QUEUE_MEM_FAULT),
        (regs::PQCSR, regs::QUEUE_ON),
    ] {
        regs::write32(core.base(), csr, value);
        assert_eq!(core.shutdown(), Err("iommu: engine not off"));
        regs::write32(core.base(), csr, 0);
    }
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}
