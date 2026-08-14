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

  _trap_stack_bottom = .;
  . += 16K;
  _trap_stack_top = .;

  _free_base = .;

  _early_stack_bottom = .;
  . += 4K;
  _early_stack_top = .;

}
