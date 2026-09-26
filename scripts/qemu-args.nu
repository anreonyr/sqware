#!/usr/bin/env nu

# QEMU **参数表的唯一出处**（用户裁定"把 boot.nu 拆出可复用的参数表"）。
#
# 它只做一件事：按环境变量凑出一张参数表，**不起机、不判定、不碰 stdin**。
# 起机归 `scripts/boot.nu`；测试路的转发归 `scripts/qtest.nu`。
#
# 两个消费者，一张表：
#   boot.nu    `use qemu-args.nu qemu-args` → 拿**全表**（含 `-kernel` 与 `-semihosting`）
#   qtest.nu   `nu qemu-args.nu --board-only` → 拿**板子那一半**。
#              embedded-test 的 runner 自己会加 `-kernel` 与
#              `-semihosting-config enable=on,target=native`，故那一半必须**让出去**，
#              否则同一格出现两次（实测：`-bios` / `-m` 这类后出现者胜，但 semihosting
#              的两个开关不是同一个选项名，会各留一份）。
#
# 环境变量（与拆分前逐字相同）：
#   QEMU_TIMEOUT  秒；空或 0 = 不限制（外接 timeout）
#   QEMU_SEED     整数；空 = 用 qemu 自己的随机（本脚本不改写它）
#   QEMU_ICOUNT   非空则加 `-icount <值>`（默认 auto,sleep=on）；**置空可关**。
#   QEMU_MEM / QEMU_SMP / QEMU_EXTRA_ARGS / QEMU_GDB
#   QEMU_SEMI / QEMU_FEATURES   含 semihosting ⇒ 加 `-semihosting`（**只管 QEMU 侧**：
#                               被跑 ELF 的 feature 由调用方 cargo 决定）。
#
# 串口只绑 stdio、**不复用 monitor**：`-nographic` 等价 `-serial mon:stdio`，mux 会把 stdin
# 按模式分派给串口或 monitor，一旦切走就是静默改道 ⇒ 显式写
# `-display none -serial stdio -monitor none`。代价：Ctrl-A c 进 monitor 的用法不再可用。
# （测试路上 runner 会先加一个 `-nographic`；本表这几项排在它后面，后出现者胜，
#  故"串口只绑 stdio"这条口径在测试路上同样成立。）

# 参数表本体。`--board-only` = 让出 `-kernel` 与 `-semihosting*` 两格（见上）。
# `--serial <后端>` = 换掉串口那一格（默认 `stdio`）。整机用例要把串口搬到一条**能喂输入**
# 的通道上（`cargo-qtest` 把 QEMU 的 stdin 钉成 null），见 [`qtest.nu`](qtest.nu) 的头注；
# **串口这一格因此只有一个出处**——`-serial` 不是"后者胜"，写两次会多出一枚串口（实测踩过）。
export def args [elf?: path, --board-only, --serial: string] {
  let root = ($env.FILE_PWD | path dirname)
  let elf = if ($elf | is-empty) { null } else { ($elf | path expand) }
  let extra = ($env.QEMU_EXTRA_ARGS? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let gdb = if ($env.QEMU_GDB? == "1") { ["-s", "-S"] } else { [] }

  let semi_requested = ($env.QEMU_SEMI? | default "0") == "1"
  let feats_initial = ($env.QEMU_FEATURES? | default "" | split row -r '\s+' | where { |s| $s != "" })
  let feats = if $semi_requested and not ($feats_initial | any { |f| $f == "semihosting" }) {
      $feats_initial | append "semihosting"
  } else {
      $feats_initial
  }
  # 板子那一半让出 semihosting：runner 自己会加 `-semihosting-config`。
  let semihosting = if (not $board_only) and ($feats | any { |f| $f == "semihosting" }) {
      ["-semihosting"]
  } else {
      []
  }

  let icount = ($env.QEMU_ICOUNT? | default "auto,sleep=on")
  let icount_args = if ($icount | is-empty) { [] } else { ["-icount", $icount] }
  let seed = ($env.QEMU_SEED? | default "")
  let seed_args = if ($seed | is-empty) { [] } else { ["-seed", $seed] }

  # initrd blob（与内核 ELF 同目录）：存在才传 `-initrd`。板子那一半**不传**——
  # 内核内用例跑在起服务之前，用不到它。
  let initrd_arg = if $board_only or ($elf == null) {
      []
  } else {
      let initrd = ($elf | path dirname | path join "initrd.img")
      if ($initrd | path exists) { ["-initrd", $initrd] } else { [] }
  }

  let kernel_arg = if $board_only or ($elf == null) { [] } else { ["-kernel", ($elf | into string)] }

  let serial_arg = if ($serial | is-empty) { ["-serial", "stdio"] } else { ["-serial", $serial] }

  [
    "-machine", "virt"
    "-bios", ($root | path join "SBI.bin")
    ...$kernel_arg
    "-display", "none"
    ...$serial_arg
    "-monitor", "none"
    "-no-reboot"
    # 默认 256：**实测的边界**（两次都量过）——
    #   ① 原先 128：debug 那份 initrd 64.6 MB（release 31.2 MB）与 13.8 MB 的内核放一起，
    #      QEMU 报 "Not enough memory to place DTB after kernel/initrd"；抬到 192 起得来。
    #   ② **照实记**：认设备那一刀（编排域多一份设备树解析）之后，debug 那份 initrd 长到
    #      98.1 MB（release 48.1 MB）——192 MB 下同一句报错**每一轮都红**（`soak` 0/10、
    #      `group` 0/3），抬到 256 又起来。**边界量出来就是这样，别把它当"启动慢"。**
    "-m", ($env.QEMU_MEM? | default "256")
    "-smp", ($env.QEMU_SMP? | default "4")
    ...$seed_args
    ...$icount_args
    ...$semihosting
    ...$initrd_arg
    ...$gdb
    ...$extra
  ]
}

# 给外壳消费者用：一行一个参数（`qtest.nu` 靠它拼 `--qemu-arg=`）。
def main [elf?: path, --board-only, --serial: string] {
  for a in (args $elf --board-only=$board_only --serial=$serial) { print $a }
}
