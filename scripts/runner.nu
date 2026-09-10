#!/usr/bin/env nu

# cargo QEMU runner — riscv64gc-unknown-none-elf
#
# 用法：`.cargo/config.toml` 的 `target.<triple>.runner` 指向本脚本，cargo 把本次要跑的 ELF
#   追加为位置参数；交互式跑 `QEMU_TIMEOUT=40 cargo run --release`（stdin 上的输入进 guest
#   控制台）。
#
# 职责只有两条：**cargo 集成点** + 交互式跑的**终端流与证据归档**。
#   判定不在这里——什么算通过归验收门 `scripts/e2e.sh`（门自己起 qemu，不走本脚本）。
#   本脚本结尾只打**一行观察**（退出码 + 正常自退 / 被 host 超时杀 / 检出内核 panic）：
#   任何情况都正常结束、**不 exit 1**，故 `cargo run` 的退出码不因内核行为而变。
#
# 证据（都落在 <TRACE_OUT>；qemu 在其中启动 ⇒ semihosting 导出与控制台捕获就地落盘）：
#   diagnose-<seed>-<ts>.jsonl     结构化导出 sqware-diagnose.jsonl，**恒归档**
#   console-<seed>-<ts>.log        终端捕获，只有「该轮看起来正常」时才删，否则归档
#   console-<seed>-<ts>-stale.log  起跑前发现的上一轮残留捕获：先归档，**绝不删**
#   「看起来正常」= qemu 退出码 0 且捕获里无 `[panic] at`。
#   导出需 semihosting 两侧齐备：内核侧 `cargo run … --features semihosting` 决定被跑 ELF 的
#   feature（缺则无导出文件）；QEMU 侧 -semihosting 由 QEMU_SEMI / QEMU_FEATURES 触发。
#
# 可配置环境变量:
#   QEMU_EXTRA_ARGS  空格分隔的额外 QEMU 参数，追加在命令行末尾（默认: 无）
#   QEMU_GDB=1       追加 -s（GDB 监听 1234）+ -S（复位后暂停 CPU）
#   QEMU_MEM         内存大小（默认 128M）
#   QEMU_SMP         CPU 核数（默认 4）
#   QEMU_SEED        -icount RNG 种子（默认随机 32 位；同 seed 可复现）
#   TRACE_OUT        归档目录（默认 <project>/trace）
#   QEMU_TIMEOUT     qemu 运行秒数上限（外接 timeout；默认空 = 不限制，如 GDB 场景）
#   QEMU_FEATURES    仅 QEMU 侧：名称含 semihosting 就给 qemu 加 -semihosting（不触发构建）
#   QEMU_SEMI        设为 1，等价于 QEMU_FEATURES 含 "semihosting"
#
# QEMU 起法（-icount / semihosting 的取舍与代价）、验收门判据与测量记录（semihosting 下诊断
# 事件流把 guest 虚拟时间拖慢约 650×）、以及「门为何不走 runner」见 docs/audit-flying-wires.md
# §9.3。
#
# 已知语义地雷（改本脚本前先读）：
#   - nu 脚本在**外部命令非零退出时当场中止整个脚本**（该语句之后的语句、以及调用方其后的
#     语句都不执行）⇒ qemu 调用必须包在 `try … catch { }` 里，否则被 timeout 杀掉时归档
#     永不执行（旧 runner 的归档分支正是因此从未跑过）。
#   - 裸 `try { ^cmd | tee { … } } catch { }` 保留终端流**与真实退出码**（实测 exit 7 → 7、
#     被 timeout 杀 → 124）；`do -i` 也继续执行，但把退出码清成 0（退出码这个观测量就丢了）。
#   - nu 不支持 `<` 输入重定向。
#   - `o+e>|` 把 stderr 并进捕获（qemu 那行 `terminating on signal … (/usr/bin/timeout)` 走
#     stderr，不并进来则捕获里没有它——曾据此写过一句永不成立的判定）。

def main [elf: path] {
  let cfg = config $elf

  print $"SEED: ($cfg.seed)"
  let code = (run_qemu $cfg)
  archive $cfg $code
}

