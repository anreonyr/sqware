#!/bin/bash
# quick — 开发期的**单次冷启 + 往返计时**（不做判定；判定在 `examine.nu`）。
#
# 为什么要有它：整门是 3 轮 QEMU + 两档 profile 的**验收**判据，定位与调优太贵。
# 本脚本只做三件事：起一次 guest、等启动行再喂命令、**打印每条命令的往返耗时**。
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

# 启动读数用 `echo: ready`——当前树里**唯一稳定可数的启动行**：它由 `echo` 域在注册
# 回显之前从调试面直打（不经过任何服务），一次启动只出现一次，且验收门的第 3 条判据
# 就是它（`examine.nu` / `soak.sh` 同源）。旧的 `sq > ` 与 `clock <秒>` 出自已删的
# console 协议，今天没有任何源码会打，拿它们计数只会每次都空等。
ready() { local n; n=$(grep -ac 'echo: ready' "$log" 2>/dev/null); echo "${n:-0}"; }
wait_ready() {            # 等 `echo: ready`（最多 25 s）——只用于等启动
    local i
    for i in $(seq 1 500); do
        [ "$(ready)" -ge 1 ] && return 0
        sleep 0.05
    done
    return 1
}

# fd 3 = 喂给 QEMU 的 stdin（显式握管，免得跟管道/tee 的 fd 混起来）
# 与验收门对齐：显式关掉 icount（默认 `auto,sleep=on` 会把 WFI 唤醒按宿主时间节流到
# 毫秒，往返计时与真相差一个数量级）。
exec 3> >(QEMU_TIMEOUT=45 QEMU_ICOUNT= nu scripts/boot.nu "$elf" 2>&1 | tee "$log" >/dev/null)
feed=$!

if ! wait_ready; then echo "quick: 等不到 echo: ready（启动失败？）" >&2; fi
echo "quick: 启动完成"

# 依次喂命令（**固定间隔喂，不等回应**——回应不作判据），跑完留 transcript。
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
