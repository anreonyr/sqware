#!/bin/sh
# 共享组台 —— 「一次投信，两个等待者都该醒；而那条消息只能归一个人」那一格的可测化。
#
# 为什么单独一条：共享组的整链放行在树里没有别的用法——`root` / `board` 各独享一只
# **独占**组（链长恒 ≤ 1 ⇒ "放行全链"与"放行一人"是同一件事），验收门与压测台都经过
# 不了那条新路。这里换引导镜像（`SQWARE_ROOT=group`），台主造**两个真正挂在同一只组键上**
# 的等待者，再投一次信。
#
# 判据（两条一起）：
#   1) 台主汇总行 `group: PASS`（内含 `hung=2 woke=2 deliver=true control=true`）；
#   2) 出现 `task: all tasks exited, system halted`（收完自己停，不是被 timeout 杀掉）。
#
# 读数（末行汇总）：
#   hung    = 挂上并报到的等待者数（要 2：组键上**真的**有两个等待者）
#   woke    = 一次投信后**被放行**的等待者数（要 2：整链放行；退回单播时是 1——
#             判据用**非破坏性**的 `peek`，理由见台子头注的照实记）
#   deliver = 那条消息只归一个人（台主取一次成功、再取答 `Busy`）
#   control = **独占组**同样两次 accord 的第二次被拒（共享可复制、独占不可）
# 「两个等待者都退场了」不在这行读数里：它由停机行担保（`Join` 对已回收干净的任务答
# `Denied`，数不出这个数——见台子头注的照实记）。
#
# 用法：
#   scripts/group.sh [轮数]           # 默认 3 轮
# 退出码：全过 0，有不过 1。日志落在 target/group/group-<时间戳>-<轮>.log。
set -u

# ── 环境对齐（必须）：显式**关掉** icount ──
#
# 与验收门 / 压测台同一条理由：`boot.nu` 默认 `-icount auto,sleep=on` 按宿主时间给 vCPU
# 记账 ⇒ WFI 里的核被 IPI 叫醒要等额度（毫秒级）。本台子的判据含"两人都醒"，
# 唤醒及时性正是被测对象，故必须与门同环境。
QEMU_ICOUNT=
export QEMU_ICOUNT

rounds="${1:-3}"
out=target/group
mkdir -p "$out"
tag="group-$(date +%s)"
pass=0
i=1
while [ "$i" -le "$rounds" ]; do
  log="$out/$tag-$i.log"
  # 时限 120 s：本台子没有轮次循环，起两台子域 + 200 ms 稳压 + 两次有界等待 ⇒ 秒级，
  # 卡住就是要红了（那时 timeout 杀掉，判据自然不过）。
  SQWARE_ROOT=group timeout 120 cargo run > "$log" 2>&1
  verdict="$(grep -a 'group: hung=' "$log" | tail -1)"
  if ! grep -q "group: PASS" "$log"; then
    echo "round $i: FAIL 台主未判 PASS（读数：${verdict:-无}）"
  elif ! grep -q "task: all tasks exited, system halted" "$log"; then
    echo "round $i: FAIL 无停机行（读数：${verdict:-无}）"
  else
    pass=$((pass + 1))
    echo "round $i: $verdict"
  fi
  i=$((i + 1))
done
echo "== 通过 $pass/$rounds（日志 $out/$tag-*.log）=="
[ "$pass" -eq "$rounds" ]
