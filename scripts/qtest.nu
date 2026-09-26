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
# **照实记（两处默认与"起机那条路"不同）**：
#
#   ① `-smp 1`。内核对 `-smp 4` 的默认是为**整机场景**定的；而 embedded-test 的入口是
#      `_start → main`，**每颗 hart 都会跑一遍** ⇒ 4 颗一起冲进 `__embedded_test_start`、
#      并发读 semihosting 命令行，是必炸的。参数表里那颗旋钮仍经 `QEMU_SMP` 给，
#      这里只把**默认**改成 1（调用方显式设了就听调用方的）。
#   ② `icount` 置空。旧的门统一关掉它（"按宿主时间节流会让 guest 与输入日程失步"，
#      见 `qemu-args.nu` 的头注）；用例要在同一档下可比，故这里也关。

def main [--package: string, ...rest: string] {
  if ($env.QEMU_SMP? | is-empty) { $env.QEMU_SMP = "1" }
  if ($env.QEMU_ICOUNT? | is-empty) { $env.QEMU_ICOUNT = "" }

  let script_dir = $env.FILE_PWD
  let board = (^nu ($script_dir | path join "qemu-args.nu") --board-only
      | lines | where { |l| ($l | str trim) != "" })
  let qemu_args = ($board | each { |a| $"--qemu-arg=($a)" })
  let pkg = if ($package | is-empty) { [] } else { ["--package" $package] }

  ^cargo qtest --target riscv64gc-unknown-none-elf ...$qemu_args ...$pkg ...$rest
  exit $env.LAST_EXIT_CODE
}
