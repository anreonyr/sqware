#![no_std]
#![no_main]

use core::arch::global_asm;
use core::sync::atomic::AtomicUsize;

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
    use core::sync::atomic::Ordering;

    #[init]
    fn init() {
        kernel::testing_mode();
        kernel::init(BOOT_DTP.load(Ordering::Relaxed))
            .unwrap_or_else(|error| kernel::boot::fail(error));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn unit_scan_and_receipts() {
        kernel::health::syscall::unit_scan_and_receipts();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn boot_error_reports_without_allocation() {
        kernel::health::boot_errors();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn spare_budget() {
        kernel::health::spare::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn kernel_memory_layout() {
        kernel::health::pagetable::kernel_layout();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn space_reservation_layout() {
        kernel::health::pagetable::space_reservation_layout();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn manager_tree_mapping() {
        kernel::health::tree::manager_tree();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn manager_tree_invalidation_failure() {
        kernel::health::tree_failure::invalidation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pole_mapping_and_creation_authority() {
        kernel::health::pole::lifecycle();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn heap_starts_after_image_bss() {
        kernel::health::pagetable::image_dynamic_region();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn interval_tree_queries_and_splits() {
        kernel::health::pagetable::interval_index();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn large_page_mapping_and_splits() {
        kernel::health::pagetable::large_pages();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pagetable_recycle() {
        kernel::health::pagetable::pagetable();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn frame_sharing_and_recycle() {
        kernel::health::frame::sharing();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn task_preparation_rollback() {
        kernel::health::task::preparation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn branch_control_and_results() {
        kernel::health::task::branch_control();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn first_spawn_atomic_commit() {
        kernel::health::task::construction();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn construction_narrow_and_cancel() {
        kernel::health::task::cancellation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn grant_authority_tracks_actual_references() {
        kernel::health::frame::authority();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn retirement_waits_for_eviction() {
        kernel::health::frame::retirement();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn stress_allocator() {
        kernel::health::stress::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn shell_primitives() {
        kernel::health::shell::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_form() {
        kernel::health::permit::form();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_members() {
        kernel::health::permit::members();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_fanout() {
        kernel::health::permit::fanout();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_subs() {
        kernel::health::permit::subs();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_references() {
        kernel::health::permit::references();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_order() {
        kernel::health::permit::order();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn permit_badge() {
        kernel::health::permit::badge();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn scheduler_ready_and_steal() {
        kernel::health::scheduler();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hart_multi() {
        kernel::health::hart::count();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_abi_and_privilege() {
        kernel::health::syscall::abi_and_privilege();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_memory() {
        kernel::health::syscall::memory();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_pointers() {
        kernel::health::syscall::pointers();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_capability() {
        kernel::health::syscall::capability();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_limits() {
        kernel::health::hole::limits();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_reading_guard() {
        kernel::health::hole::reading_guard();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_reservations() {
        kernel::health::hole::reservations();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_queue_reuse() {
        kernel::health::hole::reuse();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn hole_discard() {
        kernel::health::hole::discard_oversized();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn resource_registration_and_revocation() {
        kernel::health::syscall::resource_registration();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn task_transfer_relations() {
        kernel::health::syscall::transfer_relations();
    }

    #[test]
    fn scene() {
        kernel::boot::init().unwrap_or_else(|error| kernel::boot::fail(error));
        kernel::boot::run();
    }
}
