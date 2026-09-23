#!/bin/sh
# 宿主档的门 —— 两处**纯核心**的规矩，在**宿主**上真跑一遍：树（七条原语）· 线（账 + 界）。
#
# # 为什么要有这一门
#
# `protocol` 的 `[lib] test = false`：目标 `riscv64gc-unknown-none-elf` 上**编不出 libtest**
# （libtest 依赖 std）。于是 `cargo check --workspace --all-targets` 与 `build --release`
# **都不编**那批用例——核心的规矩长期只有"写着的规格"、没有"跑着的判据"。
# 教训与前一句是同一件事：**没有门的档 = 没有编译过的档**（`scripts/framework.sh` 头注那次
# 事故是同一句话的另一处）。
#
# # 为什么门开在**编外**的宿主 crate 上
#
# 两条实测读数把"在 protocol 里加个集成测试"那条路堵死了：
#
#   1) `[lib] test = false` 关的只是 lib 的单元测试目标，集成测试确实还能立——但**链接
#      `protocol` 就编不过**：它依赖 `runtime`，而 `runtime/src/core/tls.rs` 的两处 riscv
#      内联汇编（`mv tp, …`）在宿主上 `invalid instruction mnemonic 'mv'`；
#   2) **`cargo check` 在同一个目标上是过的**（它不做代码生成）——所以"protocol 在宿主编得过"
#      这句话只有在 check 那一档才成立。本门第一版就栽在这一格上（照实记）。
#
# 故走 `crates/alloc-probe` 那条现成的路：三台**编外**（根 workspace `exclude`）的宿主 crate
# 各把一份**逐字未改**的核心源码 `#[path]` 编进自己的测试靶：
#
#   `crates/operator-case`  `crates/protocol/src/operator/core.rs`（只依赖 `env`）
#   `crates/line-case`      `crates/protocol/src/driver/line/core.rs`
#   `crates/judge-case`     `crates/protocol/src/operator/judge.rs`（只依赖 `env`）
#
# **照实记（线那一台多一处桩）**：线的核心写的是"有主那一格"，故它 `use crate::session::Pier`
# ——宿主靶里给了一个**桩**（`Pier` 只要 `post` 一句：核心只跟泊位说这一句话，读/写泊位那一侧
# 在适配层）。桩量不了会话，量得了账——那一台钉的就是账的界。
#
# **照实记（树那两台是一份源码的两半，不是一个台子的两半）**：`operator-case` 钉树的**结构**
# （七条原语、六格失败域），`judge-case` 钉**门外那一问**（三格裁决：放行 / 终态拒 / 判不了）。
# 分开的理由是两份源码的纪律不同：`judge.rs` 里两个号是泛型（不认识 `PrincipalId` / `CoalitionId`），
# 故那一台不需要编身份与结盟的核心源码。
#
# # 判据（三条一起）
#
#   1) 三台 `cargo test` **退出码都 0**；
#   2) 每台的末行汇总 `test result: ok. N passed; 0 failed`，且 **N ≥ 1**（零用例要红：测试靶
#      没被发现就等于白立一档——`framework.sh` 的"静默零用例"是同一条顾虑）；
#   3) 全程无 `FAILED` / `panicked`。
#
# # 为什么显式给 `--target x86_64-unknown-linux-gnu`
#
# 根 `.cargo/config.toml` 把默认目标钉成 riscv；宿主档必须显式换回来，否则又会去编 riscv 的
# libtest（编不出来）。"宿主"这两个字在命令里就是这一句。
#
# 用法：
#   scripts/host.sh        # 一轮
# 退出码：全过 0，有不过 1。日志落在 target/host/host-<时间戳>.log。
set -u

out=target/host
mkdir -p "$out"
tag="host-$(date +%s)"
log="$out/$tag.log"
total=0

# `--tests`：只跑测试靶（三台都只有测试靶，没有 lib 那一路），不碰文档测试。
for m in crates/operator-case crates/line-case crates/judge-case; do
  echo "== $m" >> "$log"
  cargo test --manifest-path "$m/Cargo.toml" \
    --target x86_64-unknown-linux-gnu --tests >> "$log" 2>&1
  code=$?
  if [ "$code" -ne 0 ]; then
    echo "host: FAIL $m 退出码 $code（$log）"
    grep -a -E '^error|FAILED|panicked' "$log" | tail -5
    exit 1
  fi
  summary="$(grep -a '^test result: ' "$log" | tail -1)"
  passed="$(printf '%s' "$summary" | sed -n 's/^test result: ok\. \([0-9]*\) passed.*/\1/p')"
  if [ -z "$summary" ]; then
    echo "host: FAIL $m 无汇总行（$log）"
    exit 1
  fi
  if [ "${passed:-0}" -lt 1 ]; then
    echo "host: FAIL $m 零用例（$summary）—— 测试靶没被发现，比失败更坏"
    exit 1
  fi
  echo "  $m: $summary"
  total=$((total + passed))
done

if grep -q 'FAILED' "$log"; then
  echo "host: FAIL 有用例失败：$(grep -a 'FAILED' "$log" | head -1)"
  exit 1
fi
echo "host: $total 例全过（树 + 线 + 门禁，日志 $log）"
exit 0
