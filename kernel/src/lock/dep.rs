// lockdep 最小版工具 — 单 hart 锁序违规检测的共用设施
//
// 单 hart 内核的锁序违规只有一种形态：当前执行流在锁仍被自己持有时再次获取
// （关中断后无其他执行流），必然死循环。各锁在入口捕获调用者返回地址
// （read_ra）写入自身 holder_pc；获取时若发现锁已被持有（单 hart 即重入/升级/
// 降级等违例），调用 report 打印持有者与本次调用点后 panic——在挂死前留下现场。
//
// 报告经 putln!（console 模块，SBI DBCN 无锁直写），持锁/关中断态下输出安全。

/// 读取调用者返回地址（RISC-V `ra` 寄存器）。
///
/// 必须内联进 `#[inline(never)]` 的锁入口：内联后 asm 成为函数第一条指令，
/// 此刻 `ra` 仍是调用者的返回地址（prologue 只保存不覆盖，且其后无其他调用）。
#[inline(always)]
pub(crate) fn read_ra() -> usize {
    let ra: usize;
    // SAFETY: 读 ra 寄存器无副作用；asm! 会把未声明的 ra 视为被 clobber，
    // 编译器因此不会假设它保持——读取行为本身安全。
    unsafe { core::arch::asm!("mv {}, ra", out(reg) ra) };
    ra
}

/// 锁序违规报告：打印锁地址、持有者与本次获取调用点后 panic。
///
/// `kind` 为锁类型名（如 `"spinlock"`），`what` 为违规形态
/// （如 `"recursive acquisition"`、`"read→write upgrade"`）。
pub(crate) fn report(
    kind: &'static str,
    what: &'static str,
    lock: usize,
    holder: usize,
    caller: usize,
) -> ! {
    crate::putln!("[LOCKDEP] {kind}: {what} (single-hart lock-order violation)");
    crate::putln!("  lock     @ {lock:#x}");
    crate::putln!("  holder   @ {holder:#x}");
    crate::putln!("  acquirer @ {caller:#x}");
    panic!("{kind} lock-order violation: {what}");
}
