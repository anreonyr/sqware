#!/usr/bin/env nu

# sqware examine 验收门（原名 e2e）——nu 版，取代 scripts/examine.sh。
#
# 判据四条，缺一不可（与 .sh 版逐条相同）：
#   1) 逐步生效：八条命令**逐个等它的输出出现**再发下一条（expect 式），不是按墙钟盲排；
#      每步都有独立超时，失败能指到具体哪一条。
#   2) 自行退出：qemu 自己结束（停机走 srst）⇒ 外接 timeout 的退出码不是 124；
#      捕获里也不该出现 `terminating on signal …`。
#   3) 无崩溃：捕获里无 `[panic] at`（内核 panic 报告头）。
#   4) 九个 marker 齐全（含 `task: all tasks exited, system halted`）。
# 默认连跑 3 次要求 3/3；任一判据不过 ⇒ 该轮 FAIL，进程以非零退出。
#
# ── 三条补上的断言（§9.3「三条缺失断言的处置」，只在 audit 档跑）─────────────
# `redeem` / `wipe` / `prune` 三条内核路径**本来就已被现有探针走到**，缺的是断言：
#   1) `redeem`：`sleep <ms>` 走到「票 → 持票人 → 键 → 站点」，`woke` 即断言——但
#      原来只跑一个时长、一轮一次。audit 档在 `sleep 300` 之后**追加** `sleep 700`：
#      票单调不复用 ⇒ 若这条路上有残留，第二次就会挂住。这是**结构性**断言。
#   2) `wipe`：`hole` 的自测本就含 seal（走到了 wipe 的调用方）却零断言。userland
#      侧在同一轮里以 seal 为界各等一次并打印**结局**（`wake=seal` / `wake=timeout`），
#      门断言 `hole: wait-seal sealed=1 wake=seal`——seal 不唤醒等待者就只会打出 timeout。
#   3) `prune`（空站点出队即删）每次收队都在跑、此前零观测量。内核在 **audit 档**的
#      关机点（排在 `scheduler::rip` **之前**的那条只读钩子）打印站点表计数
#      `[audit] sites N live N tomb N orphan N waiters N`；门断言全部任务退出后
#      **无孤儿站点、无活站点、无残留等待者**（「孤儿 == 0」才是对 `prune` 的直接
#      断言；总数与墓碑数是同一帧的记账，非判据——实测见 §9.3）。
#      **ABI 未动**：只加计数与打印，没有新 envcall、没有改任何对外调用号。
#
# 用法：scripts/examine.nu                    # 默认连跑 3 次，要求 3/3
#       EXAMINE_REPEAT=10 scripts/examine.nu
#       EXAMINE_FEATURES=audit scripts/examine.nu   # 默认 3 轮 + 1 轮 audit 档
#
# ── 按档构建、按档跑（本轮修掉的设计瑕疵）────────────────────────────────────
# 原来**只构建一次** ELF（`--features $EXAMINE_FEATURES` 那一份），默认轮与 audit 轮共用。
# 于是 `EXAMINE_FEATURES=audit` 时默认轮拿到的是 audit ELF，撞上默认档自己的哨兵「默认档
# 不该出现 audit 输出」（audit ELF 每次关机都打 `[audit] sites …`）⇒ 那几轮**按构造**必挂：
# 想验默认档就不能带 feature，想验 audit 档就同时挂掉默认轮，两档不能在一次运行里各得其所。
# 现改为**按档构建、按档跑**：
#   默认轮（`EXAMINE_REPEAT` 那几轮）← 不带 feature 构建的 ELF（`const DEFAULT_FEATURES`，恒空）
#   audit 轮                        ← 带 `EXAMINE_FEATURES` 构建的 ELF
# cargo 的落点 `target/…/release/sqware` 只有一条、换 feature 就覆盖 ⇒ 每建完一档**立刻**把
# 产物（连同同目录的 `initrd.img`，boot.nu 按 ELF 同目录找它）搬进本档自己的目录
# `<OUT>/elf-default/`、`<OUT>/elf-audit/`；每轮只跑自己那份 ⇒ 两档不可能互相污染
# （调换构建顺序也一样，因为搬运发生在下一次构建之前）。
# 判据一条没动：哨兵、九 marker、逐步 expect、自退非 124、无 panic 全部照旧（新步只追加）。
#
# ── 既存违规（**不是**豁免）───────────────────────────────────────────────────
# audit 档的关机审计有一条**已记账**的既存违规（A2 线，未修）：`[audit] task lifecycle leak
# at shutdown: 19 frames, 9 blocks` ⇒ `report()` ⇒ `[panic] at …`。故 audit 轮**就该判 FAIL**、
# 整体**就该非零退出**；门只在这条 FAIL 的原因串里如实写出「是什么」并指到
# `docs/audit-flying-wires.md` §9.3「关机审计第二条违规：已收缩到一个因（属 A2 线，未修）」。
# **没有**「已知失败不算失败」的开关：那条违规没从判据里摘掉，阈值没放宽，ok 仍为 false。
# 标注只加在**已经判 FAIL** 的轮上（调用点前置 `$why != ""`）⇒ 判定既不增也不减。
#
# ── qemu 起法：唯一出处 scripts/boot.nu ────────────────────────────────────────
# 门**不凑 qemu 参数**：`^nu scripts/boot.nu <elf>`，QEMU_TIMEOUT / QEMU_SEED / QEMU_ICOUNT
# 等全由它解释（见该文件头注）。**icount 显式置空**——boot.nu 的默认档是 `auto,sleep=on`，
# 门必须主动关掉：带 icount 实测两批 14/18、关掉后两批 20/20（docs/audit-flying-wires.md §9.3）。
# 要复现「同 seed 可跑同一条轨迹」时才设 EXAMINE_ICOUNT=auto,sleep=on。
#
# ── 输入：长驻写端（为什么是一门管道，而不是 .sh 的 FIFO）─────────────────────
# 两条实测教训（§9.3）：写端在下游还没持读端时会被 **SIGPIPE 杀掉**，日程后半段的命令
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
# ── nu 语义地雷（改本脚本前先读，§9.3 记过）──────────────────────────────────
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
#   <OUT>/elf-audit/sqware     audit 档构建的产物（仅带 feature 时有）——audit 轮跑的就是它
#
# ── 旋钮 ─────────────────────────────────────────────────────────────────────
#   EXAMINE_REPEAT     轮数（默认 3）
#   EXAMINE_OUT        输出根目录（默认 trace/e2e-<时间戳>）
#   QEMU_TIMEOUT       秒（默认 60；由 boot.nu 解释：外接 timeout）
#   EXAMINE_STEP_WAIT  单步等待上限秒（默认 15）
#   EXAMINE_T_GAP      每条命令之间的让出秒（默认 2，同 .sh）
#   EXAMINE_T_BOOT     .sh 的遗留旋钮：那边也从未被读用，这里同样只接受、不影响时序
#   EXAMINE_ICOUNT     非空则透传给 boot.nu（默认空 = **关 icount**）
#   EXAMINE_HARDEN     "1" ⇒ 再加一轮 harden 档（`--profile harden` = release +
#                      `debug-assertions`）：容器⇔状态断言与整条 lockdep 放回被测产物。
#                      判据 = 无 `[depend]` + stray/cascade 两条探针；构建后另做一次
#                      **正向对照**（ELF 里必须出现某条断言串，否则这一档等于白跑）。
#   EXAMINE_FEATURES   内核 cargo feature（默认空 = 默认档，行为/输出与原版逐字相同）。
#                      含 `audit` ⇒ 默认轮之后**再加一轮 audit 档**（追加 `sleep 700`
#                      与 hole 的封印唤醒观测，并断言关机时刻的站点表计数）。
#                      **只作用于 audit 轮**：默认轮恒跑不带 feature 构建出的 ELF。
#   （默认档的构建 features **不是旋钮**：见 `const DEFAULT_FEATURES`，恒为空。加个
#    `EXAMINE_DEFAULT_FEATURES` 等于门里开一条「让默认轮跑别档 ELF」的合法通路，
#    只会把本轮修掉的瑕疵做成可配置项——反向验证要的是临时改这一行常量。）

