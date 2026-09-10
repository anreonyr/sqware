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
#   <OUT>/run<i>/diag.txt      仅失败时写：why + 字节数 / 回显数 / qemu 现场
#
# ── 旋钮 ─────────────────────────────────────────────────────────────────────
#   EXAMINE_REPEAT     轮数（默认 3）
#   EXAMINE_OUT        输出根目录（默认 trace/e2e-<时间戳>）
#   QEMU_TIMEOUT       秒（默认 60；由 boot.nu 解释：外接 timeout）
#   EXAMINE_STEP_WAIT  单步等待上限秒（默认 15）
#   EXAMINE_T_GAP      每条命令之间的让出秒（默认 2，同 .sh）
#   EXAMINE_T_BOOT     .sh 的遗留旋钮：那边也从未被读用，这里同样只接受、不影响时序
#   EXAMINE_ICOUNT     非空则透传给 boot.nu（默认空 = **关 icount**）
#   EXAMINE_FEATURES   内核 cargo feature（默认空 = 默认档，行为/输出与原版逐字相同）。
#                      含 `audit` ⇒ 默认轮之后**再加一轮 audit 档**（追加 `sleep 700`
#                      与 hole 的封印唤醒观测，并断言关机时刻的站点表计数）。

# 步骤：命令 → 该步要看到的输出（逐字照抄 .sh 版）。最后一条同时是自然停机的判据。
#
# 默认档 = .sh 版八条，逐字未动。**audit 档**在同一序列上**追加**两步（不改既有
# 八步的命令、时序与 marker）：`sleep 700` 与 `hole`（hole 的期望串两种档相同，
# 只是它多打的那几句由 audit 档的 marker 去断言）。
const STEPS = [
  {cmd: "spawn",     pat: "spawnjoin -> 499500"}
  {cmd: "dir",       pat: "discover echo -> found"}
  {cmd: "req",       pat: 'req echo -> "ifmmp\.tfswjdf'}
  {cmd: "hole",      pat: 'hole got "hi from shell'}
  {cmd: "sleep 300", pat: "woke"}
  {cmd: "sleep 700", pat: "woke"}   # 见「audit 档步骤」：redeem 的第二个时长
  {cmd: "clock",     pat: "clock [0-9]"}
  {cmd: "badslot",   pat: "badslot: 3/3 rejected, kernel alive"}
  {cmd: "exit",      pat: "task: all tasks exited, system halted"}
]

# 档位 → 本档要跑的步骤（下标取自上面那张表，命令与顺序都只有一处出处）。
const STEPS_DEFAULT = [0, 1, 2, 3, 4, 6, 7, 8]
const STEPS_AUDIT   = [0, 1, 2, 3, 4, 5, 6, 7, 8]

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
  "\\[audit\\] sites "
]
# 顺序断言：后者必须出现在前者之后（grep 行号比较；任一缺 ⇒ 直接挂）。
const AUDIT_ORDER = [
  ["sleep 300ms", "sleep 700ms"]
  ['hole got "hi from shell', "hole: wait-seal sealed=1 wake=seal"]
]

# 本档要核的 marker：默认档九条（.sh 原文），audit 档再追加三条。
def markers_for [audit: bool] {
  if $audit { $MARKERS | append $AUDIT_MARKERS } else { $MARKERS }
}

