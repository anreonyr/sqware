#![no_std]
#![no_main]

//! 内核内用例的**登记与逐例打点** —— `embedded-test` 的测试目标。
//!
//! **照实记（用户裁定"去掉所有的测试 / 迁移到 embedded-test"）**：这一份替掉了自研的
//! `kernel/src/framework/`（4 文件 274 行：`.tests` 段 + `test!` 宏 + `discover()` +
//! 逐例打点运行器）与 `health/mod.rs` 里那 8 个 `test!` 块。发现与汇报都归
//! `embedded-test`；**用例体一个字没动**，还在 `kernel::health::*` 那五个模块里。
//!
//! # 一起来（一例一个 QEMU 实例）
//!
//! runner 是 `cargo-qtest`，它对**每一例**起一个 QEMU、用 semihosting `SYS_GET_CMDLINE`
//! 告诉固件跑哪一例，固件跑完用 `SYS_EXIT` 回退出码 ⇒ 一次启动 = 一次设备复位。
//! 跑法：`nu scripts/qtest.nu`（见那份脚本的头注）。
//!
//! # 这一份 `_start` 与 bin 那一份的差别
//!
//! 只多两行：`embedded-test` 的入口**不收参数**，故 OpenSBI 给的设备树指针（`a1`）得自己
//! 存下来，供 `#[init]` 取用。其余（`tp` / canary / 栈）与 `src/main.rs` **逐字相同**——
//! 那是内核的入口约定，不是测试的事。

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
    "    j    main",         //   `main` = embedded-test 导出的那一个
);

/// OpenSBI 交过来的设备树指针。**必须 `no_mangle`**：上面那段汇编靠 `la` 按名字取它。
#[unsafe(no_mangle)]
static BOOT_DTP: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::BOOT_DTP;
    use core::sync::atomic::Ordering;

    /// 每一例开跑前：先置"在跑用例"（panic 通道据此判"这一例失败"，见
    /// `kernel::testing_mode`），再把内核起到 `trap` 就绪。
    ///
    /// **不起服务**：用例要的是调度器、分配器、页表、`Space` 原语；起了服务反而要收场。
    #[init]
    fn init() {
        kernel::testing_mode();
        kernel::init(BOOT_DTP.load(Ordering::Relaxed));
    }

    // 用例：前八个的名字与次序照搬原先 `health/mod.rs` 那 8 个 `test!` 块（失败的人要
    // 看得懂哪一块塌了），第九个是后补的 `hart_multi`。用例体仍在 `kernel::health` 里。

    #[test]
    fn spare_budget() {
        kernel::health::spare::accept();
    }

    #[test]
    fn pagetable_recycle() {
        kernel::health::pagetable::pagetable();
    }

    #[test]
    fn stress_allocator() {
        kernel::health::stress::accept();
    }

    #[test]
    fn shell_primitives() {
        kernel::health::shell::accept();
    }

    #[test]
    fn permit_form() {
        kernel::health::permit::form();
    }

    #[test]
    fn permit_members() {
        kernel::health::permit::members();
    }

    #[test]
    fn permit_fanout() {
        kernel::health::permit::fanout();
    }

    #[test]
    fn permit_order() {
        kernel::health::permit::order();
    }

    // 第九例（后补）：钉住参数表那颗 `-smp` 默认——见 `kernel/src/health/hart.rs` 的头注。
    #[test]
    fn hart_multi() {
        kernel::health::hart::count();
    }
}
