OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
  . = 0x80200000;

  _kernel_start = .;

  .text : {
    *(.text._start)
    *(.text*)
  }

  .trampoline ALIGN(0x1000) : {
    KEEP(*(.trampoline))
  }

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
