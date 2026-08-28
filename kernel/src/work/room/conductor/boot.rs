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
    let mut sched: Box<[Conductor]> = (0..n).map(Conductor::new).collect();
    // per-hart 直达挂接：tp → PerHart.conductor——借未发布前的 `&mut` 切片回填
    // 每核调度器指针（随后 Box::leak 进 CONDUCTORS；current() 零索引依赖此项，
    // 先于任何调度器访问）。
    for (h, c) in sched.iter_mut().enumerate() {
        machine::set_conductor(h, c as *mut Conductor as *mut ());
    }
    assert!(
        CONDUCTORS.set(Box::leak(sched)).is_ok(),
        "conductors double init"
    );
}

/// 副核 idle 循环：spin + steal；拿到任务即 restore（永不返回）；全退出停机。
pub fn idle() -> ! {
    restore(run())
}
