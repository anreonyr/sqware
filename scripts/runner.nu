#!/usr/bin/env nu

# cargo QEMU runner — riscv64gc-unknown-none-elf
# 由 .cargo/config.toml 的 target.<triple>.runner 触发，cargo 把 ELF 路径追加为位置参数。
#
# 导出模型（semihosting，B 通道）：
#   内核经 semihosting fs 在 **qemu 的 CWD** 创建 sqware-diagnose.jsonl（JSON Lines；
#   首行 '#' = 宿主时刻溯源）。本脚本先 cd 进 <TRACE_OUT> 再起 qemu，故导出文件
#   与 console 捕获都落在归档目录：sqware-diagnose.jsonl 归档为
#   diagnose-<seed>-<ts>.jsonl；panic（jsonl 含 "kind":"halt" 记录）时 console
#   捕获归档为 console-<seed>-<ts>.log。
#   .jsonl = diagnose 族全量（事件 live 流 + panic 的 halt 记录与 scene 现场行）；
#   .log  = 完整终端捕获（含诊断文本；panic 判定改走 halt 记录，不再靠字符串匹配）。
#
# 约定：kernel/build.rs 已负责构建 user 供 include_bytes! 嵌入，本脚本不再预构建 user；
#       仅 QEMU_FEATURES 非空或 QEMU_SEMI=1 时二次构建 kernel（带 feature）。
#
# semihosting 强关联：
#   - kernel 的 semihosting feature 与 QEMU 的 -semihosting 参数必须同时启用或禁用。
#   - 本脚本以最终 feats 列表是否包含 "semihosting" 为唯一依据：若包含则 QEMU 加参数；
#     因此可通过两种方式启用：QEMU_SEMI=1 或 QEMU_FEATURES 中显式包含 "semihosting"。
#   - 默认（两者均未设置）不启用 semihosting，内核不会触发相关 ebreak。
#
# 可配置环境变量:
#   QEMU_EXTRA_ARGS  空格分隔的额外 QEMU 参数，追加在命令行末尾（默认: 无）
#   QEMU_GDB=1       追加 -s（GDB 监听 1234）+ -S（复位后暂停 CPU）
#   QEMU_MEM         内存大小（默认 128M）
#   QEMU_SMP         CPU 核数（默认 4）
#   QEMU_SEED        -icount RNG 种子（默认随机 32 位；同 seed 可复现）
#   TRACE_OUT        归档目录（默认 <project>/trace）
#   QEMU_TIMEOUT     qemu 运行秒数上限（外接 timeout；默认空 = 不限制，如 GDB 场景）
#   QEMU_FEATURES    kernel cargo features（空格分隔；可选追加构建）
#   QEMU_SEMI        设为 1 时启用 semihosting（等价于在 QEMU_FEATURES 中添加 "semihosting"）

def main [elf: path] {
  let cfg = config $elf

  # 仅 feats 非空才二次构建（cargo run 已建过一次无 feature 的 kernel）
  if ($cfg.feats | length) > 0 {
      print $"kernel build --features ($cfg.feats | str join ',')"
      ^cargo b -p kernel --features ($cfg.feats | str join ',')
      if $env.LAST_EXIT_CODE != 0 { exit 1 }
  }

  print $"SEED: ($cfg.seed)"
  run_qemu $cfg
  archive $cfg
}

def config [elf: path] {
  let proj_root = ($env.FILE_PWD | path dirname)
  # cargo runner 传的 ELF 是**相对路径**（相对调用 cargo 的目录）；run_qemu 会
  # cd 进归档目录，故这里先展开成绝对路径，cd 后 -kernel 仍有效。
  let elf = ($elf | path expand)
  let extra = ($env.QEMU_EXTRA_ARGS? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let gdb = if ($env.QEMU_GDB? == "1") { ["-s", "-S"] } else { [] }

  # --- semihosting 强关联逻辑 ---
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

  {
    proj_root: $proj_root
    trapdir: $trapdir
    seed: $seed
    feats: $feats
    timeout: $timeout
    qemu_args: [
      "-machine", "virt"
      "-bios", ($proj_root | path join "SBI")
      "-kernel", $elf
      "-nographic"
      "-no-reboot"
      "-m", ($env.QEMU_MEM? | default "128")
      "-smp", ($env.QEMU_SMP? | default "4")
      "-seed", $seed
      # sleep=off：icount 下禁止宿主随 vCPU 空闲而休眠——虚拟时钟冻结会让 guest
      # 的 time CSR 停摆/快进（多核判据、定时器唤醒系统性失真的根因，见
      # watch.rs WALL/Suspect）；off 保持时钟连续单调，seed 复现能力不变。
      "-icount", "auto,sleep=off"
      ...$semihosting
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
  let cap = $"sqware-($cfg.seed).cap"
  # 勿包进 let —— let 会把外部输出吞掉，终端看不到（"我看不到输出"的根因）。
  if ($cfg.timeout | is-empty) {
      ^qemu-system-riscv64 ...$cfg.qemu_args | tee { save --force $cap }
  } else {
      ^timeout $cfg.timeout qemu-system-riscv64 ...$cfg.qemu_args | tee { save --force $cap }
  }
}

def archive [cfg: record] {
  cd $cfg.trapdir
  let ts = (date now | format date '%Y%m%d-%H%M%S')

  # 1) panic 判定（jsonl 含 halt 记录）→ 归档 console 捕获为 console-<seed>-<ts>.log。
  #    判定走结构化导出（"kind":"halt"），不依赖终端文本匹配。判定先于改名。
  let export = "sqware-diagnose.jsonl"
  let cap = $"sqware-($cfg.seed).cap"
  let panicked = (($export | path exists) and (open $export --raw | str contains '"kind":"halt"'))
  if $panicked and ($cap | path exists) {
      let dump = $"console-($cfg.seed)-($ts).log"
      mv $cap $dump
      print $"panic console captured -> ($cfg.trapdir)/($dump)"
  }

  # 2) 诊断导出：sqware-diagnose.jsonl → diagnose-<seed>-<ts>.jsonl
  if ($export | path exists) {
      let dumped = $"diagnose-($cfg.seed)-($ts).jsonl"
      mv $export $dumped
      print $"diagnose exported -> ($cfg.trapdir)/($dumped)"
  }
}
