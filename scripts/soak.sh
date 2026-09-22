#!/bin/sh
# 收尾口径验收门 —— 会话被控制台 `exit` 结束后，必须真的走到停机。
#
# 为什么单独一条门：`echo` 读到一行 `exit` 才退场，root 的 `wait_last` 才返回、才有
# `root: done` 与级联收尾。**不喂 stdin 的跑法永远进不了收尾**（服务常驻，QEMU 一直
# 闲等被 timeout 杀掉）——此前"5 次冷跑全过"验的只是启动读数，这一格从未被覆盖。
#
# 判据（两条一起）：
#   1) 日志里出现 `task: all tasks exited, system halted`；
#   2) 五十三条启动 / 装配读数仍在（`router: tree part=0 land=0 find=0 got=true` /
#      `router: device_count=95 ctx=1` / `uart: serial@10000000 ier=rx` / `guest: reg=0 find=0` /
#      `guest: trip ok` / `echo: ready` /
#      **`router: line 10 = serial@10000000`** / **`uart: rang n=`** /
#      **`uart: tree part=2 land=0 find=0 got=true`** / **`echo: console=true`** /
#      **`router: line 11 = rtc@101000`** / **`rtc: line occupied`** / **`rtc: armed at=`** /
#      **`router: line=11`** / **`rtc: rang n=1`** / **`router: exhaust line=11`** /
#      **`router: vacate line=1`** / **`lodger: taken=2`** / **`lodger: unknown=1`** /
#      **`router: lane dropped line=1 pies=21`** / **`lodger: pies=9`** /
#      **`rtc: tree part=2 land=0 find=0 got=true`** / **`rtc: asked now=`** /
#      **`sleeper: reg=0`** / **`sleeper: found`** / **`sleeper: now=`** /
#      **`sleeper: past=2`** / **`sleeper: armed=0`** / **`sleeper: taken=1`** /
#      **`sleeper: rang at=`** / **`sleeper: gone`** /
#      **`coalition: tree part=2 land=0 find=0 got=true`**（结盟那台落了门牌）/
#      **`member: found=0`** / **`member: found=1`**（号由服务发：零号是**一枚普通的盟**、
#      号单调稠密）/ **`member: amid(me,c0)=false`**（**立了不等于进了**）/
#      **`member: enter(c0)=ok`** / **`member: leave(c0)=ok`** /
#      **`member: amid(sub,c0)=true`**（**同一枚盟里有两位**——键 = 身份那条定理）/
#      **`member: amid(sub,c0)=false`** / **`member: amid(me,c0)=true`**（**出的是那一对，
#      不是那个人**）/ **`member: amid(me,out)=err:unknown`**（第三态：这枚盟没铸过）/
#      **`member: amid(out,me)=false`**（伪造的身份号**不是失败**）/
#      **`member: done`** /
#      **`echo: list root=0,3`** / **`echo: list names=sys,device`**（**一串**那一刀：
#      `list` 答号、`name` 按号答名——**名与号分开**；根没有号，故 0 是第一个真格子 `sys`）/
#      **`echo: list device=4,5,6`**（`/device` 那三个号：router / uart / rtc）/
#      **`echo: name miss=true`**（没铸过的号答 `Unknown`，不是"答了一格空名字"）/
#      **`echo: seq=0`** /
#      **`member: band(c0)=n1 more=false`** / **`member: band(c0,next)=n0 more=false`**
#      （**游标是阈值**：拿末一枚接着取 ⇒ 空窗，不是错，也没有"过期游标"）/
#      **`member: band(out)=err:unknown`** / **`member: bloc(me)=n2 more=false`**
#      （反向：这条身份在两枚盟里）。三格读数里的 `ids=` 只记在日志里，**不钉判据**：
#      身份号跟着启动次序走（同一份镜像实测 12 / 13 两个值），钉它等于把一条与取窗无关的数
#      钉进门里——这正是"判据跟着读数走"的另一半：**判据只钉那条轴上的数**）；
#   3) 收尾摘要那一行 **`irq: ring=…`** 仍在——外部中断那枚铃的读数：摇了几次 / 其中几次
#      "还响着" / 其中**空闲核补摇**了几支（见下）。
#
# 这两条是**驱动侧那两条**：控制器自报 95 条线、本域用的 context 是 1；串口那一台已经把
# 设备拿在手里、把"收到字节就拉线"打开（`serial@10000000` 那枚 `ONLY` 门闩换了主人）。
# 前者还是"设备树解码还对"的一条判据：解码一改错，这一格先红。
#
# **照实记**：那一格原先是 `router: ndev=95 ctx=1`——`riscv,ndev` 改名回 `device_count` 那一刀
# （设备树属性名还原）没有回头看这张判据，于是 `soak` 从那天起就一直是红的（读数换了个词，判据
# 没跟着走）。这一刀顺手改回来：**判据跟着读数走，不是反过来**。
#
# `router: tree part=0 land=0 find=0 got=true` 是**门牌**那一条：驱动自己把入口落到
# `/device/router` 上、再查回来验一遍（`part=0` = 那块目录是它建的；第二台上来会读成 2）。
# 而 `guest: reg=0 find=0` 里那个 `find` 是**从树上问到的**——按名找服务今天归树，板管生死。
#
# **照实记**：从前这里还有两条读数——`router: desk guest`（"招呼"那一趟）与 `answer=router`
# （路由者把自己的名字回给客人）。旧 32 字节"招呼"那一形状随用户裁定**整条退休**：门后只剩
# 登记一种形状（找人走树，见 `guest`）。"一问一答跨域"那格读数没丢——`guest: reg=` 与
# `find=` 是板上、树上各给的一格答码，线那一面由 `lodger: occupy=0`（路由者给的答码）顶着。
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
# 第四格 `vacate`（收线）**在这里是固定读数**了：`router: line 1 = virtio_mmio@10001000` 与
# `router: vacate line=1` 是**房客**（`prog-lodger`）那一对——它真领了一台**没人要**的设备的
# 门闩、占住 1 号线，然后**一句话不说就走**（不 `DISMISS`、不 `vacate`）。路由者每次醒来先探活
# （`alive` 答不出的那几条拆线 + 空出格子）⇒ 收线那一手第一次有了读数，而这一对只在
# "先占上、后没了"这条路上出现（喂不喂键都要有它：它跑在装配期）。
#
# **照实记**：房客原来占的是 11 号线（那时钟），第二台设备驱动上来之后那条线有主了
# （`rtc@101000`），故房客换成 1 号线——它要的是**一条没人要的线**。
#
# 第三十二条之后的十二条是**结盟那一刀**（`protocol::coalition` + `prog-coalition` +
# `prog-member`）：`coalition: tree part=2 land=0 find=0 got=true` 是**它自己的门牌**（`part=2`
# 是 `/sys` 已由身份服务建好）；`member` 那十一条把这一族要验的事各钉一格——**号由服务发**
# （`found=0` / `found=1`：零号是**一枚普通的盟**，盟无根）、**立了不等于进了**、**入**与**出**、
# **同一枚盟里有两位**（它派生第二条身份再领一次，故那不是"一串任务"而是"一组身份"）、
# **出的是那一对不是那个人**（出完 `amid(sub,c0)=false` 而 `amid(me,c0)` 照旧 true）、
# **第三态**（没铸过的号：`err:unknown`）、以及**别人的号只是标签**（伪造的身份号答 `false`，
# 不是失败）。
#
# **照实记：这一刀差点被 `echo: console=true` 判红，而它红得对。** 持树者那本客人账是**八格**
# （`Desk::CAP`，`programs/src/supervisor/operator/desk.rs`）。结盟送来**第五位常驻上树客人**
# （`coalition` 要按名字找 `/sys/principal`）与**又多一位会死的**（`member`）之后，八格在
# "5 常驻 + 3 位同时在场的临时客人"那一刻卡满：最后上树的 `echo` 进不了账，而它自己不知道
# ——它的问话孔没人管，第二次问话堵在单槽上，整台机器收不了场（第一次读数：`echo: console=false`
# 之后没有 `system halted`，`qemu-exit=124`）。故这一刀把那一格抬到 **12**（5 常驻 + 4 临时 +
# 三格余量），并把"满"这一格由**静默丢掉**改成**报一句**（`operator: desk full`）。
# **照实记**：那本账的老注早就写着"不剔，八格会被死客人占满，后来的连门都进不来"——它说对了，
# 只是它数的时候客人还没这么多。
#
# **照实记（本门一条既有的缺口）**：身份那一刀（`principal` / `prog-subject`）的读数从头到尾
# 不在上面这张判据表里——它加的时候没人回头看这张门。这一刀没有顺手补它（那要另算一次量），
# 只把缺口记在这里。
#
# **照实记（预算那一格）**：每一轮的外接期限从 **15 秒抬到 40 秒**。它是按"当年的机器"定的，
# 而**这个门跑的是 debug 档**（`cargo run` 不带 `--release`）：debug 的启动本来就慢一个量级，
# 结盟那一刀又加上两个程序（服务 + 探针，探针还要在启动期打二十几行读数）。量出来的数是
# **debug 启动到 `echo: ready` 要约 15 秒**（直接拿 debug ELF 跑，轮询到那一行 15157 ms），
# 而喂进去的 `exit` 落在启动期、要在启动完成之后才被读到 ⇒ 停机发生在 16 秒上下，正好压着旧
# 预算的线（读数：10 轮全是"无停机行"，日志停在 `member: done` 一带；同一个 ELF 手工放长到
# 60 秒就正常停）。40 秒 = 那个数的两倍半余量。**判据没变**——这一格管的是"feed 坏了就早点红"，
# 不是"必须 15 秒内停"。
# **照实记**：这一格也照旧把 `cargo run` 的**构建**算在期限里（冷跑时构建可能自己就超）。
# 今天跑门之前都先 build 过，故那一格没被量到；记在这里，别把它当成"启动慢"。
#
# 另外两条（`lodger: taken=2` / `lodger: unknown=1`）是**失败域**那两格：房客占下 1 号线之后
# 拿**同一条线**再来一次（答 `TAKEN`）与报一个**树里没有的名字**（答 `UNKNOWN`）——三格的答码
# 都由 `line::call` 那张表给出（`OK` / `UNKNOWN` / `TAKEN` = 0 / 1 / 2）。拿 `uart` 那条线试会
# 与它的登记抢时间，故 `TAKEN` 这一趟拿房客自己刚占下的线试：读数因此是确定的。
#
# `router: lane dropped line=1` 是**被拒那一趟的收尾**：`TAKEN` 这一趟已经 `seat`+`claim` 过
# 一条泊位（本端铸的那枚 + 从客户手里认下的那枚），而 `Lines::occupy` 收不下它——本域当场把
# 那两枚放下，不留在账外（否则每失败一次多两枚，直到本域退场）。这条读数与 `lodger: taken=2`
# 是同一次登记的两头：一头是客户收到的答码，一头是本域把孔放回去。
#
# 这一行还带一格 **`pies=`**（本域表里现在有几枚），那一格是**探针量过**的：把 `drop_lane` 里
# 那两枚的释放**临时关掉**，同一处读数从 `21` 变成 `23`（正好是"本端铸的那枚 + 从客户手里认下的
# 那枚"），改回来又是 `21`——故它**跟着孔走**，不是个常数。判据因此钉**整行**：漏放一枚，这一格
# 就是红的（这是"失败的登记不在账外留孔"这一刀的验法）。
#
# **照实记（这一格又从 24 回到 21）**：这一行原来钉 `pies=24`——那是 `rtc` 上来之后的值，而
# 24 里有**三枚是"答完话没放下"的回信孔副本**（`uart` 登记那一趟、`rtc` 登记那一趟、房客第一
# 趟），它们每登记一次涨一枚、直到本域退场。答完就放下之后，同一处读数是 **21**（24 − 3：
# 本趟那一枚还没到放下那一步，故只少 3 枚）。判据钉的是**整行**，故它跟着走。
#
# 房客那侧另有一格 **`lodger: pies=`**（它自己表里剩几枚），验的是**同一个纪律的另一半**：
# 失败那两趟把本端 `seat` 出去的那一枚（`Quay::shut`）与借出去的回信孔放下。探针量过——
# 把那几手**临时关掉**，同一处读数从 `9` 变成 `14`（3 枚回信孔 + 2 枚失败那两趟的泊位孔），
# 改回来又是 `9`。
#
# 六行是**第二台设备驱动**那一刀（`rtc`，`rtc@101000`，设备树里那条 11 号线）：
# `router: line 11 = rtc@101000` = **登记**（解树解出来的权威）；`rtc: line occupied` = 客户侧
# 那一头；`rtc: armed at=… ier=1 alarm=1` = **闸门开着、闹钟武装上了**（`ier` 读 `IRQ_ENABLED`、
# `alarm` 读 `ALARM_STATUS`——**它是 `alarm_running`，不是"到点了"**，那是被读数打回来之后
# 改的说法）；`router: line=11` = **那台设备真的把线拉起来了**（`uart` 那一格是 `router: line=10`）；
# `rtc: rang n=1 now=…` = 投递到了客户手里、它读走并**清掉**了那一格（`now >= at`）；
# `router: exhaust line=11` = 排空、线放回（喂不喂键都要有：那一次闹钟由客人约在 +50 ms）。
#
# 七行是**服务面那一刀**（`rtc` 兼报时服务，门牌 `/device/rtc`，客人 `prog-sleeper`）——这一刀
# 证的是"**抽象等第二个实例**"：`uart` 那一面只有一个方向（排空读到什么就交什么），这一面
# **两个方向都有**（客人问 + 设备叫）。`rtc: tree part=2 land=0 find=0 got=true` = **门牌**
# （`part=2` = 那块目录已经在了，`router` 先建的；与 `uart` 那一格同形）；`rtc: asked now=` 与
# `sleeper: now=` = **一问一答的两头**（同一趟的两个数**相等**——设备只有一个读者，读数在
# 驱动手里）；`sleeper: past=2` / `sleeper: taken=1` = **失败域那两格**（客人**有意**各走一趟：
# 过去的时刻、那一格已经有人——后者拿它自己刚约下的那一次试，故是确定的）；`sleeper: armed=0`
# = 真约上了；`sleeper: rang at=<at> now=<t>` = **设备叫的那一头**（到点设备自己拉线 ⇒ 路由者
# 投递 ⇒ 驱动清掉那一格、把"那一声"推回客人手里）。
#
# **照实记（因果变了）**：`rtc: armed at=` 那一行从前是驱动**起域时自己武装**的一次（10 Hz
# 自走），今天是**客人约的那一次**；自走那一圈连同 `PERIOD_NS` 整条撤掉了（有真客人之后它就是
# 没人要的机制——"没有读数的机制不落"的反面）。`rtc: rang n=1` 同理：还是那台设备自己拉线换来
# 的投递，只是那一次闹钟是客人定的。服务面那三份（帧形 / 那一格 / 客侧两手）住
# `programs/src/driver/rtc/`——**不进 `crates/protocol`**：旧 `uart` 协议的死因就是把它放进了
# 协议层。
#
# **照实记（那一刀量出来的三条设备语义）**：读时间要**先低后高**（低半格那次读把高半格锁存
# 起来）；`ALARM_STATUS` 是"武装着"不是"到点了"；写闹钟要**先高后低**（低半格那次写会当场
# 比较一次，首次写时高半格还是 0 ⇒ 当场判成到点，实测第一次武装早报约 99 ms）。另：那一格是
# **电平源**——把 `CLEAR_INTERRUPT` 那一手临时去掉，同一段运行里投递从 5 次变 3093 次。
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
  ( sleep 5; echo exit; sleep 3; echo exit ) | timeout 40 cargo run $prof > "$log" 2>&1
  if ! grep -q "task: all tasks exited, system halted" "$log"; then
    echo "round $i: FAIL 无停机行；$(grep -a '\[stop\]' "$log" | head -1)"
  elif ! { grep -q "router: tree part=0 land=0 find=0 got=true" "$log" \
        && grep -q "router: device_count=95 ctx=1" "$log" \
        && grep -q "uart: serial@10000000 ier=rx" "$log" \
        && grep -q "guest: reg=0 find=0" "$log" \
        && grep -q "guest: trip ok" "$log" \
        && grep -q "router: line 10 = serial@10000000" "$log" \
        && grep -q "uart: rang n=" "$log" \
        && grep -q "router: line 11 = rtc@101000" "$log" \
        && grep -q "rtc: line occupied" "$log" \
        && grep -q "rtc: armed at=" "$log" \
        && grep -q "rtc: tree part=2 land=0 find=0 got=true" "$log" \
        && grep -q "rtc: asked now=" "$log" \
        && grep -q "router: line=11" "$log" \
        && grep -q "rtc: rang n=1" "$log" \
        && grep -q "router: exhaust line=11" "$log" \
        && grep -q "sleeper: reg=0" "$log" \
        && grep -q "sleeper: found" "$log" \
        && grep -q "sleeper: now=" "$log" \
        && grep -q "sleeper: past=2" "$log" \
        && grep -q "sleeper: armed=0" "$log" \
        && grep -q "sleeper: taken=1" "$log" \
        && grep -q "sleeper: rang at=" "$log" \
        && grep -q "sleeper: gone" "$log" \
        && grep -q "router: vacate line=1" "$log" \
        && grep -q "lodger: taken=2" "$log" \
        && grep -q "lodger: unknown=1" "$log" \
        && grep -q "router: lane dropped line=1 pies=21" "$log" \
        && grep -q "lodger: pies=9" "$log" \
        && grep -q "uart: tree part=2 land=0 find=0 got=true" "$log" \
        && grep -q "echo: console=true" "$log" \
        && grep -q "coalition: tree part=2 land=0 find=0 got=true" "$log" \
        && grep -q "member: found=0" "$log" \
        && grep -q "member: found=1" "$log" \
        && grep -q "member: amid(me,c0)=false" "$log" \
        && grep -q "member: enter(c0)=ok" "$log" \
        && grep -q "member: leave(c0)=ok" "$log" \
        && grep -q "member: amid(sub,c0)=true" "$log" \
        && grep -q "member: amid(sub,c0)=false" "$log" \
        && grep -q "member: amid(me,c0)=true" "$log" \
        && grep -q "member: amid(me,out)=err:unknown" "$log" \
        && grep -q "member: amid(out,me)=false" "$log" \
        && grep -q "member: done" "$log" \
        && grep -q "echo: list root=0,3" "$log" \
        && grep -q "echo: list names=sys,device" "$log" \
        && grep -q "echo: list device=4,5,6" "$log" \
        && grep -q "echo: name miss=true" "$log" \
        && grep -q "echo: seq=0" "$log" \
        && grep -q "member: band(c0)=n1 more=false" "$log" \
        && grep -q "member: band(c0,next)=n0 more=false" "$log" \
        && grep -q "member: band(out)=err:unknown" "$log" \
        && grep -q "member: bloc(me)=n2 more=false" "$log" \
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
