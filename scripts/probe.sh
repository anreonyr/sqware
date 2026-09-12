#!/bin/bash
# probe — 带时间戳的最小驱动：把「何时喂」与「客人何时退」都记进 transcript。
#
# 为什么不用 fast.sh：它把喂入放在一个后台块里，一旦出问题就无从判断是
# 「字符被吃」还是「命令没执行」还是「客人自己退了」——三种现象在 transcript
# 里长得一样（都是"没有输出"）。本脚本每步打一条 [probe] 时间戳行。
#
# 用法：scripts/probe.sh "churn 4 1 1"        （命令，逐条）
#       PROBE_SETTLE=3 scripts/probe.sh ...   （等启动的秒数）
set -u
elf=${PROBE_ELF:-target/riscv64gc-unknown-none-elf/release/sqware}
mem=${QEMU_MEM:-64}
settle=${PROBE_SETTLE:-3}
deadline=${PROBE_TIMEOUT:-60}
log=${PROBE_LOG:-trace/probe-$(date +%H%M%S).log}
fifo=$(mktemp -u /tmp/sqprobe.XXXXXX)
mkdir -p "$(dirname "$log")"
: > "$log"
mkfifo "$fifo"
cleanup() { exec 3>&- 2>/dev/null; rm -f "$fifo"; }
trap cleanup EXIT

QEMU_MEM="$mem" QEMU_TIMEOUT="$deadline" QEMU_ICOUNT= \
    nu scripts/boot.nu "$elf" <"$fifo" >>"$log" 2>&1 &
qpid=$!
exec 3>"$fifo"

t0=$(date +%s%N)
stamp() { printf '[probe] +%dms %s\n' "$(( ($(date +%s%N) - t0) / 1000000 ))" "$1" >>"$log"; }
stamp "qemu up pid=$qpid"

sleep "$settle"
stamp "settle done, feeding"
printf '\n' >&3
for c in "$@"; do
    printf '%s\n' "$c" >&3
    stamp "fed: $c"
done
printf 'exit\n' >&3
stamp "fed: exit"

for _ in $(seq 1 $((deadline * 10))); do
    grep -aq 'system halted' "$log" 2>/dev/null && { stamp "saw 'system halted'"; break; }
    kill -0 "$qpid" 2>/dev/null || { stamp "qemu exited"; break; }
    sleep 0.1
done
sleep 0.3
exec 3>&- 2>/dev/null
wait "$qpid" 2>/dev/null
stamp "done rc=$?"
echo "probe: log=$log"
