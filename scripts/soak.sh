#!/bin/sh
# 收尾口径验收门 —— 会话被控制台 `exit` 结束后，必须真的走到停机。
#
# 为什么单独一条门：`echo` 读到一行 `exit` 才退场，root 的 `wait_last` 才返回、才有
# `root: done` 与级联收尾。**不喂 stdin 的跑法永远进不了收尾**（服务常驻，QEMU 一直
# 闲等被 timeout 杀掉）——此前"5 次冷跑全过"验的只是启动读数，这一格从未被覆盖。
#
# 判据（两条一起）：
#   1) 日志里出现 `task: all tasks exited, system halted`；
#   2) 八条启动读数仍在（`router: tree part=0 land=0 find=0 got=true` / `router: desk guest` /
#      `router: ndev=95 ctx=1` / `uart: serial@10000000 ier=rx` / `guest: reg=0 find=0` /
#      `answer=router` / `guest: trip ok` / `echo: ready`）。
#
# 这两条是**驱动侧那两条**：控制器自报 95 条线、本域用的 context 是 1；串口那一台已经把
# 设备拿在手里、把"收到字节就拉线"打开（`serial@10000000` 那枚 `ONLY` 门闩换了主人）。
# 前者还是"设备树解码还对"的一条判据：解码一改错，这一格先红。
#
# `router: tree part=0 land=0 find=0 got=true` 是**门牌**那一条：驱动自己把入口落到
# `/device/router` 上、再查回来验一遍（`part=0` = 那块目录是它建的；第二台上来会读成 2）。
# 而 `guest: reg=0 find=0` 里那个 `find` 是**从树上问到的**——按名找服务今天归树，板管生死。
#
# 用法：
#   scripts/soak.sh [轮数] [--release]      # 默认 10 轮，debug 档
# 退出码：全过 0，有不过 1。日志落在 target/soak/soak-<时间戳>-<轮>.log。
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

rounds="${1:-10}"
[ "$#" -ge 1 ] && shift
prof=""
[ "${1:-}" = "--release" ] && prof="--release"
out=target/soak
mkdir -p "$out"
tag="soak-$(date +%s)"
pass=0
i=1
while [ "$i" -le "$rounds" ]; do
  log="$out/$tag-$i.log"
  ( sleep 5; echo exit; sleep 3; echo exit ) | timeout 15 cargo run $prof > "$log" 2>&1
  if ! grep -q "task: all tasks exited, system halted" "$log"; then
    echo "round $i: FAIL 无停机行；$(grep -a '\[stop\]' "$log" | head -1)"
  elif ! { grep -q "router: tree part=0 land=0 find=0 got=true" "$log" \
        && grep -q "router: desk guest" "$log" \
        && grep -q "router: ndev=95 ctx=1" "$log" \
        && grep -q "uart: serial@10000000 ier=rx" "$log" \
        && grep -q "guest: reg=0 find=0" "$log" \
        && grep -q "answer=router" "$log" \
        && grep -q "guest: trip ok" "$log" \
        && grep -q "echo: ready" "$log"; }; then
    echo "round $i: FAIL 启动读数不全（$log）"
  else
    pass=$((pass + 1))
    echo "round $i: PASS"
  fi
  i=$((i + 1))
done
echo "== 通过 $pass/$rounds（日志 $out/$tag-*.log）=="
[ "$pass" -eq "$rounds" ]