# 步骤：命令 → 该步要看到的输出（逐字照抄 .sh 版）。最后一条同时是自然停机的判据。
#
# 默认档 = .sh 版八条，逐字未动。**audit 档**在同一序列上**追加**三步（不改既有
# 八步的命令、时序与 marker）：`sleep 700`、`hole`（hole 的期望串两种档相同，
# 只是它多打的那几句由 audit 档的 marker 去断言）、`stray`（野 id 自检：从未入册的
# task id 去 Join 必须 -1 Denied——判活并成一条来源之后，这条才答得出来）与 `cascade`
# （派生级联自检：它覆盖 `doom::doom → cull → suspend/reap` 这条 kill 路径——此前
# 两档控制台里 `killed` 出现 0 次，等于零覆盖）。
const STEPS = [
  {cmd: "spawn",     pat: "spawnjoin -> 499500"}
  {cmd: "dir",       pat: "discover echo -> found"}
  {cmd: "req",       pat: 'req echo -> "ifmmp\.tfswjdf'}
  {cmd: "hole",      pat: 'hole got "hi from shell'}
  {cmd: "sleep 300", pat: "woke"}
  {cmd: "sleep 700", pat: "woke"}   # 见「audit 档步骤」：redeem 的第二个时长
  {cmd: "clock",     pat: "clock [0-9]"}
  {cmd: "badslot",   pat: "badslot: 3/3 rejected, kernel alive"}
  {cmd: "stray",     pat: "stray: 3/3 illegal-id joins denied"}
  {cmd: "cascade",   pat: "cascade: ok"}
  {cmd: "exit",      pat: "task: all tasks exited, system halted"}
]

