// ── 适配层：boot ──
//
// 装配入口：按 DTB 核数建立 per-hart 调度器态；副核 idle 循环。

use alloc::boxed::Box;

use crate::machine;
use crate::runtime::switcher::trampoline::restore;

use super::core::{CONDUCTORS, Conductor};
use super::trap::run;

/// 按实际核数（DTB）动态分配 per-hart 调度器状态（调用**恰好一次**，先于任何
/// 调度器访问）。
pub fn init() {
    let n = machine::hart_count();
    assert!(n > 0, "no harts");
    let sched: Box<[Conductor]> = (0..n).map(Conductor::new).collect();
    assert!(
        CONDUCTORS.set(Box::leak(sched)).is_ok(),
        "conductors double init"
    );
}

/// 副核 idle 循环：spin + steal；拿到任务即 restore（永不返回）；全退出停机。
pub fn idle() -> ! {
    restore(run())
}
