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
# # stdin：TTY 原样继承；管道则**攒住再喂**，且按行数分两档
#
# 这一格是**量出来的**，不是洁癖：`echo exit | cargo run --release` 把命令在**起机那一
# 瞬**灌进去，guest 当场吃掉一个字节——日志里逐字留下一个孤立的 `xit`。而整机收场的
# 扳机是 `echo`（装配单位次 18，最后一条）：它读不到 `exit` 就永不退场，编排域的
# `server::supervise` 于是等一条**永远不响的道**，引导域不退、机器不停机。
#
#   立刻喂      11~12 笔结局 · 停机行 0 · 退出码 124
#   起机后 3 s  14 笔结局 · 停机行 1 · 退出码 0
#
# 也就是说：**门删掉时被一并带走的，除了判定，还有 stdin 的日程**——门当年是"等机器
# 起来再逐条喂"。这里补回来，但**不赌"等多久"**：
#
#   单行（`exit` 那一档）→ **反复喂**（立刻一次、之后每 2 s 一次）直到写不进去（机器没了）。
#                          重喂一条幂等的口令没有任何代价，而"起稳"这件事由机器自己说
#                          ——连"第一次要等多久"都不必猜：第一次被吃掉是**预期之内**的。
#   多行（一整份日程）  → **只喂一次**（重放会重复执行命令），首喂前等 `QEMU_SETTLE` 秒。
#
# 为什么单行那一档不能也靠"固定停顿"：那个秒数一旦猜短，症状与原缺陷**逐字相同**
# （结局笔数变少、没有停机行、退出码 124）——最难查的失败模式。
#
# 用法：nu scripts/boot.nu <elf>
# 环境变量：
#   QEMU_SETTLE  秒；**多行**输入首喂前的等待（默认 3）。置空或 0 = **完全不攒**，
#                stdin 原样继承（旧行为，留作对照）。
#   其余（QEMU_TIMEOUT / QEMU_SEED / QEMU_ICOUNT / QEMU_MEM / QEMU_SMP / QEMU_EXTRA_ARGS /
#   QEMU_GDB / QEMU_SEMI / QEMU_FEATURES）见 [`qemu-args.nu`](qemu-args.nu) 的头注。

use ./qemu-args.nu args

# 上游：把 stdin 攒进临时文件再喂（见头注那两档）。
# **单引号**——这段归 bash 解释，nu 一个字都不插值（`$"…"` 里的 `$(` 会被 nu 当成变量）。
const FEED = 'tmp=$(mktemp)
cat > "$tmp"
if [ "$(wc -l < "$tmp")" -le 1 ]; then
  while :; do cat "$tmp" || break; sleep 2; done
else
  sleep "${QEMU_BOOT_SETTLE:-3}"
  cat "$tmp"
fi
rm -f "$tmp"'

def main [elf: path] {
  let qargs = (args $elf)

  # 退出码必须 **exit 出去**，不能只当返回值：nu 脚本把 main 的返回值**打印**出来而进程仍以 0
  # 结束，消费者（runner）看到的就永远是 0 —— 超时杀那档会被误报成「正常自退」（实测踩过）。
  let t = ($env.QEMU_TIMEOUT? | default "")
  let wrap = not (($t | is-empty) or ($t == "0"))
  let cmd = if $wrap { "timeout" } else { "qemu-system-riscv64" }
  let head = if $wrap { [$t "qemu-system-riscv64"] } else { [] }

  let settle = ($env.QEMU_SETTLE? | default "3")
  let staged = (not (is_tty)) and (not ($settle | is-empty)) and ($settle != "0")
  $env.QEMU_BOOT_SETTLE = $settle

  # **必须包裸 try**：nu 在外部命令非零退出时当场中止整个脚本（其后语句都不执行），
  # 这样退出码才拿得到、调用方才不会被莫名中止。
  if $staged {
      try { ^bash -c $FEED | ^$cmd ...$head ...$qargs } catch { }
  } else {
      try { ^$cmd ...$head ...$qargs } catch { }
  }
  exit $env.LAST_EXIT_CODE
}

# 本脚本的 stdin 是不是终端（`test -t 0` 是 POSIX，`/usr/bin/test` 支持 `-t fd`）。
def is_tty [] {
  (^test -t 0 | complete).exit_code == 0
}
