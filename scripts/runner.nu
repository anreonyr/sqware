#!/usr/bin/env nu

# cargo QEMU runner — riscv64gc-unknown-none-elf
# 由 .cargo/config.toml 的 target.<triple>.runner 触发，cargo 把 ELF 路径追加为位置参数。
#
# 导出模型（semihosting，B 通道）：
#   内核经 semihosting fs 在 **qemu 的 CWD** 创建 sqware-trace.jsonl（JSON Lines；
#   首行 '#' = 宿主时刻溯源）。本脚本先 cd 进 <TRACE_OUT> 再起 qemu，
#   故导出的文件与 console 捕获都落在归档目录：sqware-trace.jsonl 归档为
#   trace-<seed>-<ts>.jsonl；panic 文本场景（[PANIC]）归档为 .log。
#   .jsonl = 事件全量（live 流，panic 只追加自身一行）；.log = 场景补充（CSR/GPR/回溯）。
#
# 约定：kernel/build.rs 已负责构建 user 供 include_bytes! 嵌入，本脚本不再预构建 user；
#       仅 QEMU_FEATURES 非空时二次构建 kernel（带 feature）。
#
# 可配置环境变量:
#   QEMU_EXTRA_ARGS  空格分隔的额外 QEMU 参数，追加在命令行末尾（默认: 无）
#   QEMU_GDB=1       追加 -s（GDB 监听 1234）+ -S（复位后暂停 CPU）
#   QEMU_MEM         内存大小（默认 128M）
#   QEMU_SMP         CPU 核数（默认 4）
#   QEMU_SEED        -icount RNG 种子（默认随机 32 位；同 seed 可复现）
#   TRACE_OUT        归档目录（默认 <project>/trace）
#   QEMU_TIMEOUT     qemu 运行秒数上限（外接 timeout；默认空 = 不限制，如 GDB 场景）
#   QEMU_FEATURES    kernel cargo features（空格分隔；可选追加构建）。
#                   kernel 的 semihosting feature 默认开启，故 QEMU 恒带 -semihosting。

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
  let feats = ($env.QEMU_FEATURES? | default "" | split row ' ' | where { |s| $s != "" })
  # kernel 的 semihosting feature 已默认开启（Cargo.toml default）：QEMU **恒加**
  # -semihosting，否则内核首个 fs ebreak 落成 Breakpoint panic。
  let semihosting = ["-semihosting"]
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
      "-icount", "auto,sleep=on"
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
  rm --force sqware-trace.jsonl   # 干净基线（guest create 本会 truncate，双保险）
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

  # 1) semihosting fs 导出：sqware-trace.jsonl → trace-<seed>-<ts>.jsonl
  let export = "sqware-trace.jsonl"
  if ($export | path exists) {
      let dumped = $"trace-($cfg.seed)-($ts).jsonl"
      mv $export $dumped
      print $"trace exported -> ($cfg.trapdir)/($dumped)"
  }

  # 2) panic 文本场景（halt.rs panic_handler 打 '[PANIC]'）：有则归档 console 捕获。
  let cap = $"sqware-($cfg.seed).cap"
  let panicked = (($cap | path exists) and (open $cap --raw | str contains '[PANIC]'))
  if $panicked {
      let dump = $"trace-($cfg.seed)-($ts).log"
      mv $cap $dump
      print $"panic captured -> ($cfg.trapdir)/($dump)"
  }
}