# 档位 → 本档要跑的步骤（下标取自上面那张表，命令与顺序都只有一处出处）。
# **改上面那张表就要重算这里**：本轮往 `exit` 前插 `cascade` 时漏算，默认档的 9 从
# `exit` 指到了 `cascade` ⇒ 默认轮从不 exit、三轮都挂到超时（症状像内核挂，其实是门）。
const STEPS_DEFAULT = [0, 1, 2, 3, 4, 6, 7, 10]
const STEPS_AUDIT   = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

# 默认档的构建 features：**恒为空串**，是构造上的保证，不是旋钮（见头注「按档构建」）。
# 想反向验证默认档哨兵（「默认档不该出现 audit 输出」）还拦不拦得住，就临时把它改成
# `"audit"`：默认轮便会跑在 audit ELF 上，哨兵必响 —— 验完改回。
const DEFAULT_FEATURES = ""

# 全量 marker（含 sleep 探针的 `sleep 300ms`），跑完逐条核。默认八步那九条
# 同样是 .sh 版原文，逐字未动；audit 档另加三条（redeem 的第二个时长 + wipe 的
# 封印唤醒 + prune 的站点表计数）。
const MARKERS = [
  "spawnjoin -> 499500"
  "discover echo -> found"
  'req echo -> "ifmmp\.tfswjdf'
  'hole got "hi from shell'
  "sleep 300ms"
  "woke"
  "clock [0-9]"
  "badslot: 3/3 rejected, kernel alive"
  "task: all tasks exited, system halted"
]

# ── audit 档追加的断言（默认档一条都不跑）──────────────────────────────────
#
# 1) `redeem`：票**单调不复用**（`Ticket::alloc` 只有 fetch_add），到点兑现走
#    「票 → 持票人 → 键 → 站点」四步。第二个时长不是把同一个检查做两遍：它是
#    **结构性**的——若这条路上有残留（票根没作废、任务停在 Blocked、站点队列里
#    那张票没摘掉），第一次的 `sleep 300` 可能照样过，而第二次就会挂住。两条
#    `sleep …ms` 与两次 `woke` 都逐条断言，且 `sleep 700` 必须在 `sleep 300`
#    之后出现（顺序也断言，防「同一句被数了两次」）。
const AUDIT_MARKERS = [
  "sleep 700ms"
  'hole: wait-seal sealed=1 wake=seal'
  "stray: 3/3 illegal-id joins denied"
  "cascade: ok"
  "\\[audit\\] sites "
]
# 顺序断言：后者必须出现在前者之后（grep 行号比较；任一缺 ⇒ 直接挂）。
const AUDIT_ORDER = [
  ["sleep 300ms", "sleep 700ms"]
  ['hole got "hi from shell', "hole: wait-seal sealed=1 wake=seal"]
]

