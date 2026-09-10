#!/usr/bin/env bash
# sqware e2e 验收门。
#
# 三轮判据，缺一不可：
#   1) 自行退出：qemu 自己结束（停机走 srst），不是被 host 侧 timeout 杀掉
#      —— 捕获里出现 `terminating on signal … (/usr/bin/timeout)` 即判失败。
#      这正是「偶发挂在停机前」以前不可见的地方（那时靠人眼比对，超时被杀与正常
#      结束在脚本层面同形）。
#   2) 无崩溃：捕获里无 `[panic] at`（内核 panic 报告头）。
#   3) 语义：九个 marker 逐条出现（人眼比对改成脚本比对）。
#
# 每轮独立 TRACE_OUT 目录，终端输出另存 console.log —— 捕获不再被下一轮删除，
# 偶发失败（同产物时而 8/9）因此可复盘。
#
# **默认不开 semihosting**：semihosting + `-icount auto` 下诊断事件流（每个 room
# park/wake/envcall 一条）经 ebreak 落宿主文件，实测 guest 虚拟时间被拖慢约 650×
# （60 s 墙钟只推进 92 ms、5245 条事件），e2e 在超时前走不完。结构化导出只适合
# 小规模短跑；e2e 判据因此取「自行退出 + 无 panic + marker」，不依赖导出。
#
# 用法：scripts/e2e.sh                     # 默认连跑 3 次，要求 3/3
#       E2E_REPEAT=1 scripts/e2e.sh
#       E2E_FEATURES=audit scripts/e2e.sh  # 审计档（fence 记账检查）
set -uo pipefail

cd "$(dirname "$0")/.."

REPEAT=${E2E_REPEAT:-3}
FEATURES=${E2E_FEATURES:-}
OUT=${E2E_OUT:-"trace/e2e-$(date +%Y%m%d-%H%M%S)"}
QEMU_TIMEOUT=${QEMU_TIMEOUT:-80}

# 九个 marker（正则）。`clock` 那条要避开 shell 的逐键回显（`sq > clock`），故要求后随数字。
MARKERS=(
  'spawnjoin -> 499500'
  'discover echo -> found'
  'req echo -> "ifmmp\.tfswjdf'
  'hole got "hi from shell'
  'sleep 300ms'
  'woke'
  'clock [0-9]'
  'badslot: 3/3 rejected, kernel alive'
  'task: all tasks exited, system halted'
)

# 输入时序（与既有基线一致；改这里等于改门，改完必须 3/3 复验）。
drive() {
  sleep 8;  printf 'spawn\n'
  sleep 3;  printf 'dir\n'
  sleep 2;  printf 'req\n'
  sleep 2;  printf 'hole\n'
  sleep 2;  printf 'sleep 300\n'
  sleep 3;  printf 'clock\n'
  sleep 2;  printf 'badslot\n'
  sleep 2;  printf 'exit\n'
}

echo "e2e: features='${FEATURES:-<none>}' repeat=$REPEAT out=$OUT timeout=${QEMU_TIMEOUT}s"
mkdir -p "$OUT"

# 先建一次，别把构建时间算进第 1 轮；features 为空时不传该参数。
build=(cargo build --release -p kernel)
[ -n "$FEATURES" ] && build+=(--features "$FEATURES")
"${build[@]}" || exit 1

pass=0
for i in $(seq 1 "$REPEAT"); do
  dir="$OUT/run$i"
  mkdir -p "$dir"
  run=(cargo run --release)
  [ -n "$FEATURES" ] && run+=(--features "$FEATURES")
  drive | TRACE_OUT="$dir" QEMU_TIMEOUT="$QEMU_TIMEOUT" QEMU_EXPECT=halt \
    "${run[@]}" 2>&1 | tee "$dir/console.log"
  rc=${PIPESTATUS[1]}

  why=""
  grep -q 'terminating on signal' "$dir/console.log" && why="$why 被超时杀"
  grep -q '\[panic\] at' "$dir/console.log" && why="$why 内核panic"
  for m in "${MARKERS[@]}"; do
    grep -Eq "$m" "$dir/console.log" || why="$why 缺[$m]"
  done
  [ "$rc" -ne 0 ] && why="$why runner退码$rc"

  if [ -z "$why" ]; then
    pass=$((pass + 1))
    echo "run $i: PASS (自退 + 无 panic + marker 9/9)"
  else
    echo "run $i: FAIL —$why —— 现场留在 $dir"
  fi
done

echo "e2e: $pass/$REPEAT"
[ "$pass" -eq "$REPEAT" ]
