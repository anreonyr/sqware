#!/usr/bin/env nu

# sqware examine 验收门（原名 e2e）——nu 版，取代 scripts/examine.sh。
#
# 判据四条，缺一不可（与 .sh 版逐条相同）：
#   1) 逐步生效：十二条命令（默认档；harden/框架档十五条）**逐个等它的输出出现**再发下一条
#      （expect 式），不是按墙钟盲排；
#      每步都有独立超时，失败能指到具体哪一条。
#   2) 自行退出：qemu 自己结束（停机走 srst）⇒ 外接 timeout 的退出码不是 124；
#      捕获里也不该出现 `terminating on signal …`。
#   3) 无崩溃：捕获里无 `[panic] at`（内核 panic 报告头）。
#   4) marker 齐全（默认档十五条；harden/框架档在其上再加非默认档那四条），含
#      `task: all tasks exited, system halted` 与
#      `badslot: 1/1 abnormal exit reaped, kernel alive`，以及中断链那条
#      `plic: line 10 delivered`。
# 默认连跑 3 次要求 3/3；任一判据不过 ⇒ 该轮 FAIL，进程以非零退出。
#
# ── 三条补上的断言（非默认档跑）──────────────────────────────────────────────
# `redeem` / `wipe` / `prune` 三条内核路径**本来就已被现有探针走到**，缺的是断言：
#   1) `redeem`：`sleep <ms>` 走到「票 → 持票人 → 键 → 站点」，`woke` 即断言——但
#      原来只跑一个时长、一轮一次。非默认档在 `sleep 300` 之后**追加** `sleep 700`：
#      票单调不复用 ⇒ 若这条路上有残留，第二次就会挂住。这是**结构性**断言。
#   2) `wipe`：`hole` 的自测本就含 seal（走到了 wipe 的调用方）却零断言。userland
#      侧在同一轮里以 seal 为界各等一次并打印**结局**（`wake=seal` / `wake=timeout`），
#      门断言 `hole: wait-seal sealed=1 wake=seal`——seal 不唤醒等待者就只会打出 timeout。
#   3) `prune`（空站点出队即删）**目前没有专属判据**：它当年靠审计档关机钩子里的
#      站点表计数（`[audit] sites N live N tomb N orphan N waiters N`）看着，那条钩子
#      随审计层一起删了。间接覆盖仍在——`wipe` / `hole` / `cascade` 三条都要求站点
#      在被 seal/退役时当场删掉，删不掉就会在后续等待里现形。要恢复直接判据，就在
#      `health/` 里加一条读站点表的用例，而不是把关机钩子搬回来。
#
# 用法：scripts/examine.nu                        # 默认连跑 3 次，要求 3/3
#       EXAMINE_REPEAT=10 scripts/examine.nu
#       EXAMINE_HARDEN=1 scripts/examine.nu       # 默认 3 轮 + 1 轮 harden 档
#       EXAMINE_HARDEN=1 scripts/examine.nu       # ↑ **全门**：默认 3 轮 + harden 轮 + 框架轮
#                                                 #   （框架轮默认就开，见 EXAMINE_FRAMEWORK）
#       EXAMINE_FRAMEWORK=0 scripts/examine.nu    # 只跑默认轮（跳过用例/自检那一档）
#
# ── 按档构建、按档跑（修掉的设计瑕疵）────────────────────────────────────────
# 原来**只构建一次** ELF，默认轮与另一档共用。于是想让某一档跑到自己那份产物，就会撞上
# 另一档按构造必挂的期望 —— 两档不能在一次运行里各得其所。现改为**按档构建、按档跑**：
#   默认轮 ← `--profile release`（不带 feature）
#   harden 轮 ← `--profile harden`（= release + debug-assertions，不带 feature）
#   框架轮 ← `--profile framework --features framework`
# 三者的 profile / features / 步骤集 / marker 集 / 附加检查**只有一处事实**：`const FLAVORS`。
# cargo 的落点 `target/…/release/sqware` 每档各一条、换 feature 就覆盖 ⇒ 每建完一档**立刻**把
# 产物（连同同目录的 `initrd.img`，boot.nu 按 ELF 同目录找它）搬进本档自己的目录
# `<OUT>/elf-default/`、`<OUT>/elf-harden/`、`<OUT>/elf-framework/`；每轮只跑自己那份
# ⇒ 三档不可能互相污染（调换构建顺序也一样，因为搬运发生在下一次构建之前）。
# 判据一条没动：逐条 marker、逐步 expect、自退非 124、无 panic 全部照旧。
#
# ── 三档都该绿（**没有**豁免机制）─────────────────────────────────────────────
# 默认 / harden / 框架三档跑的是同一套判据，三档**都该 PASS**。没有「已知失败不算失败」
# 的开关：没有白名单，没摘 marker，没放宽阈值，ok 恒为 `$why == ""`。
# 若某一轮报出内核自己点的名（锁序违规 `[depend]`、挂起自检 `[weak]`、健康用例
# `[health]`、用例失败 `[case] FAIL`），那是一条**回归**，该轮当场 FAIL；
# `existing_violation_note` 只负责把这条 FAIL 的原因串写得更具体——它只在**已经判 FAIL**
# 的轮上追加话，判定既不增也不减。
#
# ── qemu 起法：唯一出处 scripts/boot.nu ────────────────────────────────────────
# 门**不凑 qemu 参数**：`^nu scripts/boot.nu <elf>`，QEMU_TIMEOUT / QEMU_SEED / QEMU_ICOUNT
# 等全由它解释（见该文件头注）。**icount 显式置空**——boot.nu 的默认档是 `auto,sleep=on`，
# 门必须主动关掉：带 icount 实测两批 14/18、关掉后两批 20/20。
# 要复现「同 seed 可跑同一条轨迹」时才设 EXAMINE_ICOUNT=auto,sleep=on。
#
# ── 输入：长驻写端（为什么是一门管道，而不是 .sh 的 FIFO）─────────────────────
# 两条实测教训：写端在下游还没持读端时会被 **SIGPIPE 杀掉**，日程后半段的命令
# 一条都写不出去；stdin 一旦 **EOF**，guest 把它当 **Ctrl-D** ⇒ shell 自退、自然停机
# （qemu 退码 0），于是「超时/存活」这类探针测到的是关机。
# .sh 的手法是自持 FIFO 写端（`exec 3>` 阻塞到 qemu 打开读端），收尾 `exec 3>&-`。
# nu 里这条路不可达：**nu 没有 `<` 重定向**（实测 `^cat < f` 把 `<` 当普通参数交给 cat，
# nu 0.115.1），FIFO 便没法接到 qemu 的 stdin 上；nu 里「文件 → 子进程 stdin」只剩
# `open $f | ^cmd`，而那是读到底才转发、且立刻 EOF —— 正是要避开的形态。
# 故改用管道，性质与 .sh 的 FIFO 相同（写端长驻、读端常在）：
#     ^tail -f -n +1 <命令文件> | ^nu scripts/boot.nu <elf>
#   - **tail 是长驻写端**：它活着，管道写端就不关 ⇒ 不会 EOF（不会被当 Ctrl-D）；
#   - **读端自进程启动起就在**（nu → timeout → qemu 持有）⇒ 不会 SIGPIPE；管道 64 KB
#     缓冲还兜住「门写早了、guest 还没读」的窗口；
#   - 门只用 `save --append` 往命令文件里逐条追加，tail 逐行吐给 guest。
# 最小实验（本轮实跑，现场在 trace/examine-exp/）：追加 `spawn\n` 后 guest 逐键回显
# `sq > spawn` 并打出 `spawnjoin -> 499500`；追加 `exit\n` 后打出
# `task: all tasks exited, system halted` 且 qemu 自退 rc=0；同时 `ps -o args=` 里 qemu
# 命令行**无 `-icount`**。同一次实验也记下 `^cat < f` 的失败形态。
#
# ── nu 语义地雷（改本脚本前先读）──────────────────────────────────────────────
#   - 外部命令非零退出 ⇒ **当场中止整个脚本**（其后语句都不执行）。故取退出码只有两条路：
#     `… | complete`（读 .exit_code）或 `try { … } catch { }` 后再读 `$env.LAST_EXIT_CODE`。
#   - **没有 `<` 重定向**；`do -i` 会把退出码清成 0（退出码这个观测量就丢了）。
#   - `job spawn` 的闭包跑在另一线程，`job kill` 连带杀它的子进程；nu 0.115 **没有
#     `job wait`** ⇒ qemu 退出码由 job 自己落盘（`<run>/qemu.rc`），门轮询该文件判收尾。
#
# ── 证据（每轮独立目录，失败现场保留）────────────────────────────────────────
#   <OUT>/run<i>/console.log   qemu 控制台与 stderr 的合并捕获（`o+e>`）
#   <OUT>/run<i>/cmds.txt      门写出的命令（长驻写端读的就是它）
#   <OUT>/run<i>/qemu.rc       qemu 退出码（124 = 被外接 timeout 杀）
#   <OUT>/run<i>/diag.txt      仅失败时写：why + 本轮跑的 ELF / features + 字节数 / 回显数 / qemu 现场
#   <OUT>/elf-default/sqware   默认档构建的产物（含同目录 initrd.img）——默认轮跑的就是它
#   <OUT>/elf-harden/sqware    harden 档构建的产物（`--profile harden`）——harden 轮跑的就是它
#   <OUT>/elf-framework/sqware 框架档构建的产物（`--features framework`）——框架轮跑的就是它
#
# ── 旋钮 ─────────────────────────────────────────────────────────────────────
#   EXAMINE_REPEAT     轮数（默认 3）
#   EXAMINE_OUT        输出根目录（默认 trace/e2e-<时间戳>）
#   QEMU_TIMEOUT       秒（默认 60；由 boot.nu 解释：外接 timeout）。**别按"日程要多久"估**：
#                      实测一整轮墙钟 ≈ 57 s（guest 自报时钟 34.7 s + 启动 ≈ 20 s + 每步 2 s 让出），
#                      而**本轮试过把它压到 10 s ⇒ 5/5 全挂、每轮只走到第 4 步**（rc=124）。
#                      这条是**兜底**不是"挂在半路"的检测：轮次的耗时预算与它无关（逐轮在
#                      QEMU_TIMEOUT+10 内落定），真正决定"挂住多久"的是单步 EXAMINE_STEP_WAIT。
#   EXAMINE_STEP_WAIT  单步等待上限秒（默认 15）
#   EXAMINE_T_GAP      每条命令之间的让出秒（默认 2，同 .sh）
#   EXAMINE_T_BOOT     .sh 的遗留旋钮：那边也从未被读用，这里同样只接受、不影响时序
#   EXAMINE_ICOUNT     非空则透传给 boot.nu（默认空 = **关 icount**）
#   EXAMINE_HARDEN     "1" ⇒ 再加一轮 harden 档（`--profile harden` = release +
#                      `debug-assertions`，**不带 feature**）：整条 debug 断言面（容器⇔状态
#                      不变量、lockdep 校验、帧/页表护栏）放回被测产物，而 panic 走**崩溃
#                      转储**（用例框架不在场 ⇒ 会话期的断言失败有完整现场）。
#                      判据 = 无 `[depend]` + 两对顺序断言 + stray/cascade 探针；构建后另做
#                      两道**正向对照**（ELF 里必须出现断言串与 lockdep 报文体，否则这一档
#                      等于白跑）。
#   EXAMINE_FRAMEWORK  "0" ⇒ **跳过**框架档（默认为开）。这一档跑内核内测试框架
#                      （`--profile framework --features framework`）：用例登记在链接期、
#                      跑在启动期，逐例打点 + 末行汇总；**自检**（挂起自检 / 帧取还范围 /
#                      簿记↔页表双向核对）也在这一档。判据是同一条路数加上「用例全过（条数
#                      具体到个位，零用例与全过只差一个数字）+ 15 步照跑」。
#                      默认开是因为它判的是**别的档判不到**的东西：分配器/页表/Space 的
#                      内部不变量，而那正是既往几轮真缺陷的所在。
#   （**没有** `EXAMINE_FEATURES` 这个旋钮了：profile / features / 步骤集 / marker 集 /
#    附加检查一律从 `const FLAVORS` 那一行读。想反向验证某一档的哨兵，就临时改那一行 ——
#    可配置的「让某轮跑别档 ELF」正是本门修掉的那个瑕疵。）

