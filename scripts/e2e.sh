#!/usr/bin/env bash
# sqware e2e 验收门。
#
# 判据四条，缺一不可：
#   1) 逐步生效：八条命令**逐个等它的输出出现**再发下一条（expect 式），不是按墙钟
#      盲排；每步都有独立超时，失败能指到具体哪一条。
#   2) 自行退出：qemu 自己结束（停机走 srst）⇒ timeout 的退出码不是 124；
#      捕获里也不该出现 `terminating on signal … (/usr/bin/timeout)`。
#   3) 无崩溃：捕获里无 `[panic] at`（内核 panic 报告头）。
#   4) 九个 marker 齐全（含 `task: all tasks exited, system halted`）。
#
# **为什么不走 `cargo run` / runner.nu**（实测，见 docs/audit-flying-wires.md §9.3）：
#   `cargo run` 的 runner 链是「cargo → nu → timeout → qemu」，输入要穿过 nu 的 stdin。
#   实测同样八条命令、同样 FIFO、同样时序：
#     - 直连 qemu：3 轮 **24/24** 步全部生效；
#     - 走 runner：**1/3** 通过，且在随机一步之后输入再也不生效（guest 正常停在提示符上）。
#   runner 只是 `cargo run` 的便利层，门不必依赖它；门自己起 qemu，输入由门持有的 FIFO
#   直接给 qemu，`exec 3>` 阻塞到 qemu 打开读端 ⇒ 输入与「qemu 就绪」天然同步。
#   代价：QEMU 参数与 runner 里那份**重复**了（已知债，改一处要改两处）。
#
# **默认不开 semihosting**：semihosting + `-icount auto` 下诊断事件流经 ebreak 落宿主
# 文件，实测把 guest 虚拟时间拖慢约 650×（60 s 墙钟只推进 92 ms、5245 条事件），e2e 在
# 超时前走不完。故结构化导出只适合小规模短跑，不进门。
#
# 用法：scripts/e2e.sh                     # 默认连跑 3 次，要求 3/3
#       E2E_REPEAT=1 scripts/e2e.sh
set -uo pipefail

cd "$(dirname "$0")/.."

REPEAT=${E2E_REPEAT:-3}
OUT=${E2E_OUT:-"trace/e2e-$(date +%Y%m%d-%H%M%S)"}
QEMU_TIMEOUT=${QEMU_TIMEOUT:-60}
STEP_WAIT=${E2E_STEP_WAIT:-15}     # 单步等待上限（秒）
T_BOOT=${E2E_T_BOOT:-3}            # 首条命令前的余量（guest 实测 3 s 内到提示符）
T_GAP=${E2E_T_GAP:-2}              # 每条命令之间的让出

ELF=target/riscv64gc-unknown-none-elf/release/sqware
INITRD=target/riscv64gc-unknown-none-elf/release/initrd.img

# 步骤：命令 → 该步要看到的输出。最后一条同时是自然停机的判据。
STEPS=(
  'spawn|spawnjoin -> 499500'
  'dir|discover echo -> found'
  'req|req echo -> "ifmmp\.tfswjdf'
  'hole|hole got "hi from shell'
  'sleep 300|woke'
  'clock|clock [0-9]'
  'badslot|badslot: 3/3 rejected, kernel alive'
  'exit|task: all tasks exited, system halted'
)
# 全量 marker（含 sleep 探针的 `sleep 300ms`），跑完逐条核。
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

echo "e2e: repeat=$REPEAT out=$OUT qemu_timeout=${QEMU_TIMEOUT}s step_wait=${STEP_WAIT}s"
mkdir -p "$OUT"

cargo build --release -p kernel || exit 1
[ -f "$ELF" ] || { echo "缺 $ELF"; exit 1; }

# 等 marker 出现在日志里；$1=正则 $2=上限秒 $3=步骤名。失败返回 1。
expect() {
  local pat=$1 limit=$2 step=$3 start=$SECONDS
  while [ $((SECONDS - start)) -lt "$limit" ]; do
    grep -Eq "$pat" "$LOG" && return 0
    sleep 0.2
  done
  echo "  步骤[$step] 超时（${limit}s 内没等到 /$pat/）"
  return 1
}

pass=0
for i in $(seq 1 "$REPEAT"); do
  dir="$OUT/run$i"; mkdir -p "$dir"
  LOG="$dir/console.log"; FIFO="$dir/in.fifo"
  : > "$LOG"; rm -f "$FIFO"; mkfifo "$FIFO"
  seed=$((RANDOM * 32768 + RANDOM))

  ( timeout "$QEMU_TIMEOUT" qemu-system-riscv64 \
      -machine virt -bios SBI.bin -kernel "$ELF" -nographic -no-reboot \
      -m 128 -smp 4 -seed "$seed" -icount auto,sleep=on -initrd "$INITRD" \
      < "$FIFO" > "$LOG" 2>&1 ) &
  QPID=$!
  exec 3> "$FIFO"        # 阻塞直到 qemu 持有读端 ⇒ 与「qemu 就绪」同步

  why=""; sent=0
  # 首条前留引导余量（上一步的 expect 已保证 guest 到提示符则无需再等）。
  expect 'sq > ' "$STEP_WAIT" 'boot' || why="引导/提示符"
  for s in "${STEPS[@]}"; do
    cmd=${s%%|*}; pat=${s#*|}
    [ -n "$why" ] && break
    sleep "$T_GAP"
    if printf '%s\n' "$cmd" >&3 2>/dev/null; then sent=$((sent + 1)); else why="输入写失败($cmd)"; break; fi
    expect "$pat" "$STEP_WAIT" "$cmd" || why="步骤 $cmd"
  done
  exec 3>&-
  wait "$QPID"; rc=$?

  [ "$sent" -ne "${#STEPS[@]}" ] && why="${why:-}只写出 $sent/${#STEPS[@]} 条"
  [ "$rc" -eq 124 ] && why="${why:+$why +}被超时杀"
  grep -q 'terminating on signal' "$LOG" && why="${why:+$why +}被超时杀"
  grep -q '\[panic\] at' "$LOG" && why="${why:+$why +}内核panic"
  for m in "${MARKERS[@]}"; do
    grep -Eq "$m" "$LOG" || why="${why:+$why +}缺[$m]"
  done

  if [ -z "$why" ]; then
    pass=$((pass + 1))
    echo "run $i: PASS (自退 + 无 panic + 9 步全过 + 9 marker 齐)"
  else
    echo "run $i: FAIL — $why —— 现场留在 $dir"
  fi
done

echo "e2e: $pass/$REPEAT"
[ "$pass" -eq "$REPEAT" ]
