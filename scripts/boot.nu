#!/usr/bin/env nu

# QEMU 起法：**起 qemu + 返回退出码**。
# 不判定、不归档——那是消费者的事；**stdin 的交付时机归本文件**（见下）。
#
# 消费者：
#   scripts/runner.nu  cargo 集成（交互式跑）。
#
# 退出码原样返回（124 = 被外接 timeout 杀）。消费者据此**观察**，不做 pass/fail；
# 注意 nu 脚本在外部命令非零退出时当场中止，故消费者调用本脚本时也要包 `try`。
#
# # stdin：TTY 原样继承；管道攒住后延迟交付
#
# 起机期间 UART 尚未就绪，过早交付可能丢输入。两档共用 QEMU_SETTLE（默认 6 s）。
# 单行输入每 2 s 重喂，多行输入只喂一次。内容原样交付，终端不识别退出口令。
# 默认验收喂入由 qtest.nu 负责；交互启动时 terminal 没有用法横幅。
#
# **（攒不住就出声）**：`mktemp` 一失败（/tmp 只读、TMPDIR 不可写），`$tmp` 就是
# 空串，其后的 `cat` / `wc` 全落空 ⇒ **一个字节都不喂**，而脚本仍以 0 退出：症状与
# "喂得太早"逐字相同（rc=124、无停机行），只有一行 `cat: '': 没有那个文件或目录` 可查。
# 故 mktemp / 写不进 / 数不出三处一律**当场报一句到 stderr 并退出**——退路是既有的
# `QEMU_SETTLE=0`（stdin 原样继承）。
#
# 用法：nu scripts/boot.nu <elf>
# 环境变量：
#   QEMU_SETTLE  秒；**两档**输入首喂前的等待（默认 6）——单行档也先起稳再喂（见上）。
#                置空或 0 = **完全不攒**，stdin 原样继承（旧行为，留作对照）。
#   其余（QEMU_TIMEOUT / QEMU_SEED / QEMU_ICOUNT / QEMU_MEM / QEMU_SMP / QEMU_EXTRA_ARGS /
#   QEMU_GDB / QEMU_SEMI / QEMU_FEATURES）见 [`qemu-args.nu`](qemu-args.nu) 的头注。

use ./qemu-args.nu args

# 上游：把 stdin 攒进临时文件再喂（见头注那两档）。
# **单引号**——这段归 bash 解释，nu 一个字都不插值（`$"…"` 里的 `$(` 会被 nu 当成变量）。
const FEED = 'tmp=$(mktemp) || {
  echo "boot.nu: 攒不住 stdin（mktemp 失败）——QEMU_SETTLE=0 可退回 stdin 原样继承" >&2
  exit 1
}
if ! cat > "$tmp"; then
  echo "boot.nu: 攒不住 stdin（写不进 $tmp）" >&2
  rm -f "$tmp"
  exit 1
fi
lines=$(wc -l < "$tmp") || {
  echo "boot.nu: 攒不住 stdin（读不出 $tmp）" >&2
  rm -f "$tmp"
  exit 1
}
sleep "${QEMU_BOOT_SETTLE:-6}"
if [ "$lines" -le 1 ]; then
  while :; do cat "$tmp" || break; sleep 2; done
else
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

  let settle = ($env.QEMU_SETTLE? | default "6")
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