# 步骤：命令 → 该步要看到的输出（逐字照抄 .sh 版）。最后一条同时是自然停机的判据。
#
# 默认档 = .sh 版八条，逐字未动。**非默认档**（harden / 框架）在同一序列上**追加**三步
# （不改既有的命令、时序与 marker）：`sleep 700`、`hole`（hole 的期望串两种档相同，
# 只是它多打的那几句由非默认档的 marker 去断言）、`stray`（野 id 自检：从未入册的
# task id 去 Join 必须 -1 Denied——判活并成一条来源之后，这条才答得出来）与 `cascade`
# （派生级联自检：它覆盖 `doom::doom → cull → suspend/reap` 这条 kill 路径——此前
# 两档控制台里 `killed` 出现 0 次，等于零覆盖）。
const STEPS = [
  {cmd: "spawn",     pat: "spawnjoin -> 499500"}
  {cmd: "dir",       pat: "discover echo -> found"}
  {cmd: "req",       pat: 'req echo -> "ifmmp\.tfswjdf'}
  {cmd: "hole",      pat: 'hole got "hi from shell'}
  {cmd: "sleep 300", pat: "woke"}
  {cmd: "sleep 700", pat: "woke"}   # 见「非默认档步骤」：redeem 的第二个时长
  {cmd: "clock",     pat: "clock [0-9]"}
  {cmd: "badslot",   pat: "badslot: 3/3 rejected, kernel alive"}
  {cmd: "stray",     pat: "stray: 3/3 illegal-id joins denied"}
  {cmd: "cascade",   pat: "cascade: ok"}
  # 他杀（`docs/driver.md` §12 的 `kill`）：**经 root 的他杀服务**收掉 echo，
  # 随后一步是它的牙——名字在目录里**再没有活实例**（不是"root 说成了"）。
  {cmd: "kill echo", pat: "kill echo -> ok"}
  {cmd: "dir",       pat: "discover echo -> not found"}
  # 线的权威（`docs/driver.md` §12 甲）：**反证探针**。shell 谁的名下设备都不是
  # （root 只把 console 那台交出去了）⇒ 抢 console 的名字必须被拒；而设备树里没有的
  # 名字必须是"不认识"。两条合起来说明：线号是名字的函数、名字的属主只能由 root 写
  # ——报文里既没有线号，也没有任何能自证"这台设备是我的"的字段。
  {cmd: "line serial@10000000", pat: "line serial@10000000 -> not-yours"}
  {cmd: "line rtc@101000",      pat: "line rtc@101000 -> unclaimed"}
  {cmd: "exit",      pat: "task: all tasks exited, system halted"}
]

