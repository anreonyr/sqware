#!/usr/bin/env nu

# QEMU 起法的**唯一出处**：凑参数 + 起 qemu + 返回退出码。
# 不判定、不归档、不碰 stdin 的所有权——那是消费者的事。
#
# 消费者：
#   scripts/runner.nu  cargo 集成（交互式跑）：stdin 直接继承调用者。
#   scripts/examine.nu 验收门：**输入不穿 nu** —— `tail -f -n +1 <命令文件> | nu boot.nu <elf>`
#                      （长驻写端喂 guest，逐步 expect）。nu 无 `<`，故用管道代 FIFO。
#
# 退出码原样返回（124 = 被外接 timeout 杀）。消费者据此**观察**，不做 pass/fail；
# 注意 nu 脚本在外部命令非零退出时当场中止，故消费者调用本脚本时也要包 `try`。
#
# 用法：nu scripts/boot.nu <elf>
# 环境变量：
#   QEMU_TIMEOUT  秒；空或 0 = 不限制（外接 timeout）
#   QEMU_SEED     整数；空 = 用 qemu 自己的随机（本脚本不改写它）
#   QEMU_ICOUNT   非空则加 `-icount <值>`（默认 auto,sleep=on）；**置空可关**。
#                 验收门关掉它：按宿主时间节流会让 guest 与输入日程失步，实测约 1/5 的轮次
#                 guest 会在某一步之后停止取输入（详见 docs/audit-flying-wires.md §9.3）。
#   QEMU_MEM / QEMU_SMP / QEMU_EXTRA_ARGS / QEMU_GDB
#   QEMU_SEMI / QEMU_FEATURES   含 semihosting ⇒ 给 qemu 加 -semihosting。**只管 QEMU 侧**：
#                               被跑 ELF 的 feature 由调用方 cargo 决定（缺则无结构化导出）。
#
# 串口只绑 stdio、**不复用 monitor**：`-nographic` 等价 `-serial mon:stdio`，mux 会把 stdin
# 按模式分派给串口或 monitor，一旦切走就是静默改道 ⇒ 显式写
# `-display none -serial stdio -monitor none`。代价：Ctrl-A c 进 monitor 的用法不再可用。

def main [elf: path] {
  let proj_root = ($env.FILE_PWD | path dirname)
  let elf = ($elf | path expand)
  let extra = ($env.QEMU_EXTRA_ARGS? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let gdb = if ($env.QEMU_GDB? == "1") { ["-s", "-S"] } else { [] }

  let semi_requested = ($env.QEMU_SEMI? | default "0") == "1"
  let feats_initial = ($env.QEMU_FEATURES? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let feats = if $semi_requested and not ($feats_initial | any { |f| $f == "semihosting" }) {
      $feats_initial | append "semihosting"
  } else {
      $feats_initial
  }
  let semihosting = if ($feats | any { |f| $f == "semihosting" }) { ["-semihosting"] } else { [] }

  let icount = ($env.QEMU_ICOUNT? | default "auto,sleep=on")
  let icount_args = if ($icount | is-empty) { [] } else { ["-icount", $icount] }
  let seed = ($env.QEMU_SEED? | default "")
  let seed_args = if ($seed | is-empty) { [] } else { ["-seed", $seed] }

  # initrd blob（build.rs 打包，与内核 ELF 同目录）：存在才传 -initrd。
  let initrd = ($elf | path dirname | path join "initrd.img")
  let initrd_arg = if ($initrd | path exists) { ["-initrd", $initrd] } else { [] }

  let args = [
    "-machine", "virt"
    "-bios", ($proj_root | path join "SBI.bin")
    "-kernel", $elf
    "-display", "none"
    "-serial", "stdio"
    "-monitor", "none"
    "-no-reboot"
    "-m", ($env.QEMU_MEM? | default "128")
    "-smp", ($env.QEMU_SMP? | default "4")
    ...$seed_args
    ...$icount_args
    ...$semihosting
    ...$initrd_arg
    ...$gdb
    ...$extra
  ]

  # 退出码必须 **exit 出去**，不能只当返回值：nu 脚本把 main 的返回值**打印**出来而进程仍以 0
  # 结束，消费者（runner/门）看到的就永远是 0 —— 超时杀那档会被误报成「正常自退」（实测踩过）。
  let t = ($env.QEMU_TIMEOUT? | default "")
  # **必须包裸 try**：nu 在外部命令非零退出时当场中止整个脚本（其后语句都不执行），
  # 这样退出码才拿得到、调用方才不会被莫名中止。
  if ($t | is-empty) or ($t == "0") {
      try { ^qemu-system-riscv64 ...$args } catch { }
  } else {
      try { ^timeout $t qemu-system-riscv64 ...$args } catch { }
  }
  exit $env.LAST_EXIT_CODE
}