# ── harden 档（告警：**不是**第二道 audit）───────────────────────────────────
#
# 这一档跑的是 `--profile harden`（= release + `debug-assertions = true`）：把
# **容器⇔状态断言**与**整条 lockdep（L1/L3 锁序）**放回被测产物里。它断言两件事：
#   ① 那条 kill 路径的探针照旧（`stray` / `cascade`）；
#   ② 控制台里**没有** `[depend]`——锁序违规的报文体（`report` 拼出来的那一行）。
# 内核里任何 `debug_assert` 失败都会走 panic ⇒ 已被通用判据「无 panic」抓住；`[depend]`
# 单列一条是为了在原因串里点明「这是锁序违规」，不是别的 panic。
const HARDEN_MARKERS = [
  "stray: 3/3 illegal-id joins denied"
  "cascade: ok"
]

# harden ELF 的**正向对照**：这一档必须真的带着断言，否则它就退化成「又跑了一遍默认档」
# 而没人发现。实测过的事实（docs §10.1）：release ELF 里这两句各 0 次、debug ELF 里各 1 次。
# 取容器断言那一句当探针——它在 `Scheduler::push` 里，任何构建都编得进去（非 cfg 代码）。
const HARDEN_PROBE = "starved 容器只收 Starved 任务"

# 本档要核的 marker：默认档九条（.sh 原文），audit 档再追加三条。
def markers_for [flavor: string] {
  match $flavor {
    "audit"  => ($MARKERS | append $AUDIT_MARKERS)
    "harden" => ($MARKERS | append $HARDEN_MARKERS)
    _        => $MARKERS
  }
}

# 本档要跑的步骤（命令与 marker 同源，见 STEPS / STEPS_AUDIT）。
def steps_for [flavor: string] {
  # harden 档跑**全部**步骤（含 audit 档那两步的探针）：断言的覆盖面越大，lockdep 与
  # 容器⇔状态断言能验到的路径越多——这一档要的就是「多跑一点、让校验抓到东西」。
  let idx = if $flavor == "default" { $STEPS_DEFAULT } else { $STEPS_AUDIT }
  $idx | each { |i| $STEPS | get $i }
}

# 匹配一律走 `grep -E`（与 .sh 逐字同语义），且**字节安全**：控制台捕获里有 ANSI 转义、
# 也可能出现非法 UTF-8，nu 的 `open --raw` 遇非法 UTF-8 会报错——门不能因为内核打出
# 一段怪字节就自己崩掉。`complete` 是为绕开「外部非零退出当场中止」这条地雷。
def hit [pat: string, file: path] {
  if not ($file | path exists) { return false }
  ((^grep -Eq -- $pat $file | complete).exit_code) == 0
}

# 命中行号（1 基；无命中 ⇒ null）。顺序断言（AUDIT_ORDER）用——「两句话都在」
# 不等于「先后对」，同一句 marker 被数两次也能骗过 `hit`。
def at [pat: string, file: path] {
  if not ($file | path exists) { return null }
  let r = (^grep -nE -- $pat $file | complete)
  if $r.exit_code != 0 { return null }
  let first = ($r.stdout | lines | first)
  ($first | split row ':' | first | into int)
}

# 从 audit 档那行计数里取一个数：`[audit] sites N live N tomb N orphan N waiters N`。
# 取不到 ⇒ null（由调用方当失败处理：**没量到**与「量到 0」必须分开）。
def audit_count [field: string, file: path] {
  if not ($file | path exists) { return null }
  let r = (^grep -oE -- '\[audit\] sites [0-9]+ live [0-9]+ tomb [0-9]+ orphan [0-9]+ dead [0-9]+ waiters [0-9]+' $file | complete)
  if $r.exit_code != 0 { return null }
  let line = ($r.stdout | lines | first)
  if $line == null { return null }
  let toks = ($line | split row -r '\s+')
  # 手写查找：nu 0.115 在这台机上没有 list 的 `index-of`（只有 bytes-/str- 前缀那两个）。
  # 找不到 ⇒ -1 ⇒ null（**没量到**与「量到 0」必须分开，故不返 0）。
  mut i = -1
  for k in 0..(($toks | length) - 1) {
    if ($toks | get $k) == $field { $i = $k; break }
  }
  if $i < 0 { return null }
  ($toks | get ($i + 1) | into int)
}

