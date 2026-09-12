// 健康检查 · spare — 后备仓预算验收（预算即契约）：ring 常驻 + 溢出演练。
//
// 断言：
//   · ring 常驻后余量 ≥ DUMP_BUDGET（打印预算未被吃穿）；
//   · 溢出演练：逐块拉取 1KiB 直到 AllocError（失败路径返回 Err、不 panic），
//     再全部归还——余量须还原到演练前（分配/释放/合并闭环无泄漏）。
// 断言用 `expect!`（health 专用宏）：失败统一报告 + fail-fast。

// 用例只在 debug / framework 档存在（与 `pagetable.rs` 同一 gate）：这一档才有
// 消费者调用它，其余档里编进去就是一段没人跑、也没人读的代码。
#![cfg(any(debug_assertions, feature = "framework"))]

use core::alloc::{Allocator, Layout};
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::machine;
use crate::memory::allocator::spare;
use crate::memory::allocator::spare::DUMP_BUDGET;
use crate::memory::allocator::statistics;
use crate::runtime::diagnose::trace;

/// spare 预算验收（用例体；登记在 `mod.rs` 的 `test!` 块）。
pub(super) fn accept() {
    let h = machine::hart_count();
    let ring = trace::ring_bytes(h);

    crate::expect!(
        statistics::spare_occupied() >= ring,
        "spare: ring {ring} B not resident (occupied {})",
        statistics::spare_occupied()
    );
    crate::expect!(
        statistics::spare_available() >= DUMP_BUDGET,
        "spare: dump budget {DUMP_BUDGET} B not reserved (available {})",
        statistics::spare_available()
    );

    let step = Layout::from_size_align(1024, 16).unwrap();
    // 演练前后的 (在手段数, 余量) 快照：余量由在手段数导出，两条一起核。
    let before = (statistics::spare_occupied(), statistics::spare_available());
    let mut held: Vec<NonNull<[u8]>> = Vec::new();
    while let Ok(b) = spare::spare().allocate(step) {
        held.push(b)
    }
    crate::expect!(
        spare::spare().allocate(step).is_err(),
        "spare: drill did not reach exhaustion (available {})",
        statistics::spare_available()
    );
    for b in held.iter().rev() {
        unsafe { spare::spare().deallocate(b.cast(), step) };
    }
    let after = (statistics::spare_occupied(), statistics::spare_available());
    crate::expect!(
        after.1 == before.1,
        "spare: drill leaked budget (available {0} → {1})",
        before.1,
        after.1
    );
    crate::expect!(
        after.0 == before.0,
        "spare: drill left residue (occupied {0}), want {1}",
        after.0,
        before.0
    );
}