# ── 中断面（`docs/driver.md` §11 第二步）────────────────────────────────────
#
# 上面那张表的每一步都**先敲键盘后看输出**：`dir` 那个字符串是经 UART RX 进来的。
# 于是"输入能用"本身已经隐含了整条链——但它是**隐含**的：把中断登记摘掉，输入会退化成
# 有界轮询，上面那些步照样全过（实测过：闸门坏了那轮，门是绿的）。
#
# 故这里钉一条**只可能由中断产生**的读数：PLIC 驱动域在**第一次 claim → 投递**时打的
# 那一行（`prog-plic`，一次性标记）。它要成立，必须
#   设备拉线 → PLIC 置 pending → SEI → 内核推空令牌进 `irq` 门闩 → 驱动 claim 到线号
#   → 投进 console 的会话门闩 → console 的输入线程被唤醒
# 这一整条都在。轮询路径**不会**产生它：驱动只在 claim 到东西时说话。
#
# **线的权威落地之后这条 marker 的牙更硬了**（`docs/driver.md` §12 甲）：报文的线号字段
# 已经不存在，客户端只报名字 ⇒ "line 10" 这个数字只能是驱动**自己从设备树解出来**的。
# 名字没写属主（root 的 `Refer` 没到）、或树里解不出这条线，这条 marker 都不会出现。
const IRQ_MARKER = "plic: line 10 delivered"

# 档位 → 本档要跑的步骤（下标取自上面那张表，命令与顺序都只有一处出处）。
# **改上面那张表就要重算这里**：本轮往 `exit` 前插 `cascade` 时漏算，默认档的 9 从
# `exit` 指到了 `cascade` ⇒ 默认轮从不 exit、三轮都挂到超时（症状像内核挂，其实是门）。
# 插 `kill`/`dir`/`line` 那几步时同样要重算（本轮又走了一遍这张表）。
const STEPS_DEFAULT = [0, 1, 2, 3, 4, 6, 7, 10, 11, 12, 13, 14]
const STEPS_FULL    = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]

# 默认档的构建 features **恒为空串**：那是 `FLAVORS` 里 `default` 那一行（构造上的保证，
# 不是旋钮，见头注「按档构建」）。

# 全量 marker（含 sleep 探针的 `sleep 300ms`），跑完逐条核。默认档那批是 .sh 版原文
# 逐字未动；他杀那两条（`kill echo -> ok` / `discover echo -> not found`）是后加的，
# 一条管回执、一条管复验；非默认档另加四条（票单调不复用 + wipe 的封印唤醒 + 他杀
# 两条）。计数以实跑为准，报告行里的数字是人写的、要跟着改。
const MARKERS = [
  "spawnjoin -> 499500"
  "discover echo -> found"
  'req echo -> "ifmmp\.tfswjdf'
  'hole got "hi from shell'
  "sleep 300ms"
  "woke"
  "clock [0-9]"
  "badslot: 3/3 rejected, kernel alive"
  # 带**原因码**退场（`RoomCall::Reap { reason }`）必须与上面那条**同轮**成立：非法
  # 调用要「调用方活、内核活」，带原因的退场要「**调用方死**、内核活」。内核一度把
  # "域级退场"实现成 `panic!`（用户态一句话打死整机）；这条 marker 就是它的牙。
  "badslot: 1/1 abnormal exit reaped, kernel alive"
  # 他杀：回执 `ok` 的含义是"内核确认它**走完了死亡路径**"（服务手里那枚指向它的
  # 副本被摘掉），下一条则从**目录**那一侧再验一遍"这个实例没了"——两条都要成立。
  "kill echo -> ok"
  "discover echo -> not found"
  # 线的权威（§12 甲）的两条反证：抢别人的名字被拒、树里没有的名字不认识。
  # 前一条的牙在"驱动那条属主判据还在不在"：去掉它，shell 会打出 `ok` 并**真的**
  # 把 console 的线抢走（输入随后退化成有界轮询，而这门里输入照旧"能用"）。
  "line serial@10000000 -> not-yours"
  "line rtc@101000 -> unclaimed"
  "task: all tasks exited, system halted"
]

