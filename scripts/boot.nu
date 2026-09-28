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
# 扳机是 `canonical`（装配单位次 22，最后一条）：它读不到 `exit` 就永不退场，编排域的
# `server::supervise` 于是等一条**永远不响的道**，引导域不退、机器不停机。
#
# **照实记（重量的那一遍：同一份提交镜像、直接起 QEMU、绕开本脚本）**：
#
#   单发一行 · t=0    3/4 **不停机**（rc=124、无停机行，而 14~16 条读数都在）＋ 1 次装配折
#   单发一行 · t≥1 s  28/28 停机          ← "等多久"的下限是**一秒**，不是二十秒
#   完全不喂          0/10 停机（本来就不该停）—— 不喂时机器不会自己停，读数会全部到齐
#
# 两档都**先起稳再喂**：起机那一瞬喂进去的那一发落不进去（第一行）；而"装配期折一条"
# （`system: assemble`；一折之后**其后台全部没有读数**）**只在起机那一瞬有输入的跑里
# 出现过**（2/6 对 0/38）。故单行档与多行档**共用同一个 `QEMU_SETTLE`**（默认 6 s）：
#
#   单行（`exit` 那一档）→ 起稳后喂一次，之后**每 2 s 重喂**直到写不进去（机器没了）。
#                          重喂一条幂等的口令没有任何代价 ⇒ 那一发到底被吃掉没有**不必判**。
#   多行（一整份日程）  → **只喂一次**（重放会重复执行命令），首喂前等 `QEMU_SETTLE` 秒。
#
# **为什么是 6 而不是 3**：让机器**停机**只要 1 s 就够（第二行），但**读数**要等它自己走完——
# 那一行口令一到，`canonical`（那张单上最后一条 = 停机扳机）就退场，编排域当场扑杀还在跑的台。
# 实测同一份镜像：t=3 s 那一档 `probe-operator-gate` 那条读数丢 2/2；t=4 / 5 / 6 s 六跑全在
# （0/6）；而"完全不喂"那十跑读数全在。故这一格取的是**读数走完的下限**，不是"能停机的最小值"。
#
# 为什么重喂那一格不能退成"就喂一次、赌一个停顿"：那个秒数一旦猜短，症状与原缺陷**逐字
# 相同**（没有停机行、退出码 124）——最难查的失败模式。起稳那一下给的是**下限**，不是
# "猜准某一刻"。
#
# **照实记（这一格的下限被设备账那一刀抬高了）**：`QEMU_SETTLE` 默认 **6 s** 是照旧机器量出来的
# ——设备账那一台起来之后，装配那条线多了一段**要等它落完一整台机器的格**（见
# `programs/src/system/hub/mod.rs` 那一节），而 **debug 档**下这一整段比 release 慢好几倍
# （实测：debug 档单发一行 `exit`，非 30 s 以上喂进去，"读数走完"那条下限到不了——症状正是
# 上面那一句"没有停机行"）。**故 debug 档手工跑请给 `QEMU_SETTLE=30` 上下**；整机验收那条路
# （`scripts/qtest.nu --scene …`）不受影响：它**每 2 s 重喂**，且跑的是 release 档。
#
# **照实记（攒不住就出声）**：`mktemp` 一失败（/tmp 只读、TMPDIR 不可写），`$tmp` 就是
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
