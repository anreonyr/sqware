#!/bin/sh
# 忙机台 —— 「到点兑现」的收益面（见 `programs/src/bin/stress/load.rs`）。
#
# 为什么单独一台：这条债要"核一刻不闲"才成立——`redeem`/`drain` 是**全局**的，**任一颗空闲核**
# 都会按 `due()` 武装、在到点那一刻替全局兑现，故 `soak`（全员 1 ms 轮询睡眠）与 `rig`（只有
# 一枚 `churn`）都量不到它（release soak 16k 样本 A/B 无差别）。本台子用占核者（`busy`）把核
# 钉住，再用**一枚稀疏**打点者（`park`）把"到点兑现迟到"量出来。
#
# **核数就是这条债的开关**（默认单核，见下面的 `QEMU_SMP`）：
#   默认（单核）   机制隔离档：没有第二颗核能替全局兑现 ⇒ 债必然现身；实测修复前
#                  `late_avg=97 ms / late_max=99 ms`，修复后 `0 ms / 125 µs`。
#   QEMU_SMP=4     对照档：有核空闲 ⇒ 债被"空闲核按 due() 武装"盖住（这正是树内量不出来的
#                  原因）。用法：`QEMU_SMP=4 scripts/load.sh 1 --release`。
#
# 判据（三条一起）：
#   1) `load: spawned rows=…`（负荷真铺开了）；
#   2) 停机行 `task: all tasks exited, system halted`（自己收干净，不是被 timeout 杀掉）；
#   3) 紧随其后的内核读数 `timer: late_n=… late_max_ms=… late_avg_ms=… traps=… tocks=…`。
#
# 用法：
#   scripts/load.sh [轮数] [--release]      # 默认 1 轮，debug 档
# 退出码：全过 0，有不过 1。日志落在 target/load/load-<时间戳>-<轮>.log。
set -u
# 默认单核（可用环境变量覆盖）：多核档下这条债量不出来，见头注。
QEMU_SMP="${QEMU_SMP:-1}"
export QEMU_SMP
rounds="${1:-1}"
[ "$#" -ge 1 ] && shift
prof=""
[ "${1:-}" = "--release" ] && prof="--release"
out=target/load
mkdir -p "$out"
tag="load-$(date +%s)"
pass=0
i=1
while [ "$i" -le "$rounds" ]; do
  log="$out/$tag-$i.log"
  SQWARE_ROOT=load timeout 120 cargo run $prof > "$log" 2>&1
  if ! grep -q "load: spawned rows=" "$log"; then
    echo "round $i: FAIL 负荷没铺开（末行：$(grep -a 'load:' "$log" | tail -1)）"
  elif ! grep -q "task: all tasks exited, system halted" "$log"; then
    echo "round $i: FAIL 无停机行（末行：$(grep -a -E 'load:|root:|\[stop\]' "$log" | tail -1)）"
  elif ! grep -q "timer: late_n=" "$log"; then
    echo "round $i: FAIL 无内核读数"
  else
    pass=$((pass + 1))
    echo "round $i: $(grep -a 'timer: late' "$log")"
  fi
  i=$((i + 1))
done
echo "== 通过 $pass/$rounds（日志 $out/$tag-*.log）=="
[ "$pass" -eq "$rounds" ]
