#!/usr/bin/env nu

# 跑 `embedded-test` 用例（内核内）。
#
# 它**不是**一套测试框架，只是一层转发：把 [`qemu-args.nu`](qemu-args.nu) 那张**板子参数表**
# 翻成 runner 认的 `--qemu-arg=`，再交给 `cargo qtest`。
# 参数表仍**只有一处**（用户裁定"把 boot.nu 拆出可复用的参数表"）——
# 本文件不含任何一个板子参数的字面量。
#
# 用法：
#   nu scripts/qtest.nu                       # 跑当前 manifest 的全部用例
#   nu scripts/qtest.nu --package kernel      # 指定包（工作区里用）
#   nu scripts/qtest.nu -- --list             # `--` 之后原样转给 cargo-qtest
#
# **先装那个 runner**（它是个宿主工具，不在仓里）：
#
#   cargo install cargo-qemu-test --target x86_64-unknown-linux-gnu   # ⇒ cargo-qtest
#
# **照实记（`--target` 那一格是必须的）**：本工作区的 `.cargo/config.toml` 把
# `[build] target` 钉在 riscv 上，而 `cargo install` **也吃这一格** ⇒ 不带
# `--target x86_64-unknown-linux-gnu` 会拿 riscv 去编这个宿主工具，编出来一堆
# `cannot find trait PartialEq`（`std` 不在场）。实测踩过。
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

def main [--package: string, ...rest: string] {
  if ($env.QEMU_ICOUNT? | is-empty) { $env.QEMU_ICOUNT = "" }

  let script_dir = $env.FILE_PWD
  let board = (^nu ($script_dir | path join "qemu-args.nu") --board-only
      | lines | where { |l| ($l | str trim) != "" })
  let qemu_args = ($board | each { |a| $"--qemu-arg=($a)" })
  let pkg = if ($package | is-empty) { [] } else { ["--package" $package] }

  ^cargo qtest --target riscv64gc-unknown-none-elf ...$qemu_args ...$pkg ...$rest
  exit $env.LAST_EXIT_CODE
}
