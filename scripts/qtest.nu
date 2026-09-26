#!/usr/bin/env nu

# 跑 `embedded-test` 用例（内核内）。
#
# 它**不是**一套测试框架，只是一层转发：把 [`qemu-args.nu`](qemu-args.nu) 那张**板子参数表**
# 翻成 runner 认的 `--qemu-arg=`，再交给 `cargo qtest`。
# 参数表仍**只有一处**（用户裁定"把 boot.nu 拆出可复用的参数表"）——
# 本文件不含任何一个板子参数的字面量，**只多一格**：`-initrd`（参数表在 `--board-only`
# 下明确让出它，故整机用例要的那张镜像由本文件补上）。
#
# 用法：
#   nu scripts/qtest.nu                              # 跑当前 manifest 的全部用例
#   nu scripts/qtest.nu --package kernel             # 指定包（工作区里用）
#   nu scripts/qtest.nu --package kernel --scene rig # 造 rig 景、带它跑整机那一例
#   nu scripts/qtest.nu -- --list                    # `--` 之后原样转给 cargo-qtest
#
# **一例 = 一张镜像，一次运行 = 一个景**：`cargo-qtest` 没有逐例过滤器（`--help` 里只有
# `--test <目标名>`——那是**测试目标**名，不是用例名），而 `--qemu-arg=` 是**整次运行**的
# ⇒ 一张镜像只能服务一轮。故整机用例只有**一例**（`tests/embedded.rs` 的 `scene`），
# 跑七个景就是七次调用。`--scene` 给的景名由本脚本打印出来——报告里那一行只说 `scene`，
# 景在这一行。
#
# **先装那个 runner**（它是个宿主工具，不在仓里）：
#
#   cargo install cargo-qemu-test --target x86_64-unknown-linux-gnu   # ⇒ cargo-qtest
#
# **照实记（`--target` 那一格是必须的）**：本工作区的 `.cargo/config.toml` 把
# `[build] target` 钉在 riscv 上，而 `cargo install` **也吃这一格** ⇒ 不带
# `--target x86_64-unknown-linux-gnu` 会拿 riscv 去编这个宿主工具，编出来一堆
# `cannot find trait PartialEq`（`std` 不在场）。实测踩过。
#
# **照实记（`--smp` 那一格原本默认成 1，已撤）**：原先这里写"`-smp 4` 必炸——`_start`
# 每颗 hart 都会跑一遍 ⇒ 4 颗一起冲进 `__embedded_test_start`、并发读 semihosting 命令行"。
# **量下来那句是错的**：它只对 `-bios none`（M 态、所有 hart 从复位向量起跑）成立，
# 而参数表给的是 `-bios SBI.bin`——S 态下**只有引导 hart 进内核**，副核由
# `boot::boot_harts()` 经 HSM 拉起（**产品路本来就是 `-smp 4`**，`rig`/`soak` 的判据
# 前提正是"空核替全局兑现到点"）。实测 `QEMU_SMP=4` 下原八例 8/8 绿、1.92 s。
# 故这里不再改 `-smp` **默认**，跟参数表走；`QEMU_SMP` 显式设了当然也听调用方的。
# 那条结论有钉子：`kernel/src/health/hart.rs` 的 `hart_multi` 用例。
# **照实记（`icount` 那一格仍与"起机那条路"不同）**：这里把 `icount` 置空。旧的门
# 统一关掉它（"按宿主时间节流会让 guest 与输入日程失步"，见 `qemu-args.nu` 的头注）；
# 用例要在同一档下可比，故这里也关。

def main [--package: string, --scene: string, --profile: string, ...rest: string] {
  if ($env.QEMU_ICOUNT? | is-empty) { $env.QEMU_ICOUNT = "" }

  let script_dir = $env.FILE_PWD
  let board = (^nu ($script_dir | path join "qemu-args.nu") --board-only
      | lines | where { |l| ($l | str trim) != "" })
  let qemu_args = ($board | each { |a| $"--qemu-arg=($a)" })

  # 景：先造镜像，再把 `-initrd` 指过去（**绝对路径**——runner 的工作目录不是仓根）。
  # 档默认 release：`cargo image` 自己也是这个默认，而 `rig` 的照实记说 release 是
  # 它跑得动的前提（debug 下每轮都挂在 20 ms 那一缝上）。
  let scene = if ($scene | is-empty) {
    { args: [] }
  } else {
    let prof = if ($profile | is-empty) { "release" } else { $profile }
    let at = (
      ^cargo image $scene $prof
      | lines
      | where { |l| ($l | str starts-with "ok") }
      | last
      | str replace "ok" ""
      | str trim
    )
    if not ($at | path exists) {
      error make {msg: $"造不出镜像：cargo image ($scene) ($prof) 说落在 `($at)`"}
    }
    print $"景 ($scene)（档 ($prof)）· 镜像 ($at)"
    { args: ["--qemu-arg=-initrd" $"--qemu-arg=($at)"] }
  }

  let pkg = if ($package | is-empty) { [] } else { ["--package" $package] }

  ^cargo qtest --target riscv64gc-unknown-none-elf ...$qemu_args ...$scene.args ...$pkg ...$rest
  exit $env.LAST_EXIT_CODE
}
