#!/usr/bin/env nu

# cargo QEMU runner — riscv64gc-unknown-none-elf
# 由 .cargo/config.toml 的 target.<triple>.runner 触发，cargo 把 ELF 路径追加为位置参数。
#
# 导出模型（semihosting）：
#   内核经 semihosting fs 在 **qemu 的 CWD** 创建 sqware-diagnose.jsonl（JSON Lines；
#   首行 '#' = 宿主时刻溯源）。本脚本先 cd 进 <TRACE_OUT> 再起 qemu，故导出文件
#   与 console 捕获都落在归档目录。**semihosting 必须同时满足两件事**：内核侧
#   `cargo run … --features semihosting`（决定被跑 ELF 的 feature），QEMU 侧
#   -semihosting（由 QEMU_SEMI=1 或 QEMU_FEATURES 含 semihosting 触发）。缺内核侧
#   ⇒ 无导出文件，判定退化为「无导出」。
#
# 判定（三个可观测量，不依赖 semihosting）：
#   killed   = host 侧 timeout 杀了 qemu（捕获里的 `terminating on signal … (/usr/bin/timeout)`）
#   panicked = 结构化 halt:"panic"（导出在时）或控制台 `[panic] at`（内核 panic 报告头）
#   halted   = 结构化 halt:"halt"（EventKind::Halt 经 serde 外部标签落成
#              "kind":{"halt":"halt"}；整行形如 {"h":0,"when":…,"kind":{"room":{"spawn":{"tid":1}}}}）
#   QEMU_EXPECT 给期望：halt（期望**自行退出**且无 panic；导出在时还要求停机记录）/
#   panic（验证崩溃通道）/ any（**默认**，只保证 panic 不被放过）。
#
#   **为什么 halt 不要求结构化记录**：semihosting + `-icount auto` 下，诊断事件流
#   （每个 room park/wake/envcall 都写一条）经 ebreak 落宿主文件，实测 guest 虚拟时间
#   被拖慢约 650×（60 s 墙钟只推进 92 ms，5245 条事件），e2e 在超时前根本走不完
#   ⇒ 结构化导出只适合小规模短跑，不能当 e2e 判据。故门槛取「自行退出」：qemu 正常
#   停机（srst）时自己退出、捕获里没有 timeout 那行；被超时杀则必然没有自退。
#
# 证据策略（终端捕获只在一轮判定通过时才丢）：
#   - 结构化导出**恒归档** → diagnose-<seed>-<ts>.jsonl
#   - 终端捕获只在「判定通过」时 rm；否则归档 → console-<seed>-<ts>.log
#   - 起跑前发现上一轮残留 cap（上轮被信号杀死 / 未收尾）→ 归档为 …-stale.log，**不删**
#
# 约定：kernel/build.rs 已负责构建 task 并打包 initrd，本脚本不再预构建；
#       QEMU_FEATURES/QEMU_SEMI 只影响 QEMU 侧参数（-semihosting）与一次「带 feature
#       的二次构建」——后者**不决定 QEMU boot 的产物**（boot 的是 cargo 传进来的 ELF），
#       故此处只提示、不再声称它让被跑内核带上 feature。
#
# 可配置环境变量:
#   QEMU_EXTRA_ARGS  空格分隔的额外 QEMU 参数，追加在命令行末尾（默认: 无）
#   QEMU_GDB=1       追加 -s（GDB 监听 1234）+ -S（复位后暂停 CPU）
#   QEMU_MEM         内存大小（默认 128M）
#   QEMU_SMP         CPU 核数（默认 4）
#   QEMU_SEED        -icount RNG 种子（默认随机 32 位；同 seed 可复现）
#   TRACE_OUT        归档目录（默认 <project>/trace）
#   QEMU_TIMEOUT     qemu 运行秒数上限（外接 timeout；默认空 = 不限制，如 GDB 场景）
#   QEMU_FEATURES    QEMU 侧 feature 名（空格分隔；含 semihosting 则加 -semihosting）
#   QEMU_SEMI        设为 1 时启用 semihosting（等价于 QEMU_FEATURES 含 "semihosting"）
#   QEMU_EXPECT      停机期望：halt / panic / any（默认 any）
#
# 已知语义地雷（改本脚本前先读）：
#   nu 脚本在**外部命令非零退出时当场中止整个脚本**（实测：该语句之后的语句、
#   以及调用方之后的语句都不执行），故 qemu 调用必须包在 `do -i { … }` 里，
#   否则被 timeout 杀掉时 archive 永不执行（旧版正是如此）。

# 结构化导出里的停机标志（serde 外部标签：kind.halt = "halt" | "panic"）。
const MARK_HALT = '"halt":"halt"'
const MARK_PANIC = '"halt":"panic"'