# ── 非默认档**多跑三步**的判词（默认档一条都不跑）──────────────────────────────
#
# 名字里没有 audit：这几条与任何 feature 都无关，只是"那三步的期望输出"。三步是
# `sleep 700`（redeem 的第二个时长）、`stray`（野 id 自检）、`cascade`（级联扑杀）。
#
# 1) `redeem`：票**单调不复用**（`Ticket::alloc` 只有 fetch_add），到点兑现走
#    「票 → 持票人 → 键 → 站点」四步。第二个时长不是把同一个检查做两遍：它是
#    **结构性**的——若这条路上有残留（票根没作废、任务停在 Blocked、站点队列里
#    那张票没摘掉），第一次的 `sleep 300` 可能照样过，而第二次就会挂住。两条
#    `sleep …ms` 与两次 `woke` 都逐条断言，且 `sleep 700` 必须在 `sleep 300`
#    之后出现（顺序也断言，防「同一句被数了两次」）。
const EXTRA_MARKERS = [
  "sleep 700ms"
  'hole: wait-seal sealed=1 wake=seal'
  "stray: 3/3 illegal-id joins denied"
  "cascade: ok"
]
# 顺序断言：后者必须出现在前者之后（grep 行号比较；任一缺 ⇒ 直接挂）。
# 这三步只有**非默认档**跑，故两对顺序在 harden 与框架两档都核（`FLAVORS` 的 `checks`）。
const STEP_ORDER = [
  ["sleep 300ms", "sleep 700ms"]
  ['hole got "hi from shell', "hole: wait-seal sealed=1 wake=seal"]
]

# ── harden 档（告警：**不是**第二道别的档）────────────────────────────────────
#
# 这一档跑的是 `--profile harden`（= release + `debug-assertions = true`，**不带 feature**）：
# 把整条 debug 断言面（容器⇔状态不变量、lockdep 校验、帧/页表护栏）放回被测产物里，
# 而 panic 走**崩溃转储**（用例框架不在场）。它断言三件事：
#   ① kill 路径的探针照旧（`stray` / `cascade`）；
#   ② 控制台里**没有** `[depend]`——锁序违规的报文体（`report` 拼出来的那一行）；
#   ③ 那两对顺序断言（与框架档同）。
# 内核里任何 `debug_assert` 失败都会走 panic ⇒ 已被通用判据「无 panic」抓住；`[depend]`
# 单列一条是为了在原因串里点明「这是锁序违规」，不是别的 panic。

# harden ELF 的**正向对照**：这一档必须真的带着断言，否则它就退化成「又跑了一遍默认档」
# 而没人发现。实测过的事实：同一句断言在 release ELF 里 0 次、harden ELF 里 1 次。
# 取容器断言那一句当探针——它在 `Scheduler::push` 里，是 `debug_assert!`（关了自动消失）。
const HARDEN_PROBE = "starved 容器只收 Starved 任务"
# 两道**正向对照**（都从 ELF 里 grep，不看运行输出）：一句容器⇔状态断言
# （`debug_assert!` 的产物），一句 lockdep 锁序报文体（`lock/depend.rs` 整块
# `#[cfg(debug_assertions)]` 的产物，由 `--profile harden` 打开 —— 与 cargo feature 无关）。
# 少任何一道，这一档就退化成「又跑了一遍别的档」而没人发现。
const HARDEN_PROBES = [
  $HARDEN_PROBE
  "lock-order level violation"
]

# 框架档的 marker：用例汇总行。
#
# 判据是**全过**（`0 fail` 且 `ok` 数 == 用例总数）：用例失败会走 panic 通道 → 被通用的
# 「无 panic」判据抓住，这一条是**正向**读数 —— 它同时挡掉"零用例"（`.tests` 段被链接器
# 丢掉时用例一个都不跑，而其余所有 marker 照旧齐）。零用例的症状比失败更坏：绿着，
# 什么都没测，故这条必须断言具体条数。
const FRAMEWORK_MARKER = "\\[case\\] cases 4 ok 4 fail 0"

# ── 档位表：**一档一行**，档位的事实只有这一处 ─────────────────────────────────
#
# 一行管五件事：构建参数（profile / features）、步骤集（steps）、要核的 marker
# （markers）、通用判据之外的附加检查（checks），报告行的数字则由前三项**算出来**
# （手抄的数字在改档时必然漂 —— 这张表就是为了让那类漂移不可能）。
# 加一档 = 加一行；改一档 = 改那一行。`run_once` / `main` 全从这里读。
#
# `markers` 三段的含义见 `markers_for`：`base` / `extra` / `case`。
# `checks` 的取值见 `run_once` 末：`depend`（`[depend]` 点名）、`order`（顺序断言）。
const FLAVORS = [
  {name: "default",   profile: "release",   features: "",          steps: "default", markers: "base",            checks: []}
  {name: "harden",    profile: "harden",    features: "",          steps: "full",    markers: "base+extra",      checks: ["depend" "order"]}
  {name: "framework", profile: "framework", features: "framework", steps: "full",    markers: "base+extra+case", checks: ["depend" "order"]}
]

# 按名字取那一行（档案的全部事实都从它读；名字写错就该当场炸，故 `first` 之后必有值）。
def flavor [name: string] { $FLAVORS | where name == $name | first }

# 本档要核的 marker：`base` = 默认档那十五条 + 中断链（**每一档都核**：中断面不是某一档的
# 附属品）；`extra` = 非默认档多跑三步的判词；`case` = 用例汇总行（判据是**全过**——
# `0 fail` 且 `ok` 数 == 用例总数：零用例的症状比失败更坏，绿着、什么都没测，故必须断言
# 具体条数）。
def markers_for [f: record] {
  mut out = ($MARKERS | append $IRQ_MARKER)
  if ($f.markers | str contains "extra") { $out = ($out | append $EXTRA_MARKERS) }
  if ($f.markers | str contains "case") { $out = ($out | append $FRAMEWORK_MARKER) }
  $out
}

# 本档要跑的步骤（命令与 marker 同源，见 STEPS / STEPS_DEFAULT / STEPS_FULL）。
# 非默认档跑**全部**步骤：断言的覆盖面越大，lockdep 与容器⇔状态断言能验到的路径越多
# —— 这一档要的就是「多跑一点、让校验抓到东西」。
def steps_for [f: record] {
  let idx = if $f.steps == "default" { $STEPS_DEFAULT } else { $STEPS_FULL }
  $idx | each { |i| $STEPS | get $i }
}

# 报告行：数字从表里算（步数 / marker 数不再手抄）。拼接而非 `$"…(…)…"`：插值里的括号
# 会被当成子表达式（nu 0.115 实测，本脚本踩过两次）。
def report_for [f: record, i: int] {
  let steps = ((steps_for $f) | length | into string)
  let marks = ((markers_for $f) | length | into string)
  'run ' + ($i | into string) + ': PASS (' + $f.name + ' 档：自退 + 无 panic + ' + $steps + ' 步全过 + ' + $marks + ' marker 齐)'
}

