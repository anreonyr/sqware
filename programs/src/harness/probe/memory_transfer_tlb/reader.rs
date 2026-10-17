//! 那台远端读者：同一 Team 里 raw 起的一台真任务，进热窗口的就是它。
//!
//! 它不建 TLS（一阵就走）、**也不分配**：报话那一句是几条固定字符串之一，
//! 控制者那一侧才是带数字的那一行（一个事实一个印点）。
//! 报话与收工都在窗口**出去之后**——那时 `stvec`/`sstatus` 已经装回生产那一套，
//! 报话这一条 ecall 本身就是“出口真的恢复过了”的现场证据。

use env::{EXIT_OK, TaskId, TeamId};

use super::hotpath;
use super::window::Window;

/// 这几种出口各自那一句话：都以 `memory-transfer-tlb: reader left` 起头，
/// 故 qtest 那一侧的完成检查只认前缀，而 semihosting 日志里还能看出是哪一种。
const TRAPPED: &str = "memory-transfer-tlb: reader left trapped";
const STALE: &str = "memory-transfer-tlb: reader left stale";
const LATE: &str = "memory-transfer-tlb: reader left late";
const OTHER: &str = "memory-transfer-tlb: reader left";

/// 起一台读者。
///
/// 回到家时它是 `Held`：控制者写完共享窗、报完它的 tid 才放行，
/// 故读者第一眼看到的共享窗已经是完整的。
pub(super) fn spawn() -> TaskId {
    execution::unit::spawn(TeamId::new(0), entry as *const () as usize, &[], 0)
        .unwrap_or_else(|error| panic!("memory-transfer-tlb: spawn the reader: {:?}", error.source))
}

/// 那条 raw 入口（`a0` = 启动参数区；本景不给参数，故不读它）。
extern "C" fn entry(_arg: usize) -> ! {
    let verdict = hotpath::enter();
    crate::debug::put(match verdict {
        Window::TRAPPED => TRAPPED,
        Window::STALE => STALE,
        Window::TIMEOUT => LATE,
        _ => OTHER,
    });
    execution::room::reap(EXIT_OK, None)
}
