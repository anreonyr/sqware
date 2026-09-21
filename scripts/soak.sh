#!/bin/sh
# 收尾口径验收门 —— 会话被控制台 `exit` 结束后，必须真的走到停机。
#
# 为什么单独一条门：`echo` 读到一行 `exit` 才退场，root 的 `wait_last` 才返回、才有
# `root: done` 与级联收尾。**不喂 stdin 的跑法永远进不了收尾**（服务常驻，QEMU 一直
# 闲等被 timeout 杀掉）——此前"5 次冷跑全过"验的只是启动读数，这一格从未被覆盖。
#
# 判据（两条一起）：
#   1) 日志里出现 `task: all tasks exited, system halted`；
#   2) 十七条启动 / 装配读数仍在（`router: tree part=0 land=0 find=0 got=true` / `router: desk guest` /
#      `router: ndev=95 ctx=1` / `uart: serial@10000000 ier=rx` / `guest: reg=0 find=0` /
#      `answer=router` / `guest: trip ok` / `echo: ready` /
#      **`router: line 10 = serial@10000000`** / **`uart: rang n=`** /
#      **`uart: tree part=2 land=0 find=0 got=true`** / **`echo: console=true`** /
#      **`router: line 11 = rtc@101000`** / **`router: vacate line=11`** /
#      **`lodger: taken=2`** / **`lodger: unknown=1`** / **`router: lane dropped line=11`**）；
#   3) 收尾摘要那一行 **`irq: ring=…`** 仍在——外部中断那枚铃的读数：摇了几次 / 其中几次
#      "还响着" / 其中**空闲核补摇**了几支（见下）。
#
# 这两条是**驱动侧那两条**：控制器自报 95 条线、本域用的 context 是 1；串口那一台已经把
# 设备拿在手里、把"收到字节就拉线"打开（`serial@10000000` 那枚 `ONLY` 门闩换了主人）。
# 前者还是"设备树解码还对"的一条判据：解码一改错，这一格先红。
#
# `router: tree part=0 land=0 find=0 got=true` 是**门牌**那一条：驱动自己把入口落到
# `/device/router` 上、再查回来验一遍（`part=0` = 那块目录是它建的；第二台上来会读成 2）。
# 而 `guest: reg=0 find=0` 里那个 `find` 是**从树上问到的**——按名找服务今天归树，板管生死。
#
# 四条是**线 + 控制台**那一刀（`protocol::driver::line` 的四格与设备持有者那枚服务孔）：
# `router: line 10 = serial@10000000` = **登记**——串口驱动报设备名、路由者**解树**（线 = 名字的
# 函数）并把这条线接上（起域时一条都不接）；`uart: rang n=` = **投递**——那一帧真的到了
# 客户手里（那一行不带线号：线在泊位里，见 `protocol::driver::line`）；`uart: tree part=2 land=0 find=0 got=true` = **服务门牌**——`uart` 把"读行"那枚孔
# 落到 `/device/uart`（`part=2` 是"那块目录已经在了"，`router` 先建的）；`echo: console=true`
# = **客人真的从那枚孔拿到了副本**（它是回显的来路，从前是内核的调试面）。
#
# 第三格 `exhaust`（排空）**已经接上真内容**：读口搬到设备持有者之后，客户是真的读走了设备里的
# 字节才说那句话，路由者据此把线放回（`router: exhaust line=10`）——那一行只在"真的排空过"时
# 出现，故它归 `examine.nu` 那条回显判据一起看，不在这里当固定读数（喂不喂键决定它有没有）。
#
# 第四格 `vacate`（收线）**在这里是固定读数**了：`router: line 11 = rtc@101000` 与
# `router: vacate line=11` 是**房客**（`prog-lodger`）那一对——它真领了那时钟那一页的门闩、
# 占住 11 号线，然后**一句话不说就走**（不 `DISMISS`、不 `vacate`）。路由者每次醒来先探活
# （`alive` 答不出的那几条拆线 + 空出格子）⇒ 收线那一手第一次有了读数，而这一对只在
# "先占上、后没了"这条路上出现（喂不喂键都要有它：它跑在装配期）。
#
# 另外两条（`lodger: taken=2` / `lodger: unknown=1`）是**失败域**那两格：房客占下 11 号线之后
# 拿**同一条线**再来一次（答 `TAKEN`）与报一个**树里没有的名字**（答 `UNKNOWN`）——三格的答码
# 都由 `line::call` 那张表给出（`OK` / `UNKNOWN` / `TAKEN` = 0 / 1 / 2）。拿 `uart` 那条线试会
# 与它的登记抢时间，故 `TAKEN` 这一趟拿房客自己刚占下的线试：读数因此是确定的。
#
# `router: lane dropped line=11` 是**被拒那一趟的收尾**：`TAKEN` 这一趟已经 `seat`+`claim` 过
# 一条泊位（本端铸的那枚 + 从客户手里认下的那枚），而 `Lines::occupy` 收不下它——本域当场把
# 那两枚放下，不留在账外（否则每失败一次多两枚，直到本域退场）。这条读数与 `lodger: taken=2`
# 是同一次登记的两头：一头是客户收到的答码，一头是本域把孔放回去。
#
# `irq: ring=<n> busy=<m> idle_ring=<i> idle_busy=<j>` 是**铃那一刀的读数**（收尾摘要里印，
# 与 `timer:` / `doom:` / `sched:` 同族）：`ring` = 内核摇铃几次、`busy` = 其中几次铃还响着
# （trap 据此关本 hart 闸门）；`idle_*` = 其中**空闲核补摇**的那一支。那一支是"没人可调"
# 窗口的补丁——`idle_ring` 非零说明这一手真的在走，为零也是读数（那一段没发生）；实测它
# 常在 1 上下、偶发拉高（一次观测到 `idle_ring=173 idle_busy=171`，即**有界自旋**的长度，
# 消费者认领 PLIC 后收住）。
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
        && grep -q "router: line 10 = serial@10000000" "$log" \
        && grep -q "uart: rang n=" "$log" \
        && grep -q "router: line 11 = rtc@101000" "$log" \
        && grep -q "router: vacate line=11" "$log" \
        && grep -q "lodger: taken=2" "$log" \
        && grep -q "lodger: unknown=1" "$log" \
        && grep -q "router: lane dropped line=11" "$log" \
        && grep -q "uart: tree part=2 land=0 find=0 got=true" "$log" \
        && grep -q "echo: console=true" "$log" \
        && grep -q "irq: ring=" "$log" \
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
