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
  /* 代码段尾：alloc-site 回溯守卫用（候选 ra 须落在 [_kernel_start, _text_end)，
     .data/.bss 的静态符号地址不在其内——防 site 失真）。 */
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

    /* 用例登记段（`--features framework` 才有内容）。三条约束：
       ① 嵌在 `.rodata` 内 ⇒ **不动 `_kernel_edge`**（主栈 guard 靠它定位，见上段注释）；
       ② `KEEP` ⇒ 空引用不被 `--gc-sections` 丢掉 —— 这是"零维护发现"的支点，
          段一丢，症状是**静默零用例**（绿着，什么都没测），比失败更坏；
       ③ 两边界符号供 `framework::discover()` 取切片（与 `_rodata_start` 同一手法）。
       内容只有 `&'static str` 与裸函数指针，故落只读段是安全的（无重定位写）。 */
    . = ALIGN(8);
    __tests_start = .;
    KEEP(*(.tests))
    __tests_end = .;
  }

  . = ALIGN(0x1000);
  _kernel_edge = .;

}
