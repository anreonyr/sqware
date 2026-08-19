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

  /* 陷阱 trampoline 页：必须页对齐且 ≤ 4 KiB（TRAMPOLINE VA 只映射一页，
     见 manager::space::Space::TRAMPOLINE 与 runtime/trampoline.rs） */
  .trampoline ALIGN(0x1000) : {
    KEEP(*(.trampoline))
  }

  .rodata : {
    *(.rodata*)
  }
  .data : {
    *(.data*)
  }
  .bss : {
    *(.bss*)
  }

  _kernel_end = .;

  . = ALIGN(16);

  /* 引导栈：debug 构建 probe/putln 栈用量大，须留足余量（曾溢出砸坏 .bss）。
     放置在 _free_base **之前**：free 区（bump/pool/frame）从 _free_base 起，
     若不把引导栈排除在 free 区外，early 栈残留数据会带进 block 数组区
     （8 核 DTB 下 probe 栈深、fdt 指针写入数组区 → 块级双发误报）。 */
  _early_stack_bottom = .;
  . += 16K;
  _early_stack_top = .;

  . = ALIGN(0x1000);

  _free_base = .;

}
