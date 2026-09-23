#!/bin/sh
# 宿主档的门 —— **七处纯核心**的规矩，在**宿主**上真跑一遍：
# 树（八条原语）· 线（账 + 界）· 门禁（三格裁决 + 那本账）· 名册与盟籍（两本册子）·
# 板（牌子 + 台账）· 会话（码头 / 泊位 / 认领那台机器）· 编排（账 / 判定 / 配给）。
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
# 故走 `crates/alloc-probe` 那条现成的路：七台**编外**（根 workspace `exclude`）的宿主 crate
# 各把若干份**逐字未改**的核心源码 `#[path]` 编进自己的测试靶：
#
#   `crates/operator-case`   `crates/protocol/src/operator/core.rs`（只依赖 `env`）
#   `crates/line-case`       `crates/protocol/src/driver/line/core.rs`
#   `crates/judge-case`      `crates/protocol/src/operator/judge.rs`（只依赖 `env`）
#   `crates/principal-case`  `crates/protocol/src/principal/core.rs` +
#                            `crates/protocol/src/coalition/core.rs`（盟籍那一份 `use
#                            crate::principal::core::PrincipalId` ⇒ 两本册子同住一台，
#                            免得那一份核心在两台里各编一遍、用例跑两遍）
#   `crates/board-case`      `crates/protocol/src/system/board/core.rs`
#   `crates/system-case`     `crates/protocol/src/system/desk.rs`（账）+
#                            `crates/protocol/src/system/core.rs`（判定）+
#                            `crates/protocol/src/system/grant.rs`（配给）——**无桩**
#   `crates/session-case`    `crates/protocol/src/session/core.rs`（**这一台桩最大**：会话核心
#                            只跟运行时那一层说几句话（铸孔 / 交出 / 放下 / 推 / 收 / 扫表 /
#                            等 / 时钟），故靶里给了一张**进程内的假表** + 一枚**假钟** ——
#                            于是"有界等"在宿主上是确定性的）
#
# **照实记（后两台是"救活"的）**：`principal/core.rs` / `coalition/core.rs` / `board/core.rs`
# 各自的 `#[cfg(test)]` 模块**从写下那天起一次没跑过**（`protocol` 编不到、也没有别的靶编它）——
# 那些文件的头注当时就写着"只是契约的读数，不是门"。后两台一开，**19 条**（7 + 6 + 6）当场有了门。
# 其中唯一一处需要搭桥的是盟籍那一份的 `use crate::principal::core::PrincipalId`：宿主靶里给它
# 一个**真实的目录模块** `tests/principal/mod.rs`（那块地照实记了"内联模块编不过"那一次）。
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
#   1) 七台 `cargo test` **退出码都 0**；
#   2) 每台的末行汇总 `test result: ok. N passed; 0 failed`，且 **N ≥ 1**（零用例要红：测试靶
#      没被发现就等于白立一档——`framework.sh` 的"静默零用例"是同一条顾虑）；
#   3) 全程无 `FAILED` / `panicked`。
#
# # 为什么显式给 `--target x86_64-unknown-linux-gnu`
#
# 根 `.cargo/config.toml` 把默认目标钉成 riscv；宿主档必须显式换回来，否则又会去编 riscv 的
# libtest（编不出来）。"宿主"这两个字在命令里就是这一句。
#
# # 这份清单盘过：`protocol` 里还有哪些核没有门、为什么
#
# 盘于 2026-09-24（数法：把每台的 `#[path]` 收成一张表，再看剩下哪些文件里**有函数定义**）；
# 结论分三类，**每一类都有理由**——这样"没有门的档"才有一个说得出口的终点：
#
#   1) **有门**（七台，上面那 13 份源码）：树 · 线 · 门禁（`judge` / `gate` / `ledger`）·
#      名册与盟籍 · 板 · 会话 · 编排（`desk` / `core` / `grant`）。
#   2) **上不了宿主**（只能由机器那几道门管着）：
#      - 五份 `*/client.rs`（一问一答的往返）与 `operator/call.rs` / `system/board/call.rs` /
#        `session/call.rs` / `driver/supply/call.rs`：它们**拖 `runtime`**（铸孔 / 交出 / 推收），
#        而 `runtime` 在宿主上编不出来（那两处 riscv 内联汇编）——这是这一门的由来，不是漏。
#        纯的那几格（线上码表）在 `gate.rs` 里**另存一份**，并由 `operator/mod.rs` 末尾那条
#        `const _: () = assert!(…)` 在**编译期**钉住（一漂就编不过）。
#      - `coalition/call.rs` / `principal/call.rs` / `driver/line/call.rs` 的 pack/unpack **本身是纯的**，
#        但它们要同层的 `core.rs`（类型）——而那一份的**判据已经在别的台里跑着**，再编一遍就是
#        **同一批判据跑两遍**（本仓明说过不这么干：`judge-case` 头注那条"反过来把 `judge.rs`
#        引进 `operator-case` 会把上面这些再跑一遍"）。故它们的门也是机器那几道。
#   3) **只有类型、没有判据**：`driver/supply/core.rs`（五格失败域）与各 `mod.rs`（正文）——
#      没有可机械检查的判据，开台只会得到"零用例"，而零用例这一门本来就判红。
#
# 下一刀若还想加台：**先看第 2 类**——那里欠的不是"没门"，是"上了会重复跑判据"；
# 要收它得先把那几份 `call.rs` 的纯那半**拆出去**（那是结构改动，不是加台）。
#
# 用法：
#   scripts/host.sh        # 一轮
# 退出码：全过 0，有不过 1。日志落在 target/host/host-<时间戳>.log。
set -u

out=target/host
mkdir -p "$out"
# 日志名 = **秒 + pid**：光精确到秒不够——本脚本一轮只要一两秒，**同一秒里连跑两次**会复用
# 同一份日志，而写是追加的 ⇒ 末了那句 `grep -q 'FAILED' "$log"` 扫到的是**上一轮**的失败
# （实测：一个连跑十几轮的量具把"只改注释"那一轮也判成了红）。加 pid 之后互不相干。
tag="host-$(date +%s)-$$"
log="$out/$tag.log"
total=0

# `--tests`：只跑测试靶（三台都只有测试靶，没有 lib 那一路），不碰文档测试。
for m in crates/operator-case crates/line-case crates/judge-case crates/principal-case crates/board-case crates/session-case crates/system-case; do
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
echo "host: $total 例全过（树 + 线 + 门禁 + 名册/盟籍 + 板 + 会话 + 编排，日志 $log）"
exit 0