# 匹配一律走 `grep -E`（与 .sh 逐字同语义），且**字节安全**：控制台捕获里有 ANSI 转义、
# 也可能出现非法 UTF-8，nu 的 `open --raw` 遇非法 UTF-8 会报错——门不能因为内核打出
# 一段怪字节就自己崩掉。`complete` 是为绕开「外部非零退出当场中止」这条地雷。
def hit [pat: string, file: path] {
  if not ($file | path exists) { return false }
  ((^grep -Eq -- $pat $file | complete).exit_code) == 0
}

# 命中行号（1 基；无命中 ⇒ null）。顺序断言（STEP_ORDER）用——「两句话都在」
# 不等于「先后对」，同一句 marker 被数两次也能骗过 `hit`。
def at [pat: string, file: path] {
  if not ($file | path exists) { return null }
  let r = (^grep -nE -- $pat $file | complete)
  if $r.exit_code != 0 { return null }
  let first = ($r.stdout | lines | first)
  ($first | split row ':' | first | into int)
}


# ── 内核自报违约的如实标注（**不是**豁免）──────────────────────────────────
# 内核自己点名的违约会在捕获里留下一行**它自己的话**，随后走 panic 通道
# （`report(...)` ⇒ `[panic] at …`），于是「无崩溃」判据必然挂。**那一轮就该 FAIL**：
# 本函数**只**往原因串里补一句「这是哪一类违约」，不碰 ok/FAIL，不开开关、不设白名单、
# 不摘 marker、不放宽阈值。调用点另外带 `$why != ""` 前件 ⇒ 这句话只可能加在**已经判
# FAIL** 的轮上（见 run_once 末）。原文**从捕获里读**、不写死：哪一轮变了原因串跟着变。
# 只写指针、不去改 docs。
def existing_violation_note [file: path] {
  # 逐个试：命中哪条就报哪条（同一个因只报一次，顺序即"最像根因的排前面"）。
  let known = [
    ['\[depend\] ', "锁序违规（lockdep 的 report 体）"]
    ['\[weak\] 挂起自检', "跨挂起的弱引用（挂起自检）"]
    ['\[health\] ', "健康用例失败（health 的 expect!）"]
    ['\[case\] FAIL ', "测试框架用例失败（framework）"]
    ['frame freelist corrupt', "帧空闲链被写坏（pop_link 的降级分支）"]
  ]
  # 拼接而非 `$"…(…)…"`：插值里的括号会被当成子表达式（nu 0.115 实测踩过，
  # 这条正是踩点 —— 命中时整轮不是 FAIL 而是 nu 自己报 Command not found）。
  for k in $known {
    if (hit $k.0 $file) { return ("内核自报违约：" + $k.1) }
  }
  ""
}

# 等 marker 出现在捕获里；limit 秒内没等到 ⇒ false。每步独立超时（.sh 同名函数的语义，
# 连「哪一步、等的是哪条正则」那句即时提示也照旧打出来）。
def expect [pat: string, limit: int, file: path, step: string] {
  let t0 = (date now)
  loop {
    if (hit $pat $file) { return true }
    if ((date now) - $t0) >= ($limit | into duration --unit sec) {
      print $"  步骤[($step)] 超时（($limit)s 内没等到 /($pat)/）"
      return false
    }
    sleep 200ms
  }
}

# 等 job 收尾（qemu.rc 落盘 = 管道已结束），返回退出码；超限则杀掉 job 并返回 null。
def await_rc [rcfile: path, limit: int, job: int] {
  let t0 = (date now)
  while (not ($rcfile | path exists)) and (((date now) - $t0) < ($limit | into duration --unit sec)) {
    sleep 200ms
  }
  if ($rcfile | path exists) {
    (open --raw $rcfile | str trim | into int)
  } else {
    try { job kill $job } catch { }
    null
  }
}

# 原因串的拼接：.sh 用 ` + ` 串判据，这里保留同一风格。
# （.sh 的「只写出 N/8 条」那处漏了分隔符，直拼成 `步骤 hole只写出 5/8 条`；这里统一加 ` + `。）
def append_why [why: string, add: string] {
  if $why == "" { $add } else { $"($why) + ($add)" }
}

# 失败现场：留证据，不留一句「失败了」（.sh 的 poke 同义；换了机制，问的还是同一件事——
# 「字节有没有离开门」对「guest 还认不认这个输入端」）。
# **必须在收尾之前调**（.sh 的 poke 就在它的 `wait` 之前）：qemu 那时多半还活着，fd0/fd1
# 与命令行才是现场；等外接 timeout 杀完再量，只能量到一具尸体。
def snapshot [log: path, cmds: path] {
  let qpid = ((^ps -o pid= -C qemu-system-riscv64 | complete).stdout | str trim)
  let qargs = ((^ps -o args= -C qemu-system-riscv64 | complete).stdout | str trim)
  let alive = ($qpid != "")
  let gone = "无（快照时 qemu 已退出）"
  {
    qpid: (if $alive { $qpid } else { $gone })
    qargs: (if $alive { $qargs } else { $gone })
    fd0: (if $alive { (^readlink $"/proc/($qpid)/fd/0" | complete).stdout | str trim } else { $gone })
    fd1: (if $alive { (^readlink $"/proc/($qpid)/fd/1" | complete).stdout | str trim } else { $gone })
    cmds_bytes: ((^stat -c %s $cmds | complete).stdout | str trim)
    log_bytes: (if ($log | path exists) { (^stat -c %s $log | complete).stdout | str trim } else { "0（捕获还没建起来）" })
    # 回显计数：.sh 写的是 `sed 's/\r/\n/g' | grep -ac '^sq > '`——行首锚定，而 guest 的提示符
    # 前面恒有 ANSI 色码（`ESC[36msq > `）。实测 .sh 时代的轮次该计数**恒为 0**，恰恰在失败轮
    # 最需要它（trace/e2e-20260910-231112/run3 只回显 4 条，锚定计数仍报 0）。故这里去掉 `^`。
    echos: (if ($log | path exists) { (^sed -e 's/\r/\n/g' $log | ^grep -ac 'sq > ' | complete).stdout | str trim } else { "0" })
  }
}

