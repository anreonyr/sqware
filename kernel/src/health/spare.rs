// 健康检查 · spare — 后备仓预算验收（预算即契约）：ring 常驻 + 溢出演练。
//
// 断言：
//   · ring 常驻后余量 ≥ DUMP_BUDGET（打印预算未被吃穿）；
//   · 溢出演练：逐块拉取 1KiB 直到 AllocError（失败路径返回 Err、不 panic），
//     再全部归还——余量须还原到演练前（分配/释放/合并闭环无泄漏）。
// 断言用 `expect!`（health 专用宏）：失败统一报告 + fail-fast。

use core::alloc::{Allocator, Layout};
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::machine;
use crate::memory::allocator::spare;
use crate::memory::allocator::spare::DUMP_BUDGET;
use crate::memory::allocator::statistics;
use crate::runtime::diagnose::trace;

/// spare 预算验收（失败即 halt）。
pub fn accept() {
    let h = machine::hart_count();
    let ring = trace::ring_bytes(h);

    let view = statistics::view_spare();
    crate::expect!(
        view.occupied >= ring,
        "spare: ring {ring} B not resident (occupied {})",
        view.occupied
    );
    crate::expect!(
        view.available >= DUMP_BUDGET,
        "spare: dump budget {DUMP_BUDGET} B not reserved (available {})",
        view.available
    );

    let step = Layout::from_size_align(1024, 16).unwrap();
    let before = *statistics::view_spare();
    let mut held: Vec<NonNull<[u8]>> = Vec::new();
    while let Ok(b) = spare::spare().allocate(step) {
        held.push(b)
    }
    crate::expect!(
        spare::spare().allocate(step).is_err(),
        "spare: drill did not reach exhaustion (available {})",
        statistics::view_spare().available
    );
    for b in held.iter().rev() {
        unsafe { spare::spare().deallocate(b.cast(), step) };
    }
    let after = *statistics::view_spare();
    crate::expect!(
        after.available == before.available,
        "spare: drill leaked budget (available {0} → {1})",
        before.available,
        after.available
    );
    crate::expect!(
        after.occupied == before.occupied,
        "spare: drill left residue (occupied {0}), want {1}",
        after.occupied,
        before.occupied
    );

    crate::putln!("[health] spare: ok (ring {ring} B, dump budget {DUMP_BUDGET} B, drill clean)");
}
