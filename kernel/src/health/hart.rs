// 健康检查 · hart — 起步那一格：**进来的 hart 有几颗**。
//
// 断言：机器报告的 hart 数 > 1。这一例**故意钉住参数表那颗默认**（`qemu-args.nu` 的
// `-smp 4`）——失败信息把话说完。
//
// **照实记（推翻一条）**：`scripts/qtest.nu` 原先把 `-smp` 默认成 1，理由是"`_start`
// 每颗 hart 都会跑一遍 ⇒ 4 颗一起冲进 `__embedded_test_start`，必炸"。**量下来不是**：
// 那条理由只对 `-bios none`（M 态、所有 hart 从复位向量起跑）成立，而参数表给的是
// `-bios SBI.bin`——S 态下只有引导 hart 进内核，副核由 `boot::boot_harts()` 经 HSM
// 拉起（**产品路本来就是 `-smp 4`**）。实测 `QEMU_SMP=4` 下原先那八例 8/8 绿、1.92 s。
//
// 这一例就是那条照实记的钉子：**能跑到这里**（没被多颗 hart 并发冲散）本身就是读数。

#![cfg(debug_assertions)]

use crate::hart;

/// hart 数验收（用例体；登记在 `kernel/tests/embedded.rs`）。
pub fn count() {
    let count = hart::hart_count();
    crate::expect!(
        count > 1,
        "机器只报了 {count} 颗 hart ⇒ 参数表那颗 `-smp 4` 没生效（`QEMU_SMP` 被改过？）。\
         本用例钉的就是那颗默认，与 `scripts/qtest.nu` 的头注同一条"
    );
}
