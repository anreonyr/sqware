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

  .rodata : {
    *(.rodata*)
  }
  .data : {
    *(.data*)
  }
  .bss : {
    *(.bss*)
  }

  . = ALIGN(0x1000);
  _kernel_edge = .;

}
