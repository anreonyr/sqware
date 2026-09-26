OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
  . = 0x80200000;

  _kernel_base = .;

  .text : {
    *(.text._start)
    *(.text*)
  }

  .trampoline ALIGN(0x1000) : {
    KEEP(*(.trampoline))
  }
  /* 代码段尾：**今日无读者**（旧 alloc-site 回溯守卫随 fence 审计层一起删了；
     内核域判定今天比的是 `_kernel_edge`）。留着是为了链接脚本的完整性。 */
  _text_end = .;

  .data : {
    *(.data*)
  }
  .bss : {
    *(.bss*)
  }

  /* 只读段放镜像最后：主栈位于 `_kernel_edge` 之上、向下生长，越界第一脚即踩
     `.rodata`——其映射为只读（W=0，见 memory::manager），写即写保护缺页，
     天然充当主栈 guard（省 unmap 帧，且顺带给内核 .rodata 加只读保护）。
     页对齐独立：不与 .bss 同页，protect 只读时不会误伤可写段。 */
  .rodata ALIGN(0x1000) : {
    _rodata_start = .;
    *(.rodata*)
  }

  . = ALIGN(0x1000);
  _kernel_edge = .;

}

/* **照实记（两处与本文件有关的改动，用户裁定"迁移到 embedded-test"）**：
 *
 * ① 本文件叫 `link.ld` 时，`cargo-qtest` 认不出它——那份 runner 对 riscv 的判据是
 *    "manifest 目录里有没有 `.x` 文件"，没有就**自己生成一份**（`ORIGIN = 0x80000000`）。
 *    而内核链在 **0x80200000**（OpenSBI 占 0x80000000）⇒ 实测报
 *    `Some ROM regions are overlapping`。改名成 `.x` 既压掉它的生成物，又给出正确地址。
 *
 * ② `.rodata` 里原先还有一段 `.tests`（`__tests_start` / `KEEP(*(.tests))` /
 *    `__tests_end`），供自研框架的 `discover()` 取用例切片。那套已删；用例的元数据段
 *    由 `embedded-test.x` 提供（`cargo-qtest` 自己加 `-Tembedded-test.x`）。
 *
 * 本文件其余部分——尤其是 `.rodata` **放在最后**这一条——一个字没动：主栈 guard 靠
 * `_kernel_edge` 定位。
 */
