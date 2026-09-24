#!/usr/bin/env bash
# 公平台 —— **持树者公平**那一格的读数台（见 `docs/fair-gate.md`）
#
# 场景：`SQWARE_ROOT=fair` 起的是**同一份引导镜像**，只是编排域那张装配单多一条"聊天客人"
# （`probe-deep`，`system/main.rs` 里按构建期的 `--cfg sqware_fair` 选）；而那一台上它**不让手**
# （每 16 手才让一次，见 `probe_deep.rs` 的粒度表）。受害者是既有的 `echo`——它的那几条读数
# 就是判据（照裁定：**"别人的期限还在一秒内"**）。
#
# 判据（每轮）：
#   1. 有停机行（整机不塌）；
#   2. 聊天客人自己跑完（`probe-deep: tree deep=`）**且它的五条用例全过**（`[case] cases 5
#      ok 5 fail 0`——判据搬进了那台探针自己，见 `docs/harness-gate.md` §6）；
#   3. **受害者的读数没被挤过期**——`echo` 那五条与默认台逐字一致。
#
# 照实记（这一台今天是**红**的，红在哪就是读数）：不让手的条件下，持树者是串行的，
# `echo` 的同步往返会被挤到 1 秒过期（`probe_deep.rs` 头注那张表：512 层全剪那一档 soak 0/10）。
# 修法是调度/配额那一族（`docs/fair-gate.md` §4.3），**这一刀只立读数**。
#
# 用法：
#   scripts/fair.sh [轮数] [旗标]         # 默认 3 轮、**debug**（与 soak 同档，便于与旧表对照）
#                                         # 要 release 就 `scripts/fair.sh 3 --release`
#
# 环境：与验收门对齐（`QEMU_ICOUNT=`，理由见 `scripts/stress.sh` 头注）。
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

QEMU_ICOUNT=
export QEMU_ICOUNT

rounds="${1:-3}"
# debug 是 cargo 的默认档 ⇒ 默认**不传旗标**（`--debug` 不是 `cargo run` 的合法参数）。
prof="${2:-}"
[ "$prof" = "--debug" ] && prof=""
out=target/fair
mkdir -p "$out"
tag="fair-$(date +%s)"
pass=0
i=1
while [ "$i" -le "$rounds" ]; do
  log="$out/$tag-$i.log"
  # 喂键器（照 `soak.sh` 那一支）：`echo` 打完读数之后**在等控制台的 `exit`**（它就是交互回显），
  # 故这里等它的收尾读数出现，再喂一条 `exit`（第二条是保险；撞上断开的管道是预期的收尾）。
  (
    waited=0
    while [ "$waited" -lt 90 ] && ! grep -q "echo: seq=0" "$log" 2>/dev/null; do
      sleep 1
      waited=$((waited + 1))
    done
    echo exit
    sleep 3
    echo exit
  ) 2>/dev/null | SQWARE_ROOT=fair timeout 300 cargo run $prof > "$log" 2>&1
  reason=""
  if ! grep -q "task: all tasks exited, system halted" "$log"; then
    reason="无停机行（末行：$(grep -a 'fair\|probe-deep\|echo:' "$log" | tail -1)）"
  elif ! grep -q "probe-deep: tree deep=" "$log"; then
    reason="聊天客人没跑完"
  elif ! grep -qE "^\[case\] probe-deep: 5 cases[[:space:]]*$" "$log"; then
    reason="聊天客人的用例没登记全（$(grep -a '^\[case\]' "$log" | head -1)）"
  elif ! grep -qE "^\[case\] probe-deep: cases 5 ok 5 fail 0[[:space:]]*$" "$log"; then
    reason="聊天客人的用例没过：$(grep -a '^\[case\] run ' "$log" | tail -1)"
  elif ! grep -q "echo: list root=0,3" "$log"; then
    reason="受害者被挤过期：$(grep -a 'echo:' "$log" | tr '\n' ' ' | tail -c 150)"
  elif ! grep -q "echo: list names=sys,device" "$log"; then
    reason="受害者被挤过期（names）：$(grep -a 'echo: list names=' "$log" | tail -1)"
  else
    pass=$((pass + 1))
  fi
  if [ -n "$reason" ]; then
    echo "round $i: FAIL $reason"
  else
    echo "round $i: 受害人读数照旧 · $(grep -a 'probe-deep: tree deep=' "$log")"
  fi
  i=$((i + 1))
done
echo "== 公平 $pass/$rounds（日志 $out/$tag-*.log）=="
[ "$pass" -eq "$rounds" ]
