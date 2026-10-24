//! Persistent Domain identity, private IOVA allocation and shared-engine isolation.

use crate::{
    Access, PAGE_SIZE, PieToken, TaskId, acquire::AcquireRequest, api::Fail, core_fixture,
    domain::FIRST_IOVA, domain_id::DomainId, iommu_core::Core, layout, platform::Window,
    queue::FENCE_DATA, regs, state,
};
use hub_api::BindingId;
use schedule::Progress::{Done, Pending};

const DRIVER: TaskId = TaskId::new(5);
const OTHER: TaskId = TaskId::new(6);
const HUB: TaskId = TaskId::new(3);
const OTHER_HUB: TaskId = TaskId::new(4);
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
    source_for(token, pages, serial, WORK);
}

fn source_for(token: usize, pages: usize, serial: u64, work: TaskId) {
    state::add_source(token, pages, work, crate::driver::shared::dma::window_mark(serial));
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

fn acquire_for(
    core: &mut Core,
    domain: DomainId,
    serial: u64,
    token: usize,
    execution: TaskId,
) -> Result<(), Fail> {
    core.acquire(
        DRIVER,
        AcquireRequest {
            domain,
            binding: binding(serial),
            execution,
            source: PieToken::mint(token),
            access: ACCESS,
        },
    )
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
    core.release(DRIVER, domain, binding(serial), hub_api::Close::Normal.code())
}

fn retire(core: &mut Core, domain: DomainId, serial: u64) {
    assert_eq!(invalidate(core, domain, serial), Ok(Done));
    assert_eq!(quiesce(core, domain, serial), Ok(Done));
    assert_eq!(release(core, domain, serial), Ok(Done));
}

fn ddt_word(requester: usize, word: usize) -> u64 {
    let address = state::STATE
        .with_borrow(|state| state.view_address + layout::DDT * PAGE_SIZE)
        + (requester * 8 + word) * 8;
    unsafe { core::ptr::read_volatile(address as *const u64) }
}

#[test]
fn two_domains_get_equal_numeric_iova_with_distinct_roots_and_physical_references() {
    let mut core = core();
    source(42, 2, 1);
    source(43, 3, 2);
    let a = establish(&mut core, 8);
    assert_eq!(core.establish(DRIVER, 8), Ok(a));
    let b = establish(&mut core, 9);
    acquire(&mut core, a, 1, 42).unwrap();
    acquire(&mut core, b, 2, 43).unwrap();
    let (a_base, a_bytes) = core.range(a).unwrap();
    let (b_base, b_bytes) = core.range(b).unwrap();
    assert_eq!(a_base, FIRST_IOVA);
    assert_eq!(b_base, FIRST_IOVA);
    assert_eq!(a_bytes, 2 * PAGE_SIZE as u64);
    assert_eq!(b_bytes, 3 * PAGE_SIZE as u64);
    assert_eq!(core.held(), 5);
    assert_eq!(core.shutdown(), Err("iommu: binding not settled"));

    assert_eq!(publish(&mut core, a, 1), Ok(Done));
    assert_eq!(publish(&mut core, b, 2), Ok(Done));
    assert_ne!(ddt_word(8, 0), 0);
    assert_ne!(ddt_word(9, 0), 0);
    assert_eq!(core_fixture::completion(2 * 8), FENCE_DATA);
    assert_eq!(core_fixture::completion(2 * 9), FENCE_DATA + 2);

    let a_leaf = layout::tables(8).unwrap().2;
    let b_leaf = layout::tables(9).unwrap().2;
    let a_physical = core_fixture::word(a_leaf, 0) >> 10;
    let b_physical = core_fixture::word(b_leaf, 0) >> 10;
    assert_eq!(a_physical, state::source(42).unwrap().physical >> 12);
    assert_eq!(b_physical, state::source(43).unwrap().physical >> 12);
    assert_ne!(a_physical, b_physical);

    retire(&mut core, a, 1);
    assert_eq!(ddt_word(8, 0), 0);
    assert_ne!(ddt_word(9, 0), 0);
    assert_ne!(
        regs::read64(core.base(), regs::DDTP) & 0xf,
        regs::DDTP_MODE_OFF
    );
    assert_ne!(regs::read32(core.base(), regs::CQCSR) & regs::QUEUE_ON, 0);
    assert_eq!(core.held(), 3);

    retire(&mut core, b, 2);
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    assert_eq!(regs::read32(core.base(), regs::CQCSR) & regs::QUEUE_ON, 0);
    assert_eq!(regs::read32(core.base(), regs::FQCSR) & regs::QUEUE_ON, 0);
    assert_eq!(core.held(), 0);
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn a_foreign_caller_domain_or_hub_cannot_mutate_an_owned_binding() {
    let mut core = core();
    source(42, 2, 1);
    let a = establish(&mut core, 8);
    assert_eq!(core.establish(OTHER, 8), Err(Fail::Denied));
    assert_eq!(core.acquire(OTHER, request(a, 1, 42)), Err(Fail::Denied));
    assert_eq!(core.publish(OTHER, a, binding(1)), Err(Fail::Denied));
    assert_eq!(
        core.release(OTHER, a, binding(1), hub_api::Close::Normal.code()),
        Err(Fail::Denied)
    );
    let fake = DomainId::new(TaskId::new(99), a.key());
    assert_eq!(core.acquire(DRIVER, request(fake, 1, 42)), Err(Fail::Denied));
    let unknown = DomainId::new(DRIVER, a.key() + 100);
    assert_eq!(core.acquire(DRIVER, request(unknown, 1, 42)), Err(Fail::Unknown));

    acquire(&mut core, a, 1, 42).unwrap();
    assert_eq!(publish(&mut core, a, 1), Ok(Done));
    assert_eq!(
        core.release(OTHER, a, binding(1), hub_api::Close::Normal.code()),
        Err(Fail::Denied)
    );
    assert_eq!(core.range(a).is_some(), true);
    assert_eq!(
        core.release(DRIVER, a, BindingId { hub: OTHER_HUB, serial: 1 }, 1),
        Err(Fail::Denied)
    );
    assert_eq!(invalidate(&mut core, a, 1), Ok(Done));
    assert_eq!(quiesce(&mut core, a, 1), Ok(Done));
    assert_eq!(release(&mut core, a, 1), Ok(Done));
    core.shutdown().unwrap();
}

#[test]
fn the_first_full_binding_freezes_the_hub_including_close_before_acquire() {
    let mut core = core();
    source(42, 2, 1);
    let a = establish(&mut core, 8);
    assert_eq!(release(&mut core, a, 5), Ok(Done));
    assert_eq!(acquire(&mut core, a, 5, 42), Err(Fail::Conflict));
    source(44, 2, 6);
    acquire(&mut core, a, 6, 44).unwrap();
    assert_eq!(
        core.release(
            DRIVER,
            a,
            BindingId {
                hub: OTHER_HUB,
                serial: 6
            },
            hub_api::Close::Normal.code()
        ),
        Err(Fail::Denied)
    );
    assert_eq!(publish(&mut core, a, 6), Ok(Done));
    retire(&mut core, a, 6);

    let b = establish(&mut core, 9);
    assert_eq!(
        core.release(
            DRIVER,
            b,
            BindingId {
                hub: OTHER_HUB,
                serial: 1
            },
            hub_api::Close::Normal.code()
        ),
        Ok(Done)
    );
    assert_eq!(core.acquire(DRIVER, request(b, 2, 42)), Err(Fail::Denied));
    core.shutdown().unwrap();
}

#[test]
fn an_empty_domain_release_leaves_another_domains_live_engine_and_reference_intact() {
    let mut core = core();
    let empty = establish(&mut core, 8);
    let live = establish(&mut core, 9);
    source(42, 2, 102);
    acquire(&mut core, live, 102, 42).unwrap();
    assert_eq!(publish(&mut core, live, 102), Ok(Done));
    let range = core.range(live).unwrap();
    assert_eq!(invalidate(&mut core, empty, 101), Ok(Done));
    assert_eq!(quiesce(&mut core, empty, 101), Ok(Done));
    assert_eq!(release(&mut core, empty, 101), Ok(Done));
    assert_eq!(core.range(live), Some(range));
    assert_eq!(core.held(), 2);
    assert_eq!(regs::read64(core_fixture::base(), regs::DDTP) & 0xf, regs::DDTP_MODE_1LVL);
    assert_ne!(regs::read32(core_fixture::base(), regs::CQCSR) & regs::QUEUE_ENABLE, 0);
    assert_ne!(regs::read32(core_fixture::base(), regs::FQCSR) & regs::QUEUE_ENABLE, 0);
    retire(&mut core, live, 102);
    core.shutdown().unwrap();
}

#[test]
fn an_empty_idle_domain_release_requires_the_actual_off_readback() {
    let mut core = core();
    let empty = establish(&mut core, 8);
    assert_eq!(invalidate(&mut core, empty, 101), Ok(Done));
    state::STATE.with_borrow_mut(|state| state.mmio[regs::CQCSR / 8] = u64::from(regs::QUEUE_MEM_FAULT));
    assert_eq!(release(&mut core, empty, 101), Err(Fail::Unavailable));
    state::STATE.with_borrow_mut(|state| state.mmio[regs::CQCSR / 8] = 0);
    assert_eq!(release(&mut core, empty, 101), Ok(Done));
    core.shutdown().unwrap();
}

#[test]
fn an_unknown_other_binding_cannot_close_or_advance_the_current_one() {
    let mut core = core();
    source(42, 2, 1);
    let a = establish(&mut core, 8);
    acquire(&mut core, a, 1, 42).unwrap();
    assert_eq!(release(&mut core, a, 9), Err(Fail::Unknown));
    assert_eq!(quiesce(&mut core, a, 9), Err(Fail::Unknown));
    assert_eq!(invalidate(&mut core, a, 9), Err(Fail::Unknown));
    assert_eq!(core.range(a).is_some(), true);
    assert_eq!(invalidate(&mut core, a, 1), Ok(Done));
    assert_eq!(quiesce(&mut core, a, 1), Ok(Done));
    assert_eq!(release(&mut core, a, 1), Ok(Done));
    assert_eq!(release(&mut core, a, 1), Ok(Done), "closed request is idempotent");
    core.shutdown().unwrap();
}

#[test]
fn a_new_unpublished_prepare_during_lowering_waits_off_and_never_cancels_the_down_edge() {
    let mut core = core();
    source(42, 2, 1);
    source(43, 3, 2);
    let a = establish(&mut core, 8);
    acquire(&mut core, a, 1, 42).unwrap();
    assert_eq!(publish(&mut core, a, 1), Ok(Done));
    assert_eq!(invalidate(&mut core, a, 1), Ok(Done));
    core_fixture::block_lower(regs::CQCSR, true);
    assert_eq!(quiesce(&mut core, a, 1), Ok(Pending));
    assert_eq!(core.held(), 2, "data stays held through the physical down edge");
    assert_eq!(state::counts(), (1, 2, 1));
    assert_eq!(
        regs::read32(core.base(), regs::CQCSR) & regs::QUEUE_ENABLE,
        regs::QUEUE_ENABLE
    );

    let b = establish(&mut core, 9);
    acquire(&mut core, b, 2, 43).unwrap();
    assert_eq!(publish(&mut core, b, 2), Ok(Pending));
    assert_eq!(ddt_word(9, 0), 0, "lowering must not be cancelled or raced");
    assert_ne!(
        regs::read64(core.base(), regs::DDTP) & 0xf,
        regs::DDTP_MODE_OFF
    );

    core_fixture::block_lower(regs::CQCSR, false);
    assert_eq!(quiesce(&mut core, a, 1), Ok(Done));
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    assert_eq!(release(&mut core, a, 1), Ok(Done));
    assert_eq!(publish(&mut core, b, 2), Ok(Done));
    retire(&mut core, b, 2);
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    core.shutdown().unwrap();
}

#[test]
fn exhaustion_and_overflow_do_not_consume_the_private_cursor() {
    let mut core = core();
    source(42, 2, 1);
    let a = establish(&mut core, 8);
    assert_eq!(core.range(a), None);
    core.set_next_iova(a, crate::domain::IOVA_LIMIT - PAGE_SIZE as u64);
    assert_eq!(acquire(&mut core, a, 1, 42), Err(Fail::Full));
    assert_eq!(core.range(a), None);
    assert_eq!(acquire(&mut core, a, 1, 99), Err(Fail::NotReady));
    core.set_next_iova(a, FIRST_IOVA);
    acquire(&mut core, a, 1, 42).unwrap();
    assert_eq!(core.range(a).unwrap().0, FIRST_IOVA);
    retire(&mut core, a, 1);
    source(43, 2, 2);
    core.set_next_iova(a, u64::MAX - crate::domain::IOVA_STRIDE);
    assert_eq!(acquire(&mut core, a, 2, 43), Err(Fail::Full));
    assert_eq!(core.range(a), None);
    core.shutdown().unwrap();
}

#[test]
fn a_partial_acquire_retries_the_same_range_after_the_work_leaf_is_gone() {
    let mut core = core();
    source(42, 2, 1);
    let a = establish(&mut core, 8);
    state::fail(Some("traverse"));
    assert_eq!(acquire(&mut core, a, 1, 42), Err(Fail::Unavailable));
    let reserved = core.range(a).expect("binding and range persisted");
    assert_eq!(state::counts(), (1, 2, 1));
    state::kill_source(42);
    state::fail(None);
    acquire(&mut core, a, 1, 42).unwrap();
    assert_eq!(core.range(a), Some(reserved));
    assert_eq!(state::counts(), (1, 2, 1), "no second data reference");
    assert_eq!(publish(&mut core, a, 1), Ok(Done));
    retire(&mut core, a, 1);
    core.shutdown().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn explicit_tokens_bind_to_their_actual_work_task() {
    let mut core = core();
    source_for(44, 2, 1, TaskId::new(10));
    let a = establish(&mut core, 8);
    assert_eq!(acquire(&mut core, a, 1, 44), Err(Fail::Denied));
    acquire_for(&mut core, a, 1, 44, TaskId::new(10)).unwrap();
    retire(&mut core, a, 1);
    core.shutdown().unwrap();
}

#[test]
fn invalidate_first_close_tombstones_the_exact_binding_and_later_steps_are_idempotent() {
    let mut core = core();
    source(42, 2, 5);
    let a = establish(&mut core, 8);
    assert_eq!(invalidate(&mut core, a, 5), Ok(Done));
    assert_eq!(regs::read64(core.base(), regs::DDTP), regs::DDTP_MODE_OFF);
    assert_eq!(regs::read32(core.base(), regs::CQT), 0);
    assert_eq!(core.range(a), None);
    assert_eq!(quiesce(&mut core, a, 5), Ok(Done));
    assert_eq!(release(&mut core, a, 5), Ok(Done));
    assert_eq!(invalidate(&mut core, a, 5), Ok(Done));
    assert_eq!(quiesce(&mut core, a, 5), Ok(Done));
    assert_eq!(release(&mut core, a, 5), Ok(Done));
    assert_eq!(acquire(&mut core, a, 5, 42), Err(Fail::Conflict));
    source(43, 2, 6);
    acquire(&mut core, a, 6, 43).unwrap();
    retire(&mut core, a, 6);
    core.shutdown().unwrap();
}

#[test]
fn an_unknown_id_against_a_current_binding_stays_unknown_and_does_not_advance_the_floor() {
    let mut core = core();
    source(42, 2, 1);
    source(43, 2, 2);
    let a = establish(&mut core, 8);
    acquire(&mut core, a, 1, 42).unwrap();
    assert_eq!(invalidate(&mut core, a, 2), Err(Fail::Unknown));
    assert_eq!(quiesce(&mut core, a, 2), Err(Fail::Unknown));
    assert_eq!(release(&mut core, a, 2), Err(Fail::Unknown));
    assert_eq!(core.range(a).is_some(), true);
    retire(&mut core, a, 1);
    acquire(&mut core, a, 2, 43).unwrap();
    retire(&mut core, a, 2);
    core.shutdown().unwrap();
}

#[test]
fn out_of_order_closed_binding_in_another_domain_does_not_bar_this_domain() {
    let mut core = core();
    source(42, 2, 101);
    source(43, 2, 102);
    let a = establish(&mut core, 8);
    let b = establish(&mut core, 9);
    acquire(&mut core, b, 102, 43).unwrap();
    retire(&mut core, b, 102);
    acquire(&mut core, a, 101, 42).unwrap();
    retire(&mut core, a, 101);
    core.shutdown().unwrap();
}

#[test]
fn adoption_starts_at_a_persisted_binding_even_when_its_reference_failed() {
    let mut core = core();
    source(42, 2, 1);
    let a = establish(&mut core, 8);
    let b = establish(&mut core, 9);
    state::fail(Some("acquire data"));
    assert_eq!(acquire(&mut core, a, 1, 42), Err(Fail::Denied));
    assert_eq!(core.range(a).is_some(), true);
    assert_eq!(acquire(&mut core, b, 1, 42), Err(Fail::Denied));
    assert_eq!(core.range(b), None);
    state::fail(None);
    acquire(&mut core, a, 1, 42).unwrap();
    retire(&mut core, a, 1);
    core.shutdown().unwrap();
}

#[test]
fn malformed_first_binding_does_not_pin_the_domain_or_move_the_floor() {
    let mut core = core();
    source(42, 2, 1);
    let a = establish(&mut core, 8);
    let zero_hub = BindingId {
        hub: TaskId::new(0),
        serial: 5,
    };
    assert_eq!(
        core.release(DRIVER, a, zero_hub, hub_api::Close::Normal.code()),
        Err(Fail::Denied)
    );
    assert_eq!(core.invalidate(DRIVER, a, zero_hub), Err(Fail::Denied));
    assert_eq!(core.quiesce(DRIVER, a, zero_hub), Err(Fail::Denied));
    assert_eq!(
        core.release(DRIVER, a, BindingId { hub: HUB, serial: 0 }, 1),
        Err(Fail::Denied)
    );
    assert_eq!(core.release(DRIVER, a, binding(1), 0), Err(Fail::Denied));
    assert_eq!(core.release(DRIVER, a, binding(1), 3), Err(Fail::Denied));
    let mut zero_execution = request(a, 1, 42);
    zero_execution.execution = TaskId::new(0);
    assert_eq!(core.acquire(DRIVER, zero_execution), Err(Fail::Denied));
    let mut absent_source = request(a, 1, 42);
    absent_source.source = PieToken::NONE;
    assert_eq!(core.acquire(DRIVER, absent_source), Err(Fail::Denied));
    assert_eq!(core.range(a), None);
    acquire(&mut core, a, 1, 42).unwrap();
    assert_eq!(core.range(a).unwrap().0, FIRST_IOVA);
    retire(&mut core, a, 1);
    core.shutdown().unwrap();
}

#[test]
fn one_iommu_local_token_cannot_be_adopted_twice_across_live_domains() {
    let mut core = core();
    source(42, 2, 1);
    let a = establish(&mut core, 8);
    let b = establish(&mut core, 9);
    acquire(&mut core, a, 1, 42).unwrap();
    assert_eq!(acquire(&mut core, b, 1, 42), Err(Fail::Denied));
    assert_eq!(core.range(b), None);
    retire(&mut core, a, 1);
    source(43, 2, 2);
    source(45, 2, 1);
    acquire(&mut core, b, 1, 45).unwrap();
    retire(&mut core, b, 1);
    acquire(&mut core, b, 2, 43).unwrap();
    retire(&mut core, b, 2);
    core.shutdown().unwrap();
}