def config [elf: path] {
  let proj_root = ($env.FILE_PWD | path dirname)
  # cargo runner 传的 ELF 是**相对路径**（相对调用 cargo 的目录）；run_qemu 会 cd 进归档
  # 目录，故这里先展开成绝对路径，cd 后 -kernel 仍有效。
  let elf = ($elf | path expand)
  let extra = ($env.QEMU_EXTRA_ARGS? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let gdb = if ($env.QEMU_GDB? == "1") { ["-s", "-S"] } else { [] }

  # QEMU_FEATURES / QEMU_SEMI 的**唯一**含义：最终 feats 含 "semihosting" 就给 qemu 加
  # -semihosting。它不触发任何构建——被跑内核的 feature 只由调用方 cargo 决定。
  let semi_requested = ($env.QEMU_SEMI? | default "0") == "1"
  let feats_initial = ($env.QEMU_FEATURES? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let feats = if $semi_requested and not ($feats_initial | any { |f| $f == "semihosting" }) {
      $feats_initial | append "semihosting"
  } else {
      $feats_initial
  }
  let semihosting = if ($feats | any { |f| $f == "semihosting" }) { ["-semihosting"] } else { [] }

  let seed = ($env.QEMU_SEED? | default (random binary 4 | into int))
  let trapdir = ($env.TRACE_OUT? | default ($proj_root | path join "trace"))
  mkdir $trapdir
  let timeout = ($env.QEMU_TIMEOUT? | default "")

  # initrd blob（build.rs 打包，与内核 ELF 同目录）：存在才传 -initrd。
  let initrd = ($elf | path dirname | path join "initrd.img")
  let initrd_arg = if ($initrd | path exists) { ["-initrd", $initrd] } else { [] }

  {
    trapdir: $trapdir
    seed: $seed
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
      # -icount 只为「同 seed 可复现」（取舍见头注所指 §9.3）。
      "-icount", "auto,sleep=on"
      ...$semihosting
      ...$initrd_arg
      ...$gdb
      ...$extra
    ]
  }
}

def run_qemu [cfg: record] {
  # 起 qemu 前 cd 进归档目录：导出文件与 console 捕获都就地落盘，消除对调用者 CWD 的
  # 隐式依赖（-kernel/-bios 均为绝对路径，cd 无损）。
  cd $cfg.trapdir
  rm --force sqware-diagnose.jsonl   # 干净基线（guest create 本会 truncate，双保险）
  # 上一轮的残留捕获：先归档再起跑，**不删**——终端捕获是「挂在哪一步」的唯一直接证据。
  rescue
  let cap = $"sqware-($cfg.seed).cap"
  # 勿包进 let —— let 会把外部输出吞掉，终端看不到（"我看不到输出"的根因）。
  # **必须包裸 `try`（不是 `do -i`）**，并用 `o+e>|` 合并 stderr：语义差异见头注「语义地雷」。
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

def archive [cfg: record, code: int] {
  cd $cfg.trapdir
  let ts = (date now | format date '%Y%m%d-%H%M%S')
  let export = "sqware-diagnose.jsonl"
  let cap = $"sqware-($cfg.seed).cap"

  # 两个观测量（都不是判据）：qemu 退出码（run_qemu 已返回）+ 捕获里的内核 panic 报告头。
  let text = if ($cap | path exists) { open $cap --raw } else { "" }
  let panicked = ($text | str contains "[panic] at")
  # 「看起来正常」= 自退（退出码 0）且无 panic —— 只有这一档才敢丢终端捕获。
  let normal = ($code == 0) and (not $panicked)

  # 1) 结构化导出：**恒归档**（无论成败）——复盘靠它。
  if ($export | path exists) {
      let dumped = $"diagnose-($cfg.seed)-($ts).jsonl"
      mv $export $dumped
      print $"诊断导出 -> ($cfg.trapdir)/($dumped)"
  }

  # 2) 终端捕获：只有「看起来正常」的一轮才丢；其余一律归档。
  if ($cap | path exists) {
      if $normal {
          rm --force $cap
      } else {
          let dump = $"console-($cfg.seed)-($ts).log"
          mv $cap $dump
          print $"捕获归档 -> ($cfg.trapdir)/($dump)"
      }
  }

  # 3) 一行观察（信息，不是判据）：不 exit 1，`cargo run` 的退出码不因内核行为而变。
  #    panic 先于 124：panic 若走到 halt_loop 兜底自旋，必然被 host 超时杀，根因是 panic。
  let cause = if $normal {
      "正常自退"
  } else if $panicked {
      "检出内核 panic"
  } else if $code == 124 {
      "被 host 超时杀"
  } else {
      "非零退出（非超时、无 panic）"
  }
  print $"观察：qemu 退出码 ($code) · ($cause)"
}
