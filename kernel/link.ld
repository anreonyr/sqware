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