# ── 既存违规的如实标注（**不是**豁免）────────────────────────────────────────
# audit 档的关机审计有一条**已记账**的既存违规（A2 线）：`[audit] task lifecycle leak at
# shutdown: N frames, M blocks` ⇒ `report(IntegrityViolation::AuditDivergence)` ⇒ `[panic] at …`，
# 于是「无崩溃」判据必然挂。**这一轮就该 FAIL**：本函数**只**往原因串里补一句
# 「是什么、记在 §9.3 哪一节」，不碰 ok/FAIL，不开开关、不设白名单、不摘 marker、不放宽阈值。
# 调用点另外带 `$why != ""` 前件 ⇒ 这句话只可能加在**已经判 FAIL** 的轮上（见 run_once 末）。
# 数（N frames, M blocks）**从捕获里读**，不写死：哪一轮数变了，原因串跟着变（写死就成了造数）。
# 只写指针、不去改 docs。
def existing_violation_note [file: path] {
  if not (hit 'task lifecycle leak at shutdown' $file) { return "" }
  let r = (^grep -oE -- '\[audit\] task lifecycle leak at shutdown: [0-9]+ frames, [0-9]+ blocks' $file | complete)
  let found = (if $r.exit_code == 0 { ($r.stdout | lines | first) } else { null })
  let what = (if $found == null { "task lifecycle leak at shutdown（原文行取不到，见捕获）" } else { $found | str replace "[audit] " "" })
  # 同一因的第二笔账（docs §9.3 同节记着它与上面同源）：在就一并写出来，不在就不提。
  let table = (if (hit 'table frames [0-9]+ != kernel-walk count [0-9]+' $file) { " + table frames != kernel-walk count（同源）" } else { "" })
  $"既存违规[($what)($table)]：A2 线未修，记账 docs/audit-flying-wires.md §9.3「关机审计第二条违规：已收缩到一个因（属 A2 线，未修）」"
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

# 一轮：独立目录 + 起 qemu + 逐步 expect + 四条判据（audit 档另加三条）。
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
  # **本档只跑本档构建出来的 ELF**：默认轮 ← 不带 feature 的那份，audit 轮 ← 带
  # EXAMINE_FEATURES 的那份（两份产物在 <OUT>/elf-<档>/，见 main 的按档构建）。
  # 这就是本门的设计要点：一次构建、两种期望，必然有一档被按构造误判。
  let elf = (match $flavor {
    "audit"  => $cfg.elf_audit
    "harden" => $cfg.elf_harden
    _        => $cfg.elf_default
  })
  let feats = (if $flavor == "audit" { $cfg.features } else { $DEFAULT_FEATURES })
  # qemu：tail 长驻写端喂命令文件 → scripts/boot.nu（qemu 起法的唯一出处）。
  # 退出码拿不到（nu 无 job wait）⇒ job 自己落盘；**必须包 try**，否则被 timeout 杀（124）
  # 时 job 会当场中止，rc 文件永远不写（.sh 版旧 runner 的归档分支就是这么从未跑过的）。
  let job = (job spawn {
    try { ^tail -f -n +1 $cmds | ^nu $boot $elf o+e> $log } catch { }
    ($env.LAST_EXIT_CODE) | save --force $rcfile
  })

  mut why = ""
  mut sent = 0
  let steps = (steps_for $flavor)
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
  for m in (markers_for $flavor) {
    if not (hit $m $log) { $why = (append_why $why $"缺[($m)]") }
  }
  # harden 档单列一条：控制台里不许出现 `[depend]`——lockdep 的报文体（锁序违规：
  # 同层嵌套 / 层级递减 / 同锁重入）。内核里任何 debug_assert 失败都会走 panic ⇒ 已被
  # 通用判据「无 panic」抓住；这条是为了在原因串里点名「这是锁序违规」而不是别的 panic。
  if $flavor == "harden" and (hit '\[depend\]' $log) {
    $why = (append_why $why "lockdep 违规([depend])")
  }
  # 顺序断言：两步的先后也是判据（「两句话都在」不等于「第二次真的发生在第一次之后」）。
  if $flavor == "audit" {
    for pair in $AUDIT_ORDER {
      let first = (at $pair.0 $log)
      let second = (at $pair.1 $log)
      if $first == null or $second == null or $second <= $first {
        $why = (append_why $why $"顺序[($pair.0) → ($pair.1)]")
      }
    }
    # ── 站点表的断言：全部任务退出时刻的计数 ──
    # 内核侧的计数（`[audit] sites N live N tomb N orphan N dead N waiters N`）由**关机
    # 序列里排在 `scheduler::rip` 之前**的那条只读钩子打印——rip 会清空站点表，
    # 排在其后数出来的恒为 0，那样的断言没有牙。
    #
    # 判据：**live == 0、orphan == 0、dead == 0、waiters == 0**，逐条报数（不合并成一句
    # bool，失败时要能直接读数）。四项各指一件事：
    #   ① **live == 0 且 waiters == 0**：全部任务已退出 ⇒ 不该还有人挂在任何站点上。
    #   ② **orphan == 0**（队列空 ∧ 无信标）：`prune` 该删的残留——对 `prune` 的直接断言。
    #      有牙的实证：把 `prune` 关掉 ⇒ 孤儿 0 变 3；而总数只从 34 变 36（看总数的断言
    #      几乎没牙）。**轮④ 新挂 `cascade` 后它当场抓到一处真漏**：`take_beacon` 把站点
    #      清空却没 `prune` ⇒ 孤儿 2（修完回到 0，反去掉那行又回到 2）。
    #   ③ **dead == 0**（键的存活单元已死）：A2「站点寿命＝资源寿命」的**精确**形式——
    #      资源退役时 `wipe` 当场删站点，故一个死键站点存在 ⇔ 某条退役路径漏了 `wipe`。
    #   ④ `tomb`（队列空 ∧ **有**信标）**不作为判据**：那是「活键上留着一枚等未来认领的
    #      信号」，是 doorbell 语义的合法状态（`wake` 在无人在等时置的遗留信号，下一个
    #      等待者会立刻消费它）。轮④ 挂上 `cascade` 后它稳定是 1——若照老办法断言
    #      `tomb == 0`，门会在一个**合法**状态上判红。它照旧打印，供跨轮对比。
    #   ⑤ `sites` 总数同样不作判据（历史记账：它曾是 34，全是 `wipe` 墓碑；A2 之后
    #      由 ②③ 两项真正管住），只报数。
    let n_sites = (audit_count "sites" $log)
    let n_live = (audit_count "live" $log)
    let n_tomb = (audit_count "tomb" $log)
    let n_orphan = (audit_count "orphan" $log)
    let n_dead = (audit_count "dead" $log)
    let n_waiters = (audit_count "waiters" $log)
    if $n_sites == null or $n_live == null or $n_tomb == null or $n_orphan == null or $n_dead == null or $n_waiters == null {
      $why = (append_why $why "audit 站点计数取不到")
    } else {
      if $n_orphan != 0 { $why = (append_why $why $"孤儿站点[($n_orphan)]") }
      if $n_dead != 0 { $why = (append_why $why $"死键站点[($n_dead)]") }
      if $n_live != 0 { $why = (append_why $why $"活站点[($n_live)]") }
      if $n_waiters != 0 { $why = (append_why $why $"残留等待者[($n_waiters)]") }
      # 三形态必须配平（活 + 墓碑 + 孤儿 == 总数）：分列若与总数对不上，是**计数
      # 自身**坏了——那会让上面几条判据全部失效，故也当判据。
      if ($n_live + $n_tomb + $n_orphan) != $n_sites {
        $why = (append_why $why $"三形态不配平[($n_live)+($n_tomb)+($n_orphan)!=($n_sites)]")
      }
      print $"  audit 站点计数：sites=($n_sites) live=($n_live) tomb=($n_tomb) orphan=($n_orphan) dead=($n_dead) waiters=($n_waiters)"
    }
    # ── 名册：关机时**不该还有活着的任务** ──
    # 这一条比帧/块计数更早说出问题的名字：帧/块只说明"有东西没还"，它说明"哪个任务没走"。
    # 实现用 `Weak::strong_count()` 数活口（只读、不升强引用），故观测不改被观测的事实。
    let alive = ((^grep -oE -- '\[audit\] roster [0-9]+ alive [0-9]+' $log | complete).stdout | lines | first)
    if $alive == null {
      $why = (append_why $why "名册计数取不到")
    } else {
      let toks = ($alive | split row -r '\s+')
      # 取的是 grep -oE 的整段匹配 ⇒ toks = ["[audit]", "roster", N, "alive", M]
      let n_alive = ($toks | get 4 | into int)
      let n_roster = ($toks | get 2)
      print $"  名册：roster=($n_roster) alive=($n_alive)"
      if $n_alive != 0 { $why = (append_why $why $"名册活任务[($n_alive)]（就是它钉住了自己的 Team/Space ⇒ 帧/页留在类别账）") }
    }
  } else if (hit '\[audit\] sites ' $log) {
    # 默认档**不该**有 audit 输出：出现即说明跑的 ELF 带着 audit feature（不是本档构建）。
    $why = (append_why $why "默认档出现了 audit 输出")
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
  # 内核 cargo feature（默认空 = 默认档，行为与输出与本门原版逐字相同）。
  # `audit` ⇒ 默认档轮次**之后**再多跑一轮 audit 轮：步骤追加 `sleep 700` / hole 的
  # 封印唤醒观测，并断言关机时刻的站点表计数（prune 的观测量）。
  # **只作用于 audit 轮**：默认轮跑的 ELF 由 `const DEFAULT_FEATURES` 决定（恒空）。
  let features = ($env.EXAMINE_FEATURES? | default "" | str trim)
  let audit = (($features | split row -r '\s+') | any { |f| $f == "audit" })
  # `EXAMINE_HARDEN=1` ⇒ 再加一轮 harden 档（`--profile harden` = release + debug-assertions）：
  # 把容器⇔状态断言与整条 lockdep 放回被测产物里。**不是**第二道 audit：它不带 audit
  # feature，判的是「没有 `[depend]`（锁序违规）」+ 那两条探针。
  let harden = (($env.EXAMINE_HARDEN? | default "0") == "1")
  # cargo 的落点：**两档共用**（换 feature 就覆盖）⇒ 每建一档必须立刻搬走产物（见 build_flavor）。
  let built = ($root | path join "target/riscv64gc-unknown-none-elf/release/sqware")
  # 三档各自的产物：本轮（本 OUT）自己的目录，各带 initrd.img。轮次只跑自己那份。
  let elf_default = ($out | path join "elf-default" "sqware")
  let elf_audit = ($out | path join "elf-audit" "sqware")
  let elf_harden = ($out | path join "elf-harden" "sqware")
  let boot = ($root | path join "scripts" "boot.nu")

  print $"examine: repeat=($repeat) out=($out) qemu_timeout=($qemu_timeout)s step_wait=($step_wait)s"
  print ('examine: features=' + (if $features == "" { "(默认)" } else { $features }) + (if $audit { "（另加一轮 audit）" } else { "" }))
  mkdir $out

  # 构建：**按档各建一次**，建完立刻搬进本档自己的目录（cargo 落点两档共用，见头注/build_flavor）。
  # 只建**有轮次要跑**的档：
  #   REPEAT=0 且不带 audit ⇒ 一条都不建（0/0 与 .sh 的 `seq 1 0` 同义，也省一次编译）；
  #   带 audit 而 REPEAT=0   ⇒ 只建 audit 档（与改动前「只构建一次」的语义逐字一致）。
  if $repeat > 0 {
    print $"examine: 构建默认档（--features '($DEFAULT_FEATURES)'）→ ($elf_default)"
    build_flavor "release" $DEFAULT_FEATURES $built $elf_default
  }
  if $audit {
    print $"examine: 构建 audit 档（--features '($features)'）→ ($elf_audit)"
    build_flavor "release" $features $built $elf_audit
  }
  if $harden {
    # 命名档的 cargo 落点是 target/<triple>/harden/（不是 release/），故 src 单独给。
    let built_harden = ($root | path join "target/riscv64gc-unknown-none-elf/harden/sqware")
    print $"examine: 构建 harden 档（--profile harden，debug-assertions=on）→ ($elf_harden)"
    build_flavor "harden" $DEFAULT_FEATURES $built_harden $elf_harden
    # **正向对照**：这一档必须真的带着断言，否则它退化成「又跑了一遍默认档」而没人发现。
    # 实测基线（docs §10.1）：同一句断言在 release ELF 里 0 次、debug ELF 里 1 次。
    let r = (^grep -ac -- $HARDEN_PROBE $elf_harden | complete)
    let found = ($r.stdout | str trim)
    if $r.exit_code != 0 or $found == null or ($found | into int) < 1 {
      print $"examine: harden ELF 里找不到断言串『($HARDEN_PROBE)』⇒ 这一档没有 debug-assertions"
      exit 1
    }
    print $"  harden 正向对照：ELF 里『($HARDEN_PROBE)』出现 ($found) 次（release 档为 0 次）"
  }

  let cfg = {
    out: $out, elf_default: $elf_default, elf_audit: $elf_audit, elf_harden: $elf_harden, boot: $boot,
    qemu_timeout: $qemu_timeout, step_wait: $step_wait, t_gap: $t_gap,
    icount: $icount, features: $features,
  }

  mut pass = 0
  mut total_rounds = 0
  # `1..0` 在 nu 里**不是空区间**（会倒着数出 1、0 两轮），`.sh` 的 `seq 1 0` 是空的；显式挡一下，
  # 让 REPEAT=0/负数 与 .sh 同义（0 轮 ⇒ `examine: 0/0`）。
  let rounds = if $repeat > 0 { (1..$repeat) } else { [] }
  for i in $rounds {
    let r = (run_once $cfg $i "default")
    $total_rounds += 1
    # 这两行**不能**写成 `$"… (自退 + …)"`：插值里的 `(` 会被当成子表达式、把紧跟的汉字
    # 当命令调用（nu 0.115 实测：`Command `自退` not found`，正是 §9.3 记过的那颗地雷）。
    # 故结果行用拼接写，只有变量进插值。
    if $r.ok {
      $pass += 1
      print ('run ' + ($i | into string) + ': PASS (自退 + 无 panic + 8 步全过 + 9 marker 齐)')
    } else {
      print ('run ' + ($i | into string) + ': FAIL — ' + $r.why + ' —— 现场留在 ' + ($r.dir | into string))
    }
  }

  # audit 轮（仅当 EXAMINE_FEATURES 含 audit）：同一台门、同一套判据，另加三条
  # audit 断言（redeem 的第二个时长 / wipe 的封印唤醒 / prune 的站点表计数）。
  # 轮次编号接在默认轮之后，证据目录因此不会互相覆盖。
  if $audit {
    let i = $repeat + 1
    let r = (run_once $cfg $i "audit")
    $total_rounds += 1
    if $r.ok {
      $pass += 1
      # 标签照实写：audit 档判的是**孤儿 == 0 / 活 == 0 / 等待者 == 0**，不是「站点表已空」
      # （实测 sites=34 全是墓碑，docs §9.3「三条缺失断言落地」已记明总数不作判据）。
      print ('run ' + ($i | into string) + ': PASS (audit 档：自退 + 无 panic + 10 步全过 + 13 marker 齐 + 站点表：无孤儿/无死键/无活站点)')
    } else {
      print ('run ' + ($i | into string) + ': FAIL (audit 档) — ' + $r.why + ' —— 现场留在 ' + ($r.dir | into string))
    }
  }

  # harden 轮（仅当 EXAMINE_HARDEN=1）：跑 release + debug-assertions 那份产物，
  # 判「没有 lockdep 违规」+ 那两条探针。轮次编号接在前面几档之后。
  if $harden {
    let i = $repeat + (if $audit { 2 } else { 1 })
    let r = (run_once $cfg $i "harden")
    $total_rounds += 1
    if $r.ok {
      $pass += 1
      print ('run ' + ($i | into string) + ': PASS (harden 档：自退 + 无 panic + 无 lockdep 违规 + 10 步全过 + 11 marker 齐)')
    } else {
      print ('run ' + ($i | into string) + ': FAIL (harden 档) — ' + $r.why + ' —— 现场留在 ' + ($r.dir | into string))
    }
  }

  print $"examine: ($pass)/($total_rounds)"
  exit (if $pass == $total_rounds { 0 } else { 1 })
}