def main [elf: path] {
  let cfg = config $elf

  # 二次构建（旧行为，保留但**不再声称**它决定被跑内核的 feature）：
  # QEMU boot 的是 cargo 传进来的 $elf，其 feature 集合只由调用方 cargo 决定。
  if ($cfg.feats | length) > 0 {
      print $"kernel build --features ($cfg.feats | str join ',')（仅构建，不影响本次 boot 的 ELF）"
      # 该构建失败即中止整个脚本（nu 的语义：外部命令非零退出当场中止），无需再判 LAST_EXIT_CODE。
      ^cargo b -p kernel --features ($cfg.feats | str join ',')
      if ($cfg.feats | any { |f| $f == "semihosting" }) {
          print "  提示：本次 boot 的 ELF 要有 semihosting，需调用方 `cargo run … --features semihosting`，否则无结构化导出。"
      }
  }

  print $"SEED: ($cfg.seed)"
  let code = (run_qemu $cfg)
  archive $cfg $code
}

def config [elf: path] {
  let proj_root = ($env.FILE_PWD | path dirname)
  # cargo runner 传的 ELF 是**相对路径**（相对调用 cargo 的目录）；run_qemu 会
  # cd 进归档目录，故这里先展开成绝对路径，cd 后 -kernel 仍有效。
  let elf = ($elf | path expand)
  let extra = ($env.QEMU_EXTRA_ARGS? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let gdb = if ($env.QEMU_GDB? == "1") { ["-s", "-S"] } else { [] }

  # --- semihosting 强关联逻辑（QEMU 侧）---
  let semi_requested = ($env.QEMU_SEMI? | default "0") == "1"
  let feats_initial = ($env.QEMU_FEATURES? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let feats = if $semi_requested and not ($feats_initial | any { |f| $f == "semihosting" }) {
      $feats_initial | append "semihosting"
  } else {
      $feats_initial
  }
  # 只要最终 feats 包含 "semihosting"，QEMU 就加 -semihosting
  let semihosting = if ($feats | any { |f| $f == "semihosting" }) { ["-semihosting"] } else { [] }
  # --------------------------------

  let seed = ($env.QEMU_SEED? | default (random binary 4 | into int))
  let trapdir = ($env.TRACE_OUT? | default ($proj_root | path join "trace"))
  mkdir $trapdir
  let timeout = ($env.QEMU_TIMEOUT? | default "")

  # initrd blob（build.rs 打包，与内核 ELF 同目录）：存在才传 -initrd。
  let initrd = ($elf | path dirname | path join "initrd.img")
  let initrd_arg = if ($initrd | path exists) { ["-initrd", $initrd] } else { [] }

  {
    proj_root: $proj_root
    trapdir: $trapdir
    seed: $seed
    feats: $feats
    timeout: $timeout
    qemu_args: [
      "-machine", "virt"
      "-bios", ($proj_root | path join "SBI.bin")
      "-kernel", $elf
      "-nographic"
      "-no-reboot"
      "-m", ($env.QEMU_MEM? | default "128")
      "-smp", ($env.QEMU_SMP? | default "4")
      "-seed", $seed
      # sleep=off：icount 下禁止宿主随 vCPU 空闲而休眠
      # off 保持时钟连续单调，seed 复现能力不变。
      "-icount", "auto,sleep=on"
      ...$semihosting
      ...$initrd_arg
      ...$gdb
      ...$extra
    ]
  }
}

def run_qemu [cfg: record] {
  # 起 qemu 前 cd 进归档目录：导出文件与 console 捕获都就地落盘，消除对
  # 调用者 CWD 的隐式依赖（-kernel/-bios 均为绝对路径，cd 无损）。
  cd $cfg.trapdir
  rm --force sqware-diagnose.jsonl   # 干净基线（guest create 本会 truncate，双保险）
  # 上一轮的残留捕获：先归档再起跑，**不删**——终端捕获是「挂在哪一步」的唯一直接证据。
  rescue
  let cap = $"sqware-($cfg.seed).cap"
  # 勿包进 let —— let 会把外部输出吞掉，终端看不到（"我看不到输出"的根因）。
  #
  # **必须包裸 `try`（不是 `do -i`）**，两侧语义都实测过：
  #   - 不包：nu 脚本在外部命令非零退出时**当场中止整个脚本**（其后语句、以及调用方
  #     里其后的语句都不执行）⇒ 被 timeout 杀掉时 `archive` 永不执行，判定与归档恰好
  #     在最需要它们的失败路径上全部失效（旧版 runner 的 panic 归档分支因此从未可达）。
  #   - `do -i`：能继续执行，但把 LAST_EXIT_CODE 清成 0 ⇒ 退出码这一判据丢失。
  #   - 裸 `try … catch {}`：保留终端流、保留真实退出码（实测 exit 7 → 7、timeout 杀 → 124）。
  # `o+e>|` 把 stderr 并进捕获 —— qemu 自己那行 `terminating on signal … (/usr/bin/timeout)`
  # 走的是 stderr，不并进来则捕获里根本没有它（曾经据此写过一个永不成立的判定）。
  if ($cfg.timeout | is-empty) {
      try { ^qemu-system-riscv64 ...$cfg.qemu_args o+e>| tee { save --force $cap } } catch { }
  } else {
      try { ^timeout $cfg.timeout qemu-system-riscv64 ...$cfg.qemu_args o+e>| tee { save --force $cap } } catch { }
  }
  $env.LAST_EXIT_CODE
}

# 上一轮未收尾的终端捕获 → 归档（不删除）。
def rescue [] {
  let ts = (date now | format date '%Y%m%d-%H%M%S')
  for f in (glob "sqware-*.cap") {
      let seed = (($f | path basename) | str replace --regex '^sqware-' '' | str replace --regex '\.cap$' '')
      let dump = $"console-($seed)-($ts)-stale.log"
      mv $f $dump
      print $"上轮未收尾的捕获 -> ($dump)"
  }
}

# 判定：只看结构化导出。返回 { seen, halted, panicked, want, ok, shape }。
def verdict [cfg: record, code: int] {
  let export = "sqware-diagnose.jsonl"
  let cap = $"sqware-($cfg.seed).cap"
  let seen = ($export | path exists)
  let raw = if $seen { open $export --raw } else { "" }
  let text = if ($cap | path exists) { open $cap --raw } else { "" }

  # 判据都取「本机可观测量」，不依赖 semihosting（原因见头注：semihosting + icount 下
  # guest 虚拟时间被事件流拖慢约 650×，e2e 根本跑不完）：
  #   clean    = qemu 自行退出且无错（0）；停机走 srst ⇒ 正是这一档
  #   killed   = host 侧 timeout 杀（退出码 124；捕获里那行 stderr 作二次确认）
  #   panicked = 结构化 halt:"panic"（若导出在）或控制台 `[panic] at`（内核 panic 报告头）
  #   halted   = 结构化 halt:"halt"（仅 semihosting 档可见；不在则不作要求）
  let clean = ($code == 0)
  let killed = (($code == 124) or ($text | str contains "terminating on signal"))
  let panicked = (($raw | str contains $MARK_PANIC) or ($text | str contains "[panic] at"))
  let halted = ($raw | str contains $MARK_HALT)
  let want = ($env.QEMU_EXPECT? | default "any")
  # 导出若不存在（未开 semihosting）就不要求停机记录，否则要求它与自退一致。
  let halt_ok = if $seen { $halted } else { true }
  let ok = match $want {
      "halt" => ($clean and (not $panicked) and $halt_ok),
      "panic" => $panicked,
      _ => (not $panicked),
  }
  let shape = if $panicked {
      "panic"
  } else if $killed {
      "被超时杀（未自行退出 ⇒ 未走到停机）"
  } else if (not $clean) {
      $"非零退出（code ($code)）⇒ 非正常停机"
  } else if ($seen and (not $halted)) {
      "自退但导出里无停机记录"
  } else if $seen {
      "如期停机（结构化确认）"
  } else {
      "如期自退（code 0；无结构化导出，按自退 + 无 panic 判定）"
  }
  { seen: $seen, halted: $halted, panicked: $panicked, killed: $killed, clean: $clean, code: $code, want: $want, ok: $ok, shape: $shape }
}

def archive [cfg: record, code: int] {
  cd $cfg.trapdir
  let ts = (date now | format date '%Y%m%d-%H%M%S')
  let export = "sqware-diagnose.jsonl"
  let cap = $"sqware-($cfg.seed).cap"
  let v = (verdict $cfg $code)

  # 1) 结构化导出：**恒归档**（无论成败）——判定与复盘都靠它。先归档，再判退出。
  if ($export | path exists) {
      let dumped = $"diagnose-($cfg.seed)-($ts).jsonl"
      mv $export $dumped
      print $"诊断导出 -> ($cfg.trapdir)/($dumped)"
  }

  # 2) 终端捕获：只有一轮判定通过才丢（"通过的一次不留档"）；其余一律归档。
  if ($cap | path exists) {
      if $v.ok {
          rm --force $cap
      } else {
          let dump = $"console-($cfg.seed)-($ts).log"
          mv $cap $dump
          print $"捕获归档 -> ($cfg.trapdir)/($dump)"
      }
  }

  # 3) 判定（显式期望不满足 ⇒ 非零退出，验收门不再靠人眼比对）。
  print $"判定[期望 ($v.want)]: ($v.shape)（qemu code ($v.code)）"
  if not $v.ok {
      # 注意别写成 $"FAIL(seed …)：`(` 紧跟文本会让 nu 把 `FAIL(...)` 当命令调用，
      # 于是 FAIL 路径自己崩掉、诊断信息丢失（语义仍是退码 1，但现场说明没了）。
      print $"FAIL · seed=($cfg.seed) · 期望=($v.want) · 实测=($v.shape)"
      exit 1
  }
}