def write_diag [file: path, ctx: record] {
  let rc_s = if $ctx.rc == null { "缺失（job 未在限内收尾，已杀掉）" } else { $ctx.rc | into string }
  let icount_s = if $ctx.icount == "" { "关（QEMU_ICOUNT 置空）" } else { $ctx.icount }
  let feats_s = if $ctx.features == "" { "（默认档：不带 feature）" } else { $ctx.features }
  let lines = [
    $"--- 诊断：($ctx.why) ---"
    $"seed         → ($ctx.seed)"
    $"icount       → ($icount_s)"
    $"ELF          → ($ctx.elf)（本档构建的产物；features=($feats_s)）"
    $"长驻写端     → tail -f -n +1 ($ctx.cmds)（喂 scripts/boot.nu 起的 qemu；nu 无 < ⇒ 管道代 FIFO）"
    $"已写出       → ($ctx.sent)/($ctx.total) 条命令，($ctx.cmds_bytes) 字节"
    $"capture      → ($ctx.log)（($ctx.log_bytes) 字节）"
    $"回显计数     → ($ctx.echos) 行含 'sq > '"
    $"qemu pid     → ($ctx.qpid)"
    $"qemu fd0     → ($ctx.fd0)"
    $"qemu fd1     → ($ctx.fd1)"
    $"qemu 命令行  → ($ctx.qargs)"
    $"qemu rc      → ($rc_s)"
  ]
  (($lines | str join (char nl)) + (char nl)) | save --force $file
  for l in $lines { print $"  ($l)" }
}

# 一轮：独立目录 + 起 qemu + 逐步 expect + 通用四条判据 + 本档 `checks` 里的附加检查。
def run_once [cfg: record, i: int, flavor: string] {
  let dir = ($cfg.out | path join $"run($i)")
  mkdir $dir
  let log = ($dir | path join "console.log")
  let cmds = ($dir | path join "cmds.txt")
  let rcfile = ($dir | path join "qemu.rc")
  let diagfile = ($dir | path join "diag.txt")
  let seed = (random int 0..1073741823)
  "" | save --force $cmds
  rm --force $log $rcfile $diagfile

  # 环境全归 boot.nu 解释；icount 默认置空（= 关），要复现轨迹才由 EXAMINE_ICOUNT 打开。
  $env.QEMU_ICOUNT = $cfg.icount
  $env.QEMU_TIMEOUT = ($cfg.qemu_timeout | into string)
  $env.QEMU_SEED = ($seed | into string)

  let boot = $cfg.boot
  let f = (flavor $flavor)
  # **本档只跑本档构建出来的 ELF**（产物在 <OUT>/elf-<档>/，见 main 的按档构建）：
  # 一次构建、两种期望，必然有一档被按构造误判 —— 故按档构建、按档跑。
  let elf = (match $flavor {
    "harden"    => $cfg.elf_harden
    "framework" => $cfg.elf_framework
    _           => $cfg.elf_default
  })
  let feats = $f.features
  # qemu：tail 长驻写端喂命令文件 → scripts/boot.nu（qemu 起法的唯一出处）。
  # 退出码拿不到（nu 无 job wait）⇒ job 自己落盘；**必须包 try**，否则被 timeout 杀（124）
  # 时 job 会当场中止，rc 文件永远不写（.sh 版旧 runner 的归档分支就是这么从未跑过的）。
  let job = (job spawn {
    try { ^tail -f -n +1 $cmds | ^nu $boot $elf o+e> $log } catch { }
    ($env.LAST_EXIT_CODE) | save --force $rcfile
  })

  mut why = ""
  mut sent = 0
  let steps = (steps_for $f)
  if not (expect "sq > " $cfg.step_wait $log "boot") { $why = "引导/提示符" }
  for s in $steps {
    if $why != "" { break }
    sleep ($cfg.t_gap | into duration --unit sec)
    # .sh 在这里写 FIFO fd3（可能 EPIPE / Bad fd）；这里写的是普通文件，故改用「这一行是否
    # 真的落进命令文件」当「输入确实离开门」的证据（写失败仍会计入 sent 缺口）。
    $"($s.cmd)\n" | save --append $cmds
    $sent += 1
    if not (expect $s.pat $cfg.step_wait $log $s.cmd) { $why = $"步骤 ($s.cmd)" }
  }

  # 失败现场在**收尾之前**抓（.sh 的 poke 就在它 `wait` 之前）：那时 qemu 多半还活着，
  # fd0/fd1/命令行才是现场。诊断文字留到判据齐全后再落盘 ⇒ 「活着的现场」与「完整的 why」都在。
  let live = (snapshot $log $cmds)

  # 收尾：等 qemu 自己结束。外接 timeout 在 boot.nu 里兜底，故必然在 QEMU_TIMEOUT + 余量内
  # 落定。.sh 在这儿 `exec 3>&-` 关写端（EOF ⇒ guest 当 Ctrl-D 自退）；nu 的写端由 tail
  # 长驻、关不掉，也不该关（见头注）——代价是**失败轮要等满 timeout 才收尾**，判据不受影响
  # （why 是黏性的，不会因后来出现 halt marker 而翻案）。
  # QEMU_TIMEOUT 为 0/空时 boot.nu 不加外接 timeout（= 不限制），这时门也退回无限等（.sh 的
  # 裸 `wait` 同义），不拿「限内收尾」去杀一轮本来就没设限的跑。
  let wait_limit = if $cfg.qemu_timeout > 0 { $cfg.qemu_timeout + 10 } else { 86400 }
  let rc = (await_rc $rcfile $wait_limit $job)

  let total = ($steps | length)
  if $sent != $total { $why = (append_why $why $"只写出 ($sent)/($total) 条") }
  if $rc == 124 { $why = (append_why $why "被超时杀") }
  if (hit 'terminating on signal' $log) { $why = (append_why $why "被超时杀") }
  if (hit '\[panic\] at' $log) { $why = (append_why $why "内核panic") }
  for m in (markers_for $f) {
    if not (hit $m $log) { $why = (append_why $why $"缺[($m)]") }
  }
  # 附加检查（表里那一行的 `checks`）。两条都只**加话**、不加新阈值：
  #   `depend` —— 控制台里不许出现 `[depend]`：lockdep 的报文体（锁序违规：同层嵌套 /
  #     层级递减 / 同锁重入）。内核里任何 debug_assert 失败都会走 panic ⇒ 已被通用判据
  #     「无 panic」抓住；这条是为了在原因串里点名「这是锁序违规」而不是别的 panic。
  #   `order`  —— 两步的先后也是判据（「两句话都在」不等于「第二次真的发生在第一次之后」；
  #     同一句被数两次也能骗过 `hit`）。
  if "depend" in $f.checks and (hit '\[depend\]' $log) {
    $why = (append_why $why "lockdep 违规([depend])")
  }
  if "order" in $f.checks {
    for pair in $STEP_ORDER {
      let first = (at $pair.0 $log)
      let second = (at $pair.1 $log)
      if $first == null or $second == null or $second <= $first {
        $why = (append_why $why $"顺序[($pair.0) → ($pair.1)]")
      }
    }
  }
  # .sh 里 `wait` 一定有退出码；nu 这条路可能取不到（job 没收尾），那也是失败的理由。
  if $rc == null { $why = (append_why $why "qemu 未在限内收尾（退出码取不到）") }

  # ── 既存违规的如实标注（**最后一步，只加话**）────────────────────────────────
  # `ok` 就是 `$why == ""`，故这行**必须**带 `$why != ""` 这个前件：只有**已经**判 FAIL 的轮
  # 才加这句指针 ⇒ 标注不可能把一轮 PASS 翻成 FAIL，也不可能把 FAIL 翻成 PASS；判据既不增
  # 也不减（见 existing_violation_note：没有开关、没有白名单、没摘 marker、没放宽阈值）。
  let note = (existing_violation_note $log)
  if $note != "" and $why != "" { $why = (append_why $why $note) }

  if $why != "" {
    write_diag $diagfile ($live | merge {
      why: $why, seed: $seed, icount: $cfg.icount, cmds: $cmds, log: $log,
      sent: $sent, total: $total, rc: $rc, features: $feats, elf: $elf,
    })
  }
  {ok: ($why == ""), why: $why, dir: $dir}
}