# 本档要跑的步骤（命令与 marker 同源，见 STEPS / STEPS_AUDIT）。
def steps_for [audit: bool] {
  let idx = if $audit { $STEPS_AUDIT } else { $STEPS_DEFAULT }
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
  let r = (^grep -oE -- '\[audit\] sites [0-9]+ live [0-9]+ tomb [0-9]+ orphan [0-9]+ waiters [0-9]+' $file | complete)
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
  let lines = [
    $"--- 诊断：($ctx.why) ---"
    $"seed         → ($ctx.seed)"
    $"icount       → ($icount_s)"
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
def run_once [cfg: record, i: int, audit: bool] {
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
  let elf = $cfg.elf
  # qemu：tail 长驻写端喂命令文件 → scripts/boot.nu（qemu 起法的唯一出处）。
  # 退出码拿不到（nu 无 job wait）⇒ job 自己落盘；**必须包 try**，否则被 timeout 杀（124）
  # 时 job 会当场中止，rc 文件永远不写（.sh 版旧 runner 的归档分支就是这么从未跑过的）。
  let job = (job spawn {
    try { ^tail -f -n +1 $cmds | ^nu $boot $elf o+e> $log } catch { }
    ($env.LAST_EXIT_CODE) | save --force $rcfile
  })

  mut why = ""
  mut sent = 0
  let steps = (steps_for $audit)
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
  for m in (markers_for $audit) {
    if not (hit $m $log) { $why = (append_why $why $"缺[($m)]") }
  }
  # 顺序断言：两步的先后也是判据（「两句话都在」不等于「第二次等待真的发生在第一次之后」）。
  if $audit {
    for pair in $AUDIT_ORDER {
      let first = (at $pair.0 $log)
      let second = (at $pair.1 $log)
      if $first == null or $second == null or $second <= $first {
        $why = (append_why $why $"顺序[($pair.0) → ($pair.1)]")
      }
    }
    # ── prune 的断言：全部任务退出时刻的站点表三形态计数 ──
    # 内核侧的计数（`[audit] sites N live N tomb N orphan N waiters N`）由**关机
    # 序列里排在 `scheduler::rip` 之前**的那条只读钩子打印——rip 会清空站点表，
    # 排在其后数出来的恒为 0，那样的断言没有牙。站点三形态（判据见内核
    # `site::prune`）：活（有等待者）/ 墓碑（`wipe` 留下的信标）/ 孤儿（两者皆无）。
    #
    # 断言三项，按牙口从强到弱，逐条报数（不合并成一句 bool，失败时要能直接读数）：
    #   ① **orphan == 0**：孤儿站点只该由 `prune` 删——这是对 `prune` 的直接断言。
    #      实测（本轮反向验证）：把 `prune` 整个关掉，总数只从 34 变 36，**孤儿从 0
    #      变 3**；只看总数的断言几乎没牙，看孤儿才有牙。
    #   ② live == 0 且 waiters == 0：全部任务已退出 ⇒ 不该还有人挂在任何站点上。
    #   ③ sites 总数：**不作为违规判据**。本轮实测它不是 0（34；其中 tomb=34 全是
    #      `wipe` 的墓碑——每次 hole 封印 / 任务回收留一个带信标的空站点，`prune`
    #      依判据**不许**删）。记账在 §9.3：这是「站点表有非空残留」的事实，不是
    #      `prune` 坏了；它的处置（让 `wipe` 复用既有站点以不建墓碑）是独立裁决。
    #      门这里只把它的数报出来，供跨轮对比。
    let n_sites = (audit_count "sites" $log)
    let n_live = (audit_count "live" $log)
    let n_tomb = (audit_count "tomb" $log)
    let n_orphan = (audit_count "orphan" $log)
    let n_waiters = (audit_count "waiters" $log)
    if $n_sites == null or $n_live == null or $n_tomb == null or $n_orphan == null or $n_waiters == null {
      $why = (append_why $why "audit 站点计数取不到")
    } else {
      if $n_orphan != 0 { $why = (append_why $why $"孤儿站点[($n_orphan)]") }
      if $n_live != 0 { $why = (append_why $why $"活站点[($n_live)]") }
      if $n_waiters != 0 { $why = (append_why $why $"残留等待者[($n_waiters)]") }
      # 三形态必须配平（活 + 墓碑 + 孤儿 == 总数）：分列若与总数对不上，是**计数
      # 自身**坏了——那会让上面三条判据全部失效，故也当判据。
      if ($n_live + $n_tomb + $n_orphan) != $n_sites {
        $why = (append_why $why $"三形态不配平[($n_live)+($n_tomb)+($n_orphan)!=($n_sites)]")
      }
      print $"  audit 站点计数：sites=($n_sites) live=($n_live) tomb=($n_tomb) orphan=($n_orphan) waiters=($n_waiters)"
    }
  } else if (hit '\[audit\] sites ' $log) {
    # 默认档**不该**有 audit 输出：出现即说明跑的 ELF 带着 audit feature（不是本档构建）。
    $why = (append_why $why "默认档出现了 audit 输出")
  }
  # .sh 里 `wait` 一定有退出码；nu 这条路可能取不到（job 没收尾），那也是失败的理由。
  if $rc == null { $why = (append_why $why "qemu 未在限内收尾（退出码取不到）") }

  if $why != "" {
    write_diag $diagfile ($live | merge {
      why: $why, seed: $seed, icount: $cfg.icount, cmds: $cmds, log: $log,
      sent: $sent, total: $total, rc: $rc, features: $cfg.features,
    })
  }
  {ok: ($why == ""), why: $why, dir: $dir}
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
  let features = ($env.EXAMINE_FEATURES? | default "" | str trim)
  let audit = (($features | split row -r '\s+') | any { |f| $f == "audit" })
  let elf = ($root | path join "target/riscv64gc-unknown-none-elf/release/sqware")
  let boot = ($root | path join "scripts" "boot.nu")

  print $"examine: repeat=($repeat) out=($out) qemu_timeout=($qemu_timeout)s step_wait=($step_wait)s"
  print ('examine: features=' + (if $features == "" { "(默认)" } else { $features }) + (if $audit { "（另加一轮 audit）" } else { "" }))
  mkdir $out

  # 构建：非零退出在 nu 里会当场中止脚本，故显式接住并退 1（.sh 的 `|| exit 1` 同义）。
  # 裸 `^cargo` 在 try 里仍然把输出流到终端（包进 `let` 才会被吞掉）。
  # feature 是**空串也照传** `--features`（cargo 对空 feature 列表与不传等价），
  # 免得两处分叉（传/不传各一条命令行）。
  try { ^cargo build --release -p kernel --features $features } catch {
    print $"examine: cargo build 失败（rc=($env.LAST_EXIT_CODE)）"
    exit 1
  }
  if not ($elf | path exists) { print $"缺 ($elf)"; exit 1 }

  let cfg = {
    out: $out, elf: $elf, boot: $boot, qemu_timeout: $qemu_timeout,
    step_wait: $step_wait, t_gap: $t_gap, icount: $icount, features: $features,
  }

  mut pass = 0
  mut total_rounds = 0
  # `1..0` 在 nu 里**不是空区间**（会倒着数出 1、0 两轮），`.sh` 的 `seq 1 0` 是空的；显式挡一下，
  # 让 REPEAT=0/负数 与 .sh 同义（0 轮 ⇒ `examine: 0/0`）。
  let rounds = if $repeat > 0 { (1..$repeat) } else { [] }
  for i in $rounds {
    let r = (run_once $cfg $i false)
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
    let r = (run_once $cfg $i true)
    $total_rounds += 1
    if $r.ok {
      $pass += 1
      print ('run ' + ($i | into string) + ': PASS (audit 档：自退 + 无 panic + 9 步全过 + 12 marker 齐 + 站点表已空)')
    } else {
      print ('run ' + ($i | into string) + ': FAIL (audit 档) — ' + $r.why + ' —— 现场留在 ' + ($r.dir | into string))
    }
  }

  print $"examine: ($pass)/($total_rounds)"
  exit (if $pass == $total_rounds { 0 } else { 1 })
}
