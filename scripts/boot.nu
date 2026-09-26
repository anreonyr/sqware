#!/usr/bin/env nu

# QEMU 起法：**起 qemu + 返回退出码**。
# 不判定、不归档、不碰 stdin 的所有权——那是消费者的事。
#
# **照实记（用户裁定"把 boot.nu 拆出可复用的参数表"）**：这张参数表原先长在本文件里，
# 而测试路（`embedded-test` 的 runner）**自带整条 QEMU 命令行**——它硬编码
# `-M virt -bios none -m 128M -nographic -semihosting-config … -kernel`，
# 于是 `-bios SBI.bin` / `-m 256M` 只能靠 `--qemu-arg` 另写一份，参数表就有了第二处。
# 拆开之后：**参数表住 [`qemu-args.nu`](qemu-args.nu)（唯一出处）**，本文件只剩起机；
# 测试路 `scripts/qtest.nu` 读同一张表的**板子那一半**（让出 `-kernel` 与 `-semihosting`，
# 那两格归 runner 加）。
#
# 消费者：
#   scripts/runner.nu  cargo 集成（交互式跑）：stdin 直接继承调用者。
#
# 退出码原样返回（124 = 被外接 timeout 杀）。消费者据此**观察**，不做 pass/fail；
# 注意 nu 脚本在外部命令非零退出时当场中止，故消费者调用本脚本时也要包 `try`。
#
# 用法：nu scripts/boot.nu <elf>
# 环境变量：见 [`qemu-args.nu`](qemu-args.nu) 的头注（逐字搬过去的）。

use ./qemu-args.nu args

def main [elf: path] {
  let qargs = (args $elf)

  # 退出码必须 **exit 出去**，不能只当返回值：nu 脚本把 main 的返回值**打印**出来而进程仍以 0
  # 结束，消费者（runner/门）看到的就永远是 0 —— 超时杀那档会被误报成「正常自退」（实测踩过）。
  let t = ($env.QEMU_TIMEOUT? | default "")
  # **必须包裸 try**：nu 在外部命令非零退出时当场中止整个脚本（其后语句都不执行），
  # 这样退出码才拿得到、调用方才不会被莫名中止。
  if ($t | is-empty) or ($t == "0") {
      try { ^qemu-system-riscv64 ...$qargs } catch { }
  } else {
      try { ^timeout $t qemu-system-riscv64 ...$qargs } catch { }
  }
  exit $env.LAST_EXIT_CODE
}
