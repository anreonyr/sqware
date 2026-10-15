#![no_std]
#![no_main]

use ::core::arch::global_asm;
use ::core::sync::atomic::AtomicUsize;

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    la   t0, PER_HART",
    "    slli t1, a0, 6",
    "    add  tp, t0, t1",
    "    csrc sstatus, 2",
    "    la   sp, _kernel_edge",
    "    ld   t0, _canary",
    "    sd   t0, 0(sp)",
    "    la   t0, _stack",
    "    ld   t0, 0(t0)",
    "    add  sp, sp, t0",
    "    la   t0, BOOT_DTP", // ← bin 那一份没有这两行
    "    sd   a1, 0(t0)",    //   设备树指针（embedded-test 的 `main` 不收参数）
    "    tail main",         //   `main` = embedded-test 导出的那一个
);

#[unsafe(no_mangle)]
static BOOT_DTP: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::BOOT_DTP;
    use ::core::sync::atomic::Ordering;

    #[init]
    fn init() {
        kernel::testing_mode();
        kernel::entry::boot::init(BOOT_DTP.load(Ordering::Relaxed))
            .unwrap_or_else(|error| kernel::entry::boot::fail(error));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn unit_scan_and_receipts() {
        kernel::entry::check::syscall::unit_scan_and_receipts();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn boot_error_reports_without_allocation() {
        kernel::entry::check::boot_errors();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn backtrace_chain_scan_and_gaps() {
        kernel::entry::check::backtrace::walk();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn lock_dependency_check_is_armed() {
        kernel::entry::check::depend::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn spare_budget() {
        kernel::entry::check::spare::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn kernel_memory_layout() {
        kernel::entry::check::pagetable::kernel_layout();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn space_reservation_layout() {
        kernel::entry::check::pagetable::space_reservation_layout();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_views_and_primitives() {
        kernel::entry::check::nest::nest_views();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_invalidation_failure() {
        kernel::entry::check::nest_failure::invalidation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_placement() {
        kernel::entry::check::memory::placement();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_view_identity() {
        kernel::entry::check::memory::view_identity();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_access_wire() {
        kernel::entry::check::memory::access_wire();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_source_sharing() {
        kernel::entry::check::memory::source_sharing();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_lifecycle() {
        kernel::entry::check::syscall::lifecycle();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_retention() {
        kernel::entry::check::syscall::retention();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_attach_detach_access() {
        kernel::entry::check::syscall::attach_detach_access();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_gate_interleaving() {
        kernel::entry::check::syscall::gate_interleaving();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pole_mapping_and_creation_authority() {
        kernel::entry::check::pole::lifecycle();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pole_source_window_and_active_shape() {
        kernel::entry::check::pole::source_window_and_active_shape();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pole_device_region_borrow_and_access() {
        kernel::entry::check::pole::region();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_pole_source_wire() {
        kernel::entry::check::syscall::memory_pole();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_control_and_task_death() {
        kernel::entry::check::syscall::control_and_death();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_branch_revocation_and_retention() {
        kernel::entry::check::syscall::branches_and_retention();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn external_retention_orders_real_frames_and_isolation() {
        kernel::entry::check::syscall::external_retention();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_preparation_failures() {
        kernel::entry::check::syscall::preparation_failures();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn heap_starts_after_image_bss() {
        kernel::entry::check::pagetable::image_dynamic_region();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn interval_tree_queries_and_splits() {
        kernel::entry::check::pagetable::interval_index();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn large_page_mapping_and_splits() {
        kernel::entry::check::pagetable::large_pages();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pagetable_recycle() {
        kernel::entry::check::pagetable::pagetable();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn flat_source_model() {
        kernel::entry::check::flat::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn frame_sharing_and_recycle() {
        kernel::entry::check::frame::sharing();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn task_preparation_rollback() {
        kernel::entry::check::task::preparation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn branch_control_and_results() {
        kernel::entry::check::task::branch_control();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn first_spawn_atomic_commit() {
        kernel::entry::check::task::construction();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn construction_narrow_and_cancel() {
        kernel::entry::check::task::cancellation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn grant_authority_tracks_actual_references() {
        kernel::entry::check::frame::authority();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn retirement_waits_for_eviction() {
        kernel::entry::check::frame::retirement();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn stress_allocator() {
        kernel::entry::check::stress::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn shell_primitives() {
        kernel::entry::check::shell::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_form() {
        kernel::entry::check::permit::form();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_members() {
        kernel::entry::check::permit::members();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_fanout() {
        kernel::entry::check::permit::fanout();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_subs() {
        kernel::entry::check::permit::subs();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_references() {
        kernel::entry::check::permit::references();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_order() {
        kernel::entry::check::permit::order();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_badge() {
        kernel::entry::check::permit::badge();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn scheduler_ready_and_steal() {
        kernel::entry::check::scheduler();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hart_multi() {
        kernel::entry::check::hart::count();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_abi_and_privilege() {
        kernel::entry::check::syscall::abi_and_privilege();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_memory() {
        kernel::entry::check::syscall::memory();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_pointers() {
        kernel::entry::check::syscall::pointers();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_capability() {
        kernel::entry::check::syscall::capability();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_limits() {
        kernel::entry::check::hole::limits();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_reading_guard() {
        kernel::entry::check::hole::reading_guard();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_reservations() {
        kernel::entry::check::hole::reservations();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_queue_reuse() {
        kernel::entry::check::hole::reuse();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_discard() {
        kernel::entry::check::hole::discard_oversized();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn resource_registration_and_revocation() {
        kernel::entry::check::syscall::resource_registration();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn task_transfer_relations() {
        kernel::entry::check::syscall::transfer_relations();
    }

    #[test]
    fn scene() {
        kernel::entry::boot::start().unwrap_or_else(|error| kernel::entry::boot::fail(error));
        kernel::entry::boot::run();
    }
}