# 构建一档内核，并**立刻**把产物搬出 cargo 的公共落点。
# 为什么必须搬：cargo 的落点是 `target/…/release/sqware` **一条路径**，换 feature 就覆盖；
# 两档若都直接跑那条路径，先建的那档跑起来时手里那份可能已是后建的那档（原设计瑕疵的另一半）。
# 顺带把 `initrd.img` 一起搬：boot.nu 在 **ELF 同目录**找它（`-initrd`），不搬就等于把
# initrd 弄丢——那会让 guest 起不到 shell，且症状与 feature 毫无关系，极难查。
def build_flavor [profile: string, features: string, src: path, dest: path] {
  # 非零退出在 nu 里会当场中止脚本，故显式接住并退 1（.sh 的 `|| exit 1` 同义）。
  # 裸 `^cargo` 在 try 里仍然把输出流到终端（包进 `let` 才会被吞掉）。
  # feature 是**空串也照传** `--features`（cargo 对空 feature 列表与不传等价），
  # 免得两处分叉（传/不传各一条命令行）。
  # `harden` 档用 `--profile harden`（= release + debug-assertions）；命名档与 `--release` 同义。
  let flag = (if $profile == "release" { ["--release"] } else { ["--profile", $profile] })
  try { ^cargo build ...$flag -p kernel --features $features } catch {
    print $"examine: cargo build 失败（profile='($profile)' features='($features)' rc=($env.LAST_EXIT_CODE)）"
    exit 1
  }
  if not ($src | path exists) { print $"缺 ($src)"; exit 1 }
  let dir = ($dest | path dirname)
  mkdir $dir
  try { ^cp -- $src $dest } catch {
    print $"examine: 搬产物失败（($src) → ($dest) rc=($env.LAST_EXIT_CODE)）"
    exit 1
  }
  let initrd = ($src | path dirname | path join "initrd.img")
  if ($initrd | path exists) {
    try { ^cp -- $initrd ($dir | path join "initrd.img") } catch {
      print $"examine: 搬 initrd 失败（($initrd) rc=($env.LAST_EXIT_CODE)）"
      exit 1
    }
  }
}

