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
export def args [elf?: path, --board-only, --serial: string, --iommu, --endpoint: string, --endpoint-hold: string] {
  if not ($endpoint_hold | is-empty) and ($endpoint | is-empty) {
    error make {msg: "--endpoint-hold 需要 --endpoint 指定夹具磁盘"}
  }
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

  # 平台总线上的 RISC-V IOMMU：**只有要它的那一景**才加（`--iommu`）。
  # 它一开，PCI 根总线的端点 DMA 就都过它，而复位态是 Off（设备访存不翻译）——
  # 故不加这一格的那些轮次里，机器上没有任何端点过 IOMMU。
  # 它的寄存器窗口是设备树里的一段 `reg`（与其它设备同一条路登记、`platform_window`
  # 已把它挡在设备账外），不占 PCI aperture。
  let machine = if $iommu { "virt,iommu-sys=on" } else { "virt" }

  # 真实 PCI 端点：**只有要它的那一景**才加（`--endpoint <镜像文件>`）。
  # 它是 `virtio-blk-pci`（不是测具）；盘是调用方给的一张**可再生**的小镜像，
  # 只读挂上（`readonly=on`）——端点那一景只读它，不会改仓里任何一张真盘。
  # `iommu_platform=on` 是**那一格的关键**：不上它，QEMU 的 virtio 设备
  # 拿的是 `address_space_memory`（`virtio_bus_device_plugged` 的那一句默认），
  # 端点发的地址根本不过 `iommu-map` 指的那台 IOMMU——实测过：不带它时
  # 一次 `riscv_iommu_dma` 都不出现，端点静默停在读不到可用环那一格。
  # `disable-legacy=on` 是前面的代价：QEMU 不给过渡态开那两个一起用
  # （`VIRTIO_F_IOMMU_PLATFORM was supported by neither legacy nor transitional device`），
  # 而本驱动走的就是 1.x 那套寄存器，传统 I/O BAR 那一格本来也不用。
  let endpoint = if ($endpoint | is-empty) {
      []
  } else {
      let drive = if ($endpoint_hold | is-empty) {
          $"file=($endpoint),format=raw,if=none,readonly=on,id=iommu-dma"
      } else {
          $"file.driver=nbd,file.server.type=unix,file.server.path=($endpoint_hold),format=raw,if=none,readonly=on,id=iommu-dma"
      }
      [
          "-drive" $drive
          "-device" "virtio-blk-pci,drive=iommu-dma,iommu_platform=on,disable-legacy=on"
      ]
  }

  [
    "-machine", $machine
    "-bios", ($root | path join "SBI.bin")
    ...$kernel_arg
    "-display", "none"
    ...$serial_arg
    "-monitor", "none"
    "-no-reboot"
    "-m", ($env.QEMU_MEM? | default "256")
    "-smp", ($env.QEMU_SMP? | default "4")
    ...$seed_args
    ...$icount_args
    ...$semihosting
    ...$initrd_arg
    ...$endpoint
    ...$gdb
    ...$extra
  ]
}

# 给外壳消费者用：一行一个参数（`qtest.nu` 靠它拼 `--qemu-arg=`）。
def main [elf?: path, --board-only, --serial: string, --iommu, --endpoint: string, --endpoint-hold: string] {
  for a in (args $elf --board-only=$board_only --serial=$serial --iommu=$iommu --endpoint=$endpoint --endpoint-hold=$endpoint_hold) { print $a }
}
