#!/bin/sh
# 宿主档的门 —— 核心（树与七条原语）的规矩，在**宿主**上真跑一遍。
#
# # 为什么要有这一门
#
# `protocol` 的 `[lib] test = false`：目标 `riscv64gc-unknown-none-elf` 上**编不出 libtest**
# （libtest 依赖 std）。于是 `cargo check --workspace --all-targets` 与 `build --release`
# **都不编**那批用例——树与七条原语的规矩长期只有"写着的规格"、没有"跑着的判据"。
# 教训与前一句是同一件事：**没有门的档 = 没有编译过的档**（`scripts/framework.sh` 头注那次
# 事故是同一句话的另一处）。
#
# # 为什么门开在一个**编外**的宿主 crate 上
#
# 两条实测读数把"在 protocol 里加个集成测试"那条路堵死了：
#
#   1) `[lib] test = false` 关的只是 lib 的单元测试目标，集成测试确实还能立——但**链接
#      `protocol` 就编不过**：它依赖 `runtime`，而 `runtime/src/core/tls.rs` 的两处 riscv
#      内联汇编（`mv tp, …`）在宿主上 `invalid instruction mnemonic 'mv'`；
#   2) **`cargo check` 在同一个目标上是过的**（它不做代码生成）——所以"protocol 在宿主编得过"
#      这句话只有在 check 那一档才成立。本门第一版就栽在这一格上（照实记）。
#
# 故走 `crates/alloc-probe` 那条现成的路：`crates/operator-case` 是**编外**（根 workspace
# `exclude`）的宿主 crate，只依赖 `env`，把 `crates/protocol/src/operator/core.rs`
# **逐字未改**地 `include!` 进来。门开在它身上。
#
# # 判据（三条一起）
#
#   1) `cargo test` 退出码 0；
#   2) 末行汇总 `test result: ok. N passed; 0 failed`，且 **N ≥ 1**（零用例要红：测试靶没被
#      发现就等于白立一档——`framework.sh` 的"静默零用例"是同一条顾虑）；
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

# `--tests`：只跑测试靶（本 crate 只有测试靶，没有 lib 那一路），不碰文档测试。
cargo test --manifest-path crates/operator-case/Cargo.toml \
  --target x86_64-unknown-linux-gnu --tests > "$log" 2>&1
code=$?
summary="$(grep -a '^test result: ' "$log" | tail -1)"
passed="$(printf '%s' "$summary" | sed -n 's/^test result: ok\. \([0-9]*\) passed.*/\1/p')"

if [ "$code" -ne 0 ]; then
  echo "host: FAIL 退出码 $code（$log）"
  grep -a -E '^error|FAILED|panicked' "$log" | head -5
  exit 1
fi
if [ -z "$summary" ]; then
  echo "host: FAIL 无汇总行（$log）"
  exit 1
fi
if [ "${passed:-0}" -lt 1 ]; then
  echo "host: FAIL 零用例（$summary）—— 测试靶没被发现，比失败更坏"
  exit 1
fi
if grep -q 'FAILED' "$log"; then
  echo "host: FAIL 有用例失败：$(grep -a 'FAILED' "$log" | head -1)"
  exit 1
fi
echo "host: $passed 例全过 · $summary"
exit 0