def main [] {
  let root = ($env.FILE_PWD | path dirname)
  cd $root

  let repeat = ($env.EXAMINE_REPEAT? | default "3" | into int)
  let out = ($env.EXAMINE_OUT? | default $"trace/e2e-(date now | format date '%Y%m%d-%H%M%S')" | path expand)
  let qemu_timeout = ($env.QEMU_TIMEOUT? | default "60" | into int)
  let step_wait = ($env.EXAMINE_STEP_WAIT? | default "15" | into int)
  let t_gap = ($env.EXAMINE_T_GAP? | default "2" | into int)
  let _t_boot = ($env.EXAMINE_T_BOOT? | default "3" | into int)   # 同 .sh：接受但不用
  let icount = ($env.EXAMINE_ICOUNT? | default "")               # 默认空 = 关
  # `EXAMINE_HARDEN=1` ⇒ 再加一轮 harden 档（`--profile harden` = release + debug-assertions，
  # **不带 feature**）：把整条 debug 断言面（容器⇔状态不变量、lockdep 校验、帧/页表护栏）
  # 放回被测产物里，而 panic 仍走**崩溃转储**（用例框架不在场 ⇒ 会话期的断言失败有完整现场）。
  # 判词：没有 `[depend]`（锁序违规）+ 两对顺序断言 + 两道 ELF 正向对照（见 `FLAVORS`）。
  let harden = (($env.EXAMINE_HARDEN? | default "0") == "1")
  # 框架档**默认开**（`EXAMINE_FRAMEWORK=0` 可关）：这一档跑内核内测试框架
  # （`--profile framework --features framework`）—— 用例（`kernel/src/framework/` +
  # `health/`）跑在启动期，逐例打点 + 末行汇总；**自检**（挂起自检 / 帧取还范围 /
  # 簿记↔页表双向核对）也在这一档。这一档判两件事：① 用例全过（汇总行
  # `cases N ok N fail 0`，N 具体到条数 ——零用例比失败更坏）；② 判据其余几条照旧
  # （测试档要在**同一趟**里接着跑完那 15 步 shell 序列，因为用例通过即放行启动）。
  let framework = (($env.EXAMINE_FRAMEWORK? | default "1") == "1")
  # cargo 的落点：**各档共用**（换 profile/feature 就覆盖）⇒ 每建一档必须立刻搬走产物。
  let built = ($root | path join "target/riscv64gc-unknown-none-elf/release/sqware")
  # 各档自己的产物：本轮（本 OUT）自己的目录，各带 initrd.img。轮次只跑自己那份。
  let elf_default = ($out | path join "elf-default" "sqware")
  let elf_harden = ($out | path join "elf-harden" "sqware")
  let elf_framework = ($out | path join "elf-framework" "sqware")
  let boot = ($root | path join "scripts" "boot.nu")

  print $"examine: repeat=($repeat) out=($out) qemu_timeout=($qemu_timeout)s step_wait=($step_wait)s"
  print ('examine: 档位 = ' + (($FLAVORS | each {|f| $f.name }) | str join ' + ') + '（默认轮 ' + ($repeat | into string) + ' 轮' + (if $harden { " + harden 轮" } else { "" }) + (if $framework { " + 框架轮" } else { "" }) + '）')
  mkdir $out

  # 构建：**按档各建一次**，建完立刻搬进本档自己的目录（cargo 落点各档共用，见头注/build_flavor）。
  # 只建**有轮次要跑**的档（REPEAT=0 又不带别的档 ⇒ 一条都不建，0/0 与 .sh 的 `seq 1 0` 同义）。
  # profile / features 一律从 `FLAVORS` 那一行读 —— 命令行不该有第二份事实。
  let f_default = (flavor "default")
  if $repeat > 0 {
    print ('examine: 构建默认档（--profile ' + $f_default.profile + '，features ' + (if $f_default.features == "" { "(无)" } else { $f_default.features }) + '）→ ' + ($elf_default | into string))
    build_flavor $f_default.profile $f_default.features $built $elf_default
  }
  if $harden {
    let f = (flavor "harden")
    # 命名档的 cargo 落点是 target/<triple>/harden/（不是 release/），故 src 单独给。
    let built_harden = ($root | path join "target/riscv64gc-unknown-none-elf/harden/sqware")
    print ('examine: 构建 harden 档（--profile ' + $f.profile + '，debug-assertions=on，features ' + (if $f.features == "" { "(无)" } else { $f.features }) + '）→ ' + ($elf_harden | into string))
    build_flavor $f.profile $f.features $built_harden $elf_harden
    # **正向对照**：这一档必须**同时**带着两半护栏——容器⇔状态断言与 lockdep 的
    # 报文体。少任何一道，这一档就退化成"又跑了一遍别的档"而没人发现。
    # 实测基线：同一句断言在 release ELF 里 0 次、debug ELF 里 1 次。
    for probe in $HARDEN_PROBES {
      let r = (^grep -ac -- $probe $elf_harden | complete)
      let found = ($r.stdout | str trim)
      if $r.exit_code != 0 or $found == null or ($found | into int) < 1 {
        print $"examine: harden ELF 里找不到『($probe)』⇒ 这一档缺护栏的一半（断言 / lockdep）"
        exit 1
      }
    }
    print $"  harden 正向对照：两串齐（容器⇔状态断言 / lockdep），共 ($HARDEN_PROBES | length) 项"
  }

  let cfg = {
    out: $out, elf_default: $elf_default, elf_harden: $elf_harden,
    elf_framework: $elf_framework, boot: $boot,
    qemu_timeout: $qemu_timeout, step_wait: $step_wait, t_gap: $t_gap,
    icount: $icount,
  }

  if $framework {
    let f = (flavor "framework")
    # 命名档的 cargo 落点是 target/<triple>/framework/（与 harden 同理）。
    let built_fw = ($root | path join "target/riscv64gc-unknown-none-elf/framework/sqware")
    # 逐字拼（不能用 `$"…"`）：`$elf_framework` 此刻在作用域内，但下面这条打印的
    # 兄弟行曾写成纯字符串、把变量名原样打了出来——那类错在报告里看得见，在此记一笔。
    print ('examine: 构建框架档（--profile ' + $f.profile + '，features ' + $f.features + '）→ ' + ($elf_framework | into string))
    build_flavor $f.profile $f.features $built_fw $elf_framework
    # 正向对照：这一档必须真带着用例登记段 —— 段被链接器丢掉时用例一个不跑，
    # 而 transcript 上「零用例」与「全过」只差一个数字，故在此先验 ELF 里那段在。
    let n = (^readelf -sW $elf_framework | ^grep -c __tests_start | complete)
    if ($n.stdout | str trim) == "0" {
      print "examine: 框架 ELF 里没有 __tests_start ⇒ 用例登记段被丢了（零用例档）"
      exit 1
    }
    print "  framework 正向对照：.tests 段边界符号在（用例真被登记）"
  }

  mut pass = 0
  mut total_rounds = 0
  # `1..0` 在 nu 里**不是空区间**（会倒着数出 1、0 两轮），`.sh` 的 `seq 1 0` 是空的；显式挡一下，
  # 让 REPEAT=0/负数 与 .sh 同义（0 轮 ⇒ `examine: 0/0`）。
  let rounds = if $repeat > 0 { (1..$repeat) } else { [] }
  for i in $rounds {
    let r = (run_once $cfg $i "default")
    $total_rounds += 1
    # 结果行一律走 `report_for`（数字从 `FLAVORS` 那一行算），且**不能**写成
    # `$"… (自退 + …)"`：插值里的 `(` 会被当成子表达式、把紧跟的汉字当命令调用
    # （nu 0.115 实测：`Command `自退` not found`，这类地雷改写本脚本时踩过）。
    if $r.ok {
      $pass += 1
      print (report_for (flavor "default") $i)
    } else {
      print ('run ' + ($i | into string) + ': FAIL — ' + $r.why + ' —— 现场留在 ' + ($r.dir | into string))
    }
  }

  # harden 轮（仅当 EXAMINE_HARDEN=1）：跑 release + debug-assertions 那份产物（**不带
  # feature**）—— 判「没有 lockdep 违规」+ 两对顺序断言 + 那两道 ELF 正向对照。
  # 轮次编号接在默认轮之后，证据目录因此不会互相覆盖。
  if $harden {
    let i = $repeat + 1
    let r = (run_once $cfg $i "harden")
    $total_rounds += 1
    if $r.ok {
      $pass += 1
      print (report_for (flavor "harden") $i)
    } else {
      print ('run ' + ($i | into string) + ': FAIL (harden 档) — ' + $r.why + ' —— 现场留在 ' + ($r.dir | into string))
    }
  }

  # 框架轮（默认跑；`EXAMINE_FRAMEWORK=0` 跳过）：跑用例 + 自检那份产物。轮次编号接在前面几档之后。
  if $framework {
    let i = $repeat + (if $harden { 1 } else { 0 }) + 1
    let r = (run_once $cfg $i "framework")
    $total_rounds += 1
    if $r.ok {
      $pass += 1
      print (report_for (flavor "framework") $i)
    } else {
      print ('run ' + ($i | into string) + ': FAIL (框架档) — ' + $r.why + ' —— 现场留在 ' + ($r.dir | into string))
    }
  }

  print $"examine: ($pass)/($total_rounds)"
  exit (if $pass == $total_rounds { 0 } else { 1 })
}
