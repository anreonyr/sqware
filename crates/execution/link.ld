OUTPUT_ARCH(riscv)
ENTRY(_start)

/* 每个输出节页对齐：让每个 PT_LOAD 段的 vaddr 与 offset 都落在 4 KiB 边界上，
   parser 要求 LOAD 段页对齐（外带 .text/.rodata 各自成段的 ELF 也可通过）。 */
SECTIONS
{
  . = 0x10000;          /* IMAGE_BASE：内核 loader 按绝对 VMA 装载 */

  .text : ALIGN(0x1000) { *(.text*) }
  .rodata : ALIGN(0x1000) { *(.rodata*) }
  .data : ALIGN(0x1000) { *(.data*) }
  .bss : ALIGN(0x1000) { *(.bss*) }

  /* freestanding：无需 unwind 表，丢弃以免额外占用。 */
  /DISCARD/ : { *(.eh_frame*) }
}
