// 健康检查 · spare — 后备仓预算验收（预算即契约）：ring 常驻 + 溢出演练。
//
// 在 spawn 用户任务之前运行（spare 与 trace::init 均已就绪），由 `boot::init`
// 经 `crate::health::spare::accept()` 调用。断言：
//   · ring 常驻后余量 ≥ DUMP_BUDGET（打印预算未被吃穿）；
//   · 溢出演练：逐块拉取 1KiB 直到 AllocError（失败路径返回 Err、不 panic），
//     再全部归还——余量须还原到演练前（pull/split/push/merge 无泄漏无失拥）。
// 断言用 `expect!`（health 专用宏）：失败统一报告 + fail-fast。

use core::alloc::Layout;
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::machine;
use crate::memory::allocator::spare;
use crate::memory::allocator::spare::DUMP_BUDGET;
use crate::runtime::diagnose::trace;

/// spare 预算验收（boot 恰好一次；失败即 halt）。
pub fn accept() {
    let h = machine::hart_count();
    let ring = trace::ring_bytes(h);

    // ring 常驻：used ≥ 载荷（used 含块头 32B，故用 ≥；精确值不与分配器开销耦合）。
    crate::expect!(
        spare::used() >= ring,
        "spare: ring {ring} B not resident (used {})",
        spare::used()
    );
    crate::expect!(
        spare::remaining() >= DUMP_BUDGET,
        "spare: dump budget {DUMP_BUDGET} B not reserved (remaining {})",
        spare::remaining()
    );

    // 溢出演练：拉满到 Err 再全还——证明分配/释放/合并闭环且失败路径返回 Err。
    let step = Layout::from_size_align(1024, 16).unwrap();
    let (used_before, remaining_before) = (spare::used(), spare::remaining());
    let mut held: Vec<NonNull<[u8]>> = Vec::new();
    while let Ok(b) = spare::allocator().allocate(step) {
        held.push(b)
    }
    // remaining() < 1024 间接判定，余量落在 [1024, 块开销) 区间时会误报）。
    crate::expect!(
        spare::allocator().allocate(step).is_err(),
        "spare: drill did not reach exhaustion (remaining {})",
        spare::remaining()
    );
    // 逆序归还（相邻块逆序释放 → 合并链仍应还原为单块）。
    for b in held.iter().rev() {
        // SAFETY: b 来自本演练 allocate，layout 同源。
        unsafe { spare::allocator().deallocate(b.cast(), step) };
    }
    crate::expect!(
        spare::remaining() == remaining_before,
        "spare: drill leaked budget (remaining {remaining_before} → {})",
        spare::remaining()
    );
    crate::expect!(
        spare::used() == used_before,
        "spare: drill left residue (used {}), want {used_before}",
        spare::used()
    );

    crate::putln!("[health] spare: ok (ring {ring} B, dump budget {DUMP_BUDGET} B, drill clean)");
}

