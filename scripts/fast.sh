#!/bin/bash
# fast — 迭代用的**最小启动往返**：起一次 guest、喂命令、**等真的停机**再看结果。
#
# 为什么不直接用 `quick.sh`：那个面向"验收式的一次往返"（每轮重建 + 逐条等
# 提示符 + 每条命令后 `sleep 1.5`），一轮 5–6 s。定位期要的是"几秒一轮"。
#
# # 两条硬约束（本会话两次踩坑，都是**驱动**的坑，不是内核的）
#
# ① **stdin 不关，`boot.nu` 就不退出**——即使 guest 早已 `system halted`。
#    先前每条命令都写成 `{ printf ...; sleep N; } | ...`，那个 `sleep N` 一直
#    握着写端，于是**每轮精确耗时 N 秒**（60/240/300 s），与 guest 里发生什么
#    **毫无关系**。guest 本身是快的：`churn 400 1 1` 约 0.2 s。
#
# ② **反过来也不行**：写完立刻关管道 = 送 EOF，guest 可能**没跑完那条命令就退**。
#    实测 `churn 100 1 1 1` 静默消失、只剩 `system halted`；还见过启动期被吃掉
#    字符的 `unknown: hurn`。
#
# 故：既不能"握着不放"，也不能"写完就撒手"——**看输出决定何时撒手**。
# 实现：QEMU 的 stdin 接一条 FIFO；轮询 transcript，见到 `system halted`
# 或 QEMU 退出就收工。启动约 1 s，`FAST_SETTLE` 默认 2.5 s 略宽于启动，
# 免得踩在启动期喂（那时字符会被吃）。
#
# # 用法
#
#   scripts/fast.sh                       # 只启动 + exit
#   scripts/fast.sh "churn 400 1 1"       # 喂一条命令
#   QEMU_MEM=64 scripts/fast.sh "..."     # 换内存
#   SKIP_BUILD=1 scripts/fast.sh "..."    # 已 build 过
#   FAST_FEATURES=audit scripts/fast.sh   # 带 feature 重建
#
# 结果落在 `$log`（默认 `trace/fast-<时分秒>.log`），stdout 只是回声。
# **不做判定**：通过与否由调用者看 transcript 决定。
set -u

elf=target/riscv64gc-unknown-none-elf/release/sqware
mem=${QEMU_MEM:-64}
settle=${FAST_SETTLE:-2.5}
deadline=${FAST_TIMEOUT:-60}
log=${FAST_LOG:-trace/fast-$(date +%H%M%S).log}
fifo=$(mktemp -u /tmp/sqfast.XXXXXX)

if [ "${SKIP_BUILD:-0}" != "1" ]; then
    # shellcheck disable=SC2086
    cargo build --release ${FAST_FEATURES:+--features "$FAST_FEATURES"} >/dev/null 2>&1 || {
        echo "fast: build 失败" >&2
        exit 1
    }
fi
[ -f "$elf" ] || { echo "fast: 找不到 $elf" >&2; exit 1; }

mkdir -p "$(dirname "$log")"
: > "$log"
mkfifo "$fifo"
cleanup() { exec 3>&- 2>/dev/null; rm -f "$fifo"; }
trap cleanup EXIT

# QEMU 的 stdin = FIFO（不经 shell、不经管道缓冲）。
QEMU_MEM="$mem" QEMU_TIMEOUT="$deadline" QEMU_ICOUNT= \
    nu scripts/boot.nu "$elf" <"$fifo" >"$log" 2>&1 &
qpid=$!
exec 3>"$fifo"          # 打开写端（此刻 QEMU 才开始读到东西）

# 喂入：先等启动，再逐条。写完**不关** fd 3——由下面的等待循环决定何时关。
{
    sleep "$settle"
    printf '\n\n'               # 领头字符会被吃（实测：`churn` → `hurn`），喂两行兜住
    for c in "$@"; do printf '%s\n' "$c"; done
    printf 'exit\n'
    sleep "$deadline"           # 占位：真正的关闭由主流程做
} >&3 &
feeder=$!

# 边看边等：见到停机行、或 QEMU 自己退出，即收工。
for _ in $(seq 1 $((deadline * 10))); do
    grep -aq 'system halted' "$log" 2>/dev/null && break
    kill -0 "$qpid" 2>/dev/null || break
    sleep 0.1
done
sleep 0.3                     # 让尾部输出落盘
exec 3>&- 2>/dev/null          # 关写端 → QEMU 收 EOF → 退出
kill "$feeder" 2>/dev/null
wait "$qpid" 2>/dev/null
rc=$?

echo "fast: qemu-exit=${rc} mem=${mem} log=${log}"
exit 0
