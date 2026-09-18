#!/bin/sh
# 压测台 —— 「杀下去多久收干净」那一格的可测化（见 `programs/src/bin/stress/rig.rs`）。
#
# 为什么单独一条门：生产场景（`scripts/soak.sh`）每个 boot 只在收尾那一刀杀**一次**，
# 那一格的率是 ~1% 量级 ⇒ 几十轮一批也说明不了问题。这里换引导镜像（`SQWARE_ROOT=rig`），
# 每个 boot 反复"造 → 放行 → 按时序空转 → 杀 → 判"，做数百次试验。
#
# 判据（两条一起）：
#   1) 出现汇总行 `rig: total ...`；
#   2) 出现 `task: all tasks exited, system halted`（收完自己停，不是被 timeout 杀掉）。
#
# 读数里那四类（每一档一行，末尾一行汇总）：
#   now     = 杀令之前就收尾了（当场摘掉）
#   waited  = 问时还没收，等到收尾事件后复探确认
#   late    = 判定窗口（300 ms）内没收掉，宽限（1 s）内收了
#   lost    = 两个窗口都没收掉 —— **"他杀没生效"**，这就是要数的那个数
#
# 用法：
#   scripts/stress.sh [轮数]          # 默认 3 轮（每轮 ~168 次试验）
# 退出码：全过 0，有不过 1。日志落在 target/stress/stress-<时间戳>-<轮>.log。
set -u

# ── 环境对齐（必须）：显式**关掉** icount ──
#
# `scripts/boot.nu` 的默认是 `-icount auto,sleep=on`：按宿主时间给 vCPU 记账、让它睡够
# 虚拟额度 ⇒ **WFI 里的核被 IPI 叫醒要等额度（实测毫秒级）**。验收门（`scripts/examine.nu`）
# 一直是关着 icount 跑的，`fast.sh` / `probe.sh` 也是；台子与忙机台此前没关 ⇒ 两边读数
# **不可比**（照实记：rig A 的 `starved` 在 icount 开时是 317/328，关掉后是 1~3/328；
# 同一颗 ELF、同一条命，只差这一个开关）。故这里与门对齐。
QEMU_ICOUNT=
export QEMU_ICOUNT

rounds="${1:-3}"
out=target/stress
mkdir -p "$out"
tag="stress-$(date +%s)"
pass=0
i=1
while [ "$i" -le "$rounds" ]; do
  log="$out/$tag-$i.log"
  # 时限放宽到 300 s：rig A 之后每一轮都要"造 → 握手（认下它交回来的孔）→ 唤醒 → 杀 → 判"，
  # 而判决窗口是 300 ms + 1 s 宽限 ⇒ 命中 `late`/`lost` 的那些轮本来就慢。
  SQWARE_ROOT=rig timeout 300 cargo run > "$log" 2>&1
  if ! grep -q "rig: total" "$log"; then
    echo "round $i: FAIL 无汇总行（末行：$(grep -a 'rig:' "$log" | tail -1)）"
  elif ! grep -q "task: all tasks exited, system halted" "$log"; then
    echo "round $i: FAIL 无停机行"
  else
    pass=$((pass + 1))
    echo "round $i: $(grep -a 'rig: total' "$log")"
  fi
  i=$((i + 1))
done
echo "== 通过 $pass/$rounds（日志 $out/$tag-*.log）=="
[ "$pass" -eq "$rounds" ]
