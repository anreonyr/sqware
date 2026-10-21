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
    fn nest_task_exit_without_failure() {
        kernel::entry::check::nest_exit::task_exit_without_failure();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_task_exit_with_persistent_failure() {
        kernel::entry::check::nest_exit::task_exit_with_persistent_failure();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_last_drop_with_persistent_failure() {
        kernel::entry::check::nest_exit::last_drop_with_persistent_failure();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_empty_team_oust_with_persistent_failure() {
        kernel::entry::check::nest_exit::empty_team_oust_with_persistent_failure();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_oust_retries_asid_deallocation() {
        kernel::entry::check::nest_exit::oust_retries_asid_deallocation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_retired_owner_preserves_reused_asid_watchers() {
        kernel::entry::check::nest_exit::retired_owner_preserves_reused_asid_watchers();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_parent_exit_retains_empty_child() {
        kernel::entry::check::nest_exit::parent_exit_retains_empty_child();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_all_reaped_preserves_respawn() {
        kernel::entry::check::nest_exit::all_reaped_preserves_respawn();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_oust_retires_pruned_task_memory() {
        kernel::entry::check::nest_exit::oust_retires_pruned_task_memory();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn nest_task_exit_resumes_retirement_on_schedule() {
        kernel::entry::check::nest_exit::task_exit_resumes_retirement_on_schedule();
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
    fn boot_pci_ranges_windows() {
        kernel::entry::check::boot_devices::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_retention() {
        kernel::entry::check::syscall::retention();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_traverse() {
        kernel::entry::check::syscall::traverse_pages();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_source_views() {
        kernel::entry::check::source_views::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_transfer_slots() {
        kernel::entry::check::transfer::accept();
    }

    // 受控布局下的真 2 MiB 大叶验收：它要塑形帧池，所以单独一例、单独一格 feature。
    #[cfg(all(debug_assertions, feature = "mega-leaf"))]
    #[test]
    fn memory_transfer_mega_leaf() {
        kernel::entry::check::transfer::mega_leaf();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_transfer_pages() {
        kernel::entry::check::syscall::transfer_pages();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_transfer_refusals() {
        kernel::entry::check::syscall::transfer_refusals();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_transfer_wakes_relations() {
        kernel::entry::check::syscall::transfer_wakes_relations();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_page_fault_probe() {
        kernel::entry::check::fault::accept();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn memory_device_window() {
        kernel::entry::check::syscall::device_window();
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
    fn unpublished_roots_release_their_sources() {
        kernel::entry::check::pole::unpublished_roots();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn retired_space_releases_control_records() {
        kernel::entry::check::nest_exit::retirement_releases_control_records();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn closed_space_settles_late_task_memory() {
        kernel::entry::check::nest_exit::late_task_memory_settles_after_space_close();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn closed_view_does_not_block_transfer() {
        kernel::entry::check::closed_transfer::closed_view_does_not_block_transfer();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn attach_rollback_preserves_control_until_settled() {
        kernel::entry::check::attach_rollback::preserves_control_until_settled();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn source_binding_prepared_member_survives_until_settled() {
        kernel::entry::check::source_binding::prepared_member_survives_until_settled();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn source_binding_settled_member_releases_records() {
        kernel::entry::check::source_binding::settled_member_leaves_no_space_or_source_behind();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn source_binding_hand_over_keeps_one_shared_control() {
        kernel::entry::check::source_binding::hand_over_keeps_one_shared_control();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn source_binding_raw_last_drop_keeps_unknown_member() {
        kernel::entry::check::source_binding::raw_last_drop_keeps_unknown_member();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn source_binding_closed_input_allows_pole_cleanup() {
        kernel::entry::check::source_binding::closed_input_does_not_block_pole_cleanup();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pole_open_tracks_cpu_without_view_control() {
        kernel::entry::check::source_binding::pole_open_tracks_cpu_without_view_control();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_derived_child_is_its_own_root() {
        kernel::entry::check::sub_pole::derived_child_is_its_own_root();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_overlap_is_rejected_whole() {
        kernel::entry::check::sub_pole::overlap_is_rejected_whole();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_nesting_holds_the_middle_window() {
        kernel::entry::check::sub_pole::nesting_holds_the_middle_window();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_failed_derivation_publishes_nothing() {
        kernel::entry::check::sub_pole::failed_derivation_publishes_nothing();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_reuse_waits_for_settlement_and_held_frames() {
        kernel::entry::check::sub_pole::reuse_waits_for_settlement_and_held_frames();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_sealed_child_does_not_hold_its_segment() {
        kernel::entry::check::sub_pole::sealed_child_does_not_hold_its_segment();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_old_reference_cannot_bypass_the_gate() {
        kernel::entry::check::sub_pole::old_reference_cannot_bypass_the_gate();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_retained_child_blocks_parent_reuse() {
        kernel::entry::check::sub_pole::retained_child_blocks_parent_reuse();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_old_reference_cannot_transfer_a_child_page() {
        kernel::entry::check::sub_pole::old_reference_cannot_transfer_a_child_page();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_publication_keys_keep_their_domains() {
        kernel::entry::check::sub_pole::publication_keys_keep_their_domains();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_first_delivery_keeps_the_parent_allocation() {
        kernel::entry::check::sub_pole::first_delivery_keeps_the_parent_allocation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_forget_preserves_derived_indices() {
        kernel::entry::check::sub_pole::forget_preserves_derived_indices();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_forget_requires_a_qualified_parent() {
        kernel::entry::check::sub_pole::forget_requires_a_qualified_parent();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_parent_seal_retries_a_pending_descendant() {
        kernel::entry::check::sub_pole::parent_seal_retries_a_pending_descendant();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_parent_seal_closes_a_consumed_child() {
        kernel::entry::check::sub_pole::parent_seal_closes_a_consumed_child();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_sealed_source_keeps_its_inflight_allocation() {
        kernel::entry::check::sub_pole::sealed_source_keeps_its_inflight_allocation();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sub_pole_parent_seal_retries_a_failed_open() {
        kernel::entry::check::sub_pole::parent_seal_retries_a_failed_open();
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
    fn branch_window_narrows_and_binds() {
        kernel::entry::check::syscall::branch_window();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn window_inheritance_does_not_widen() {
        kernel::entry::check::syscall::window_inheritance();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn local_derive_inherits_and_narrows() {
        kernel::entry::check::syscall::local_derive();
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
    fn retention_authorization_delegates_via_accord() {
        kernel::entry::check::syscall::retention_delegation();
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
    fn team_publication_preserves_parent_identity() {
        kernel::entry::check::task::publication();
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
    fn syscall_sync_failure() {
        kernel::entry::check::syscall::sync_failure();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn syscall_capability() {
        kernel::entry::check::syscall::capability();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn execution_domain_policy_core() {
        kernel::entry::check::exit_policy::policy();
        kernel::entry::check::exit_policy::authority();
        kernel::entry::check::exit_policy::install_requires_debark();
        kernel::entry::check::exit_policy::nested_domains();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn unit_creator_distinguishes_owner() {
        kernel::entry::check::syscall::creator_and_owner();
    }

    #[cfg(all(debug_assertions, feature = "critical-scene"))]
    #[test]
    fn critical_domain_exit_stops() {
        kernel::entry::check::critical::run();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn typed_resource_call_domains() {
        kernel::entry::check::syscall::resource_domains();
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

    #[cfg(debug_assertions)]
    #[test]
    fn pie_seal_authority() {
        kernel::entry::check::syscall::pole_seal_authority();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pie_seal_refuses_transfer() {
        kernel::entry::check::syscall::pole_seal_refuses_transfer();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pie_seal_keeps_transferred_page() {
        kernel::entry::check::syscall::pole_seal_keeps_transferred_page();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pie_seal_partial_failure() {
        kernel::entry::check::syscall::pole_seal_partial_failure();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn transfer_across_tasks_and_spaces() {
        kernel::entry::check::syscall::cross_task_interleavings();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn pie_revoke_removes_child() {
        kernel::entry::check::syscall::pole_revoke_removes_child();
    }

    #[test]
    fn scene() {
        // fresh 失败回退那一景靠一个在途夹具注入一次失败：它只在这一格打开时才
        // 装上，且只在 debug 档（夹具与那四个探针同属 `cfg(debug_assertions)`）。
        #[cfg(all(feature = "fresh-scene", debug_assertions))]
        kernel::entry::check::fresh_scene::arm();
        // 真实多 Hart 交错那一景靠一个在途夹具放开另一个 Space 里的对端、等它真登记：
        // 同上，只在它自己那一格打开时才装上，且只在 debug 档。
        #[cfg(all(feature = "race-scene", debug_assertions))]
        kernel::entry::check::race_scene::arm();
        kernel::entry::boot::start().unwrap_or_else(|error| kernel::entry::boot::fail(error));
        kernel::entry::boot::run();
    }
}
