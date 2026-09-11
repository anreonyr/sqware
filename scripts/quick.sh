#!/bin/bash
# quick — 开发期的**单次冷启 + 往返计时**（不做判定；判定在 `examine.nu`）。
#
# 为什么要有它：整门是 3 轮 QEMU + 两档 profile 的**验收**判据，定位与调优太贵。
# 本脚本只做三件事：起一次 guest、等提示符再喂命令、**打印每条命令的往返耗时**。
# 永远 exit 0——"什么算通过"不在这里。
#
# 用法：
#   scripts/quick.sh                        # 默认量 `dir`
#   scripts/quick.sh dir "req echo"
#   QUICK_ELF=<path> scripts/quick.sh
#
# **它不做判定，也不假装能计时**：先后两版自动计时都被输入回显的重复提示符骗了
# （`sq > ` 每次输入出现两次：写一次、重绘一次）。所以它只负责"起一次、喂命令、
# 留一份 transcript"——快慢由人看，通过与否归 `examine.nu`。
#
# 默认**先重建产物**（不重建就会跑旧 ELF）。SKIP_BUILD=1 跳过。
set -u

built=target/riscv64gc-unknown-none-elf/release/sqware
elf=${QUICK_ELF:-$([ -f "$built" ] && echo "$built" || ls -td trace/e2e-*/elf-default/sqware 2>/dev/null | head -1)}
if [ -z "${elf:-}" ] || [ ! -f "$elf" ]; then
    echo "quick: 找不到 ELF；先跑一次 examine.nu，或给 QUICK_ELF=<path>" >&2
    exit 0
fi
cmds=("$@")
[ ${#cmds[@]} -eq 0 ] && cmds=("dir")

mkdir -p trace
log="trace/quick-$(date +%Y%m%d-%H%M%S).log"
echo "quick: elf=$elf"

# 重建（内核 build.rs 会连带重建 programs 并重打 initrd）
if [ "${SKIP_BUILD:-0}" != "1" ]; then
    echo "quick: 重建中（SKIP_BUILD=1 可跳过）..."
    if ! cargo build --release 2>/dev/null; then
        echo "quick: cargo build 失败（只读策略？）" >&2
        exit 0
    fi
fi

# 提示符 `sq > ` 在**每次输入会出现两次**（写一次、重绘一次），故不能用它计数。
# 稳定可数的是 `clock <秒>` 行——计时序列正好由它夹住。
clocks() { local n; n=$(grep -ac 'clock [0-9.]* sec' "$log" 2>/dev/null); echo "${n:-0}"; }
prompts() { local n; n=$(grep -ac 'sq > ' "$log" 2>/dev/null); echo "${n:-0}"; }
wait_clocks() {           # 等第 n 行 clock（最多 25 s）
    local want=$1 i
    for i in $(seq 1 500); do
        [ "$(clocks)" -ge "$want" ] && return 0
        sleep 0.05
    done
    return 1
}
wait_prompt() {           # 等第 n 次提示符（最多 25 s）——只用于等启动
    local want=$1 i
    for i in $(seq 1 500); do
        [ "$(prompts)" -ge "$want" ] && return 0
        sleep 0.05
    done
    return 1
}

# fd 3 = 喂给 QEMU 的 stdin（显式握管，免得跟管道/tee 的 fd 混起来）
exec 3> >(QEMU_TIMEOUT=45 nu scripts/boot.nu "$elf" 2>&1 | tee "$log" >/dev/null)
feed=$!

if ! wait_prompt 1; then echo "quick: 等不到提示符（启动失败？）" >&2; fi
echo "quick: 启动完成（$(prompts) 次提示符）"

# 依次喂命令（不等"第几次提示符"——那个计数不可靠），跑完留 transcript。
for c in "${cmds[@]}"; do
    printf '%s\n' "$c" >&3
    sleep 1.5
done
sleep 1
printf 'exit\n' >&3
sleep 2
exec 3>&-
wait "$feed" 2>/dev/null
grep -q 'system halted' "$log" && echo "quick: 正常自退" || echo "quick: 未见停机行"
# 留一份 transcript 供人看：它是本脚本唯一的产出物（放在工作区，别放 /tmp——会清）
echo "quick: transcript = $log" 
