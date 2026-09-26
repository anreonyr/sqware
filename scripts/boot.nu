#!/usr/bin/env nu

# QEMU 起法：**起 qemu + 返回退出码**。
# 不判定、不归档——那是消费者的事；**stdin 的交付时机归本文件**（见下）。
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
#   scripts/runner.nu  cargo 集成（交互式跑）。
#
# 退出码原样返回（124 = 被外接 timeout 杀）。消费者据此**观察**，不做 pass/fail；
# 注意 nu 脚本在外部命令非零退出时当场中止，故消费者调用本脚本时也要包 `try`。
#
# # stdin：TTY 原样继承；**管道攒住、等机器起稳再开闸**
#
# 这一格是**量出来的**，不是洁癖：`echo exit | cargo run --release` 把命令在**起机那一
# 瞬**灌进去，guest 当场吃掉一个字节——日志里逐字留下一个孤立的 `xit`。而整机收场的
# 扳机是 `echo`（装配单位次 18，最后一条）：它读不到 `exit` 就永不退场，编排域的
# `server::supervise` 于是等一条**永远不响的道**，引导域不退、机器不停机。
#
#   立刻喂      11~12 笔结局 · 停机行 0 · 退出码 124
#   起机后 3 s  14 笔结局 · 停机行 1 · 退出码 0（3 秒档重复 5 次 ⇒ 5/5）
#
# 也就是说：**门删掉时被一并带走的，除了判定，还有 stdin 的日程**——门当年是"等机器
# 起来再逐条喂"。这里先把最简的那一版补回来（攒住 + 固定停顿）；等日程真有第二条命令
# 时再改成"按输出触发"。
#
# 用法：nu scripts/boot.nu <elf>
# 环境变量：
#   QEMU_SETTLE  秒；stdin 是管道时"攒多久再开闸"（默认 3）。置空或 0 = 不攒（旧行为）。
#   其余（QEMU_TIMEOUT / QEMU_SEED / QEMU_ICOUNT / QEMU_MEM / QEMU_SMP / QEMU_EXTRA_ARGS /
#   QEMU_GDB / QEMU_SEMI / QEMU_FEATURES）见 [`qemu-args.nu`](qemu-args.nu) 的头注。

use ./qemu-args.nu args

def main [elf: path] {
  let qargs = (args $elf)

  # 退出码必须 **exit 出去**，不能只当返回值：nu 脚本把 main 的返回值**打印**出来而进程仍以 0
  # 结束，消费者（runner）看到的就永远是 0 —— 超时杀那档会被误报成「正常自退」（实测踩过）。
  let t = ($env.QEMU_TIMEOUT? | default "")
  let wrap = not (($t | is-empty) or ($t == "0"))
  let cmd = if $wrap { "timeout" } else { "qemu-system-riscv64" }
  let head = if $wrap { [$t "qemu-system-riscv64"] } else { [] }

  # 管道 ⇒ 加一级上游把输入攒住：`sleep` 不读 stdin（数据留在管道里），`cat` 再放行。
  # TTY、或 `QEMU_SETTLE=0` ⇒ 不加这一级，stdin 原样继承。
  let settle = ($env.QEMU_SETTLE? | default "3")
  let staged = (not (is_tty)) and (not ($settle | is-empty)) and ($settle != "0")

  # **必须包裸 try**：nu 在外部命令非零退出时当场中止整个脚本（其后语句都不执行），
  # 这样退出码才拿得到、调用方才不会被莫名中止。
  if $staged {
      try { ^bash -c $"sleep ($settle); cat" | ^$cmd ...$head ...$qargs } catch { }
  } else {
      try { ^$cmd ...$head ...$qargs } catch { }
  }
  exit $env.LAST_EXIT_CODE
}

# 本脚本的 stdin 是不是终端（`test -t 0` 是 POSIX，`/usr/bin/test` 支持 `-t fd`）。
def is_tty [] {
  (^test -t 0 | complete).exit_code == 0
}
