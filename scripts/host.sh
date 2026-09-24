#!/bin/sh
# 宿主档的门 —— **八处纯核心**的规矩，在**宿主**上真跑一遍：
# 树（八条原语）· 线（账 + 界）· 门禁（三格裁决 + 那本账）· 名册与盟籍（两本册子）·
# 板（牌子 + 台账）· 会话（码头 / 泊位 / 认领那台机器）· 编排（账 / 判定 / 配给）·
# 供单（需求单 / 回单 / 上限与失败码表）。
#
# # 那一台在哪、有几个靶
#
# **一个**编外 crate：`crates/protocol-case`。八个测试靶（靶名 = 原来那八个 crate 名去后缀）：
#
#   operator · line · judge · roster · board · quay · judgement · supply
#
# 每个靶 `#[path]` 把若干份**逐字未改**的协议源码编进来——哪一份进哪个靶、为什么，写在那个
# crate 的 `Cargo.toml` 头注与各靶自己的头注里（**话要能指回源头**，这里不抄第二份）。
#
# **照实记（这一格是用户裁定改的）**：原先**是八个 crate**（`operator-case` / `line-case` / …），
# 彼此只差一个文件名：八份 `Cargo.toml` + 八份 `Cargo.lock` + 八个 `target/`，而 `env` 被编了
# 八遍。用户的话是 **"我希望测试和运行环境分开，而不是交叉在一起"** ⇒ 收成一台。同一刀还做了：
#
#   1) **34 条 `#[cfg(test)]` 用例从协议源码里搬出来**（`principal` / `coalition` / `board` /
#      `judge` / `gate` 五份）——"测试不许住在运行时源里"。其中 `judge` / `gate` 那 14 条与
#      `judge` 靶里**更强的同名判据**重复，故**删掉**、只把独有的那几处并进台里（那几处写在
#      `tests/judge.rs` 与那两份源码的末段）。
#   2) **5 条"纯常量读数"用例交给编译器**：4 份"面不相撞"改成协议源码里的
#      `const _: () = assert!(…)`（**编译期**，riscv 那一档也一样钉着），`supply` 那条长度表整条
#      删（`WANT_LEN == 32` 早就是编译期断言，另三条是同义反复）。⇒ **137 → 118 例**。
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
# 故走 `crates/alloc-probe` 那条现成的路：**一个编外**（根 `Cargo.toml` 的 `exclude`）的宿主
# crate，八个靶各把若干份**逐字未改**的核心源码 `#[path]` 编进来：
#
#   `operator`   `operator/core.rs`（只依赖 `env`）
#   `line`       `driver/line/core.rs`（账 + 界）+ `driver/line/call.rs`（帧那一半，**零切分**）
#   `judge`      `operator/judge.rs` + `operator/gate.rs` + `operator/ledger.rs` +
#                `operator/core.rs`（**只为账要的那几样类型**）+ `operator/frame.rs`
#   `roster`     `principal/core.rs` + `coalition/core.rs` —— **两本册子同住一个靶**：盟籍那一份
#                写着 `use crate::principal::core::PrincipalId`，分两处各编一遍会让名册那批
#                跑两遍；两面的 `frame.rs` 也跟着进来（`tests/{principal,coalition}/` 是它们
#                要的**真实目录模块**）
#   `board`      `system/board/core.rs`（靶里那个模块的名字就叫 `core`——帧那一份写的是
#                `use super::core::{…}`）+ `system/board/frame.rs`
#   `quay`       `session/core.rs`（**这一台的桩最大**：会话核心只跟运行时那一层说几句话
#                （铸孔 / 交出 / 放下 / 推 / 收 / 扫表 / 等 / 时钟），故靶里给了一张**进程内的
#                假表** + 一枚**假钟**——于是"有界等"在宿主上是确定性的）
#   `judgement`  `system/desk.rs`（账）+ `system/core.rs`（判定）+ `system/grant.rs`（配给）——**无桩**
#   `supply`     `driver/supply/call.rs`（帧 / 荷载 / 上限）+ `driver/supply/core.rs`（五格失败域）
#                ——**无桩**
#
# **照实记（那 19 条是"救活"的）**：`principal/core.rs` / `coalition/core.rs` / `board/core.rs`
# 各自的 `#[cfg(test)]` 模块**从写下那天起一次没跑过**（`protocol` 编不到、也没有别的靶编它）——
# 那些文件的头注当时就写着"只是契约的读数，不是门"。靶一开，**19 条**（7 + 6 + 6）当场有了门。
# 它们后来按用户裁定搬去了 `roster` / `board` 两个靶（源里从此没有测试）。
#
# **照实记（`line` 靶多一处桩）**：线的核心写的是"有主那一格"，故它 `use crate::session::Pier`
# ——靶里给了一个**桩**（`Pier` 只要 `post` 一句：核心只跟泊位说这一句话，读/写泊位那一侧
# 在适配层）。桩量不了会话，量得了账——那个靶钉的就是账的界。
#
# **照实记（树那两个靶是一份源码的两半，不是一个靶的两半）**：`operator` 钉树的**结构**
# （七条原语、六格失败域），`judge` 钉**门外那一问**（三格裁决：放行 / 终态拒 / 判不了）。
# 分开的理由是两份源码的纪律不同：`judge.rs` 里两个号是泛型（不认识 `PrincipalId` / `CoalitionId`），
# 故那个靶不需要编身份与结盟的核心源码。
#
# # 判据（四条一起）
#
#   1) `cargo test` **退出码 0**；
#   2) **八个靶每个都要有**汇总行 `test result: ok. N passed; 0 failed`，且 **N = 基线**；
#   3) 全程无 `FAILED` / `panicked`；
#   4) 总和 **= 118**（多一个靶、少一个靶都拦得住）。
#
# **照实记（为什么要有基线）**：这一门原先每台只查"**≥ 1 例**"，于是**少跑**成了唯一看不见的
# 坏消息——那 34 条 `#[cfg(test)]` 用例只在"某个靶恰好 `#[path]` 编了那份源码"的前提下才跑，
# 哪个靶一旦不再编它，它们会**静默消失**而门照样报绿。基线的口径：**改判据就改这里的数**
# （那一改会出现在 diff 里，正是一处该被看见的地方）。
#
# # 为什么显式给 `--target x86_64-unknown-linux-gnu`
#
# 根 `.cargo/config.toml` 把默认目标钉成 riscv；宿主档必须显式换回来，否则又会去编 riscv 的
# libtest（编不出来）。"宿主"这两个字在命令里就是这一句。
#
# # 这份清单盘过：`protocol` 里还有哪些核没有门、为什么
#
# 盘于 2026-09-24（数法：把各靶的 `#[path]` 收成一张表，再看剩下哪些文件里**有函数定义**）；
# 结论分三类，**每一类都有理由**——这样"没有门的档"才有一个说得出口的终点：
#
#   1) **有门**（八个靶，各靶头注列着它编的那几份源码）：树 · 线 · 门禁（`judge` / `gate` /
#      `ledger`）· 名册与盟籍 · 板 · 会话 · 编排（`desk` / `core` / `grant`）· 供单。
#   2) **上不了宿主**（只能由机器那几道门管着）：
#      - 五份 `*/client.rs`（一问一答的往返）与 `operator/call.rs` / `system/board/call.rs` /
#        `session/call.rs` / `driver/supply/call.rs`：它们**拖 `runtime`**（铸孔 / 交出 / 推收），
#        而 `runtime` 在宿主上编不出来（那两处 riscv 内联汇编）——这是这一门的由来，不是漏。
#        纯的那几格（线上码表）在 `gate.rs` 里**另存一份**，并由 `operator/mod.rs` 末尾那条
#        `const _: () = assert!(…)` 在**编译期**钉住（一漂就编不过）。
#      - **五份 `call.rs` 的帧那一半都收进来了**（用户裁定见 `docs/frame-gate.md`）：
#        `line` **零切**（本来就全纯）进 `line` 靶；`principal`/`coalition`/`operator`/`board`
#        各拆成 `frame.rs`（纯）+ `call.rs`（适配，首行 `pub use super::frame::*;` ⇒ **调用点零改**），
#        分别进 `roster` / `judge`（它本来就带着帧要的全部依赖）/ `board` 靶。
#        为此 `fail_codes!` 搬成**自己一份源**（协议与各靶同读，见 `protocol/src/fail_codes.rs`）。
#      - **`driver/supply/call.rs` 也收进来了**（用户裁定见 `docs/supply-gate.md`）：它碰 `runtime`
#        的原本只有一行 `use runtime::core::port::{Access, Policy};`，而那两个类型**长在荷载里**。
#        故把 `Access` / `Policy`（与 `env::Permission` 同层的纯位视图）搬进 `env`
#        （`crates/env/src/wire/access.rs`），`runtime::core::port` 里转出 ⇒ **22 个调用点一行未改**；
#        那一行 `use` 改成 `env` 之后这一份**本来就全纯**（与 `line` 同形：**零切分**），
#        直接编进 `supply` 靶。
#      - 仍归这一类的只剩：`session/call.rs` **没有帧**（它本身就是运行时那一层，在 `quay` 靶里
#        以桩的形式出现）。
#   3) **只有类型、没有判据**：`driver/supply/core.rs`（五格失败域）与各 `mod.rs`（正文）——
#      没有可机械检查的判据，开靶只会得到"零用例"，而零用例这一门本来就判红。
#
# 下一刀若还想加靶：**先看第 2 类**——那里欠的不是"没门"，是"上了会重复跑判据"；
# 要收它得先把那几份 `call.rs` 的纯那半**拆出去**（那是结构改动，不是加靶）。
#
# 用法：
#   scripts/host.sh        # 一轮
# 退出码：全过 0，有不过 1。日志落在 target/host/host-<时间戳>-<pid>.log。
set -u

out=target/host
mkdir -p "$out"
# 日志名 = **秒 + pid**：光精确到秒不够——本脚本一轮只要一两秒，**同一秒里连跑两次**会复用
# 同一份日志，而写是追加的 ⇒ 末了那句 `grep -q 'FAILED' "$log"` 扫到的是**上一轮**的失败
# （实测：一个连跑十几轮的量具把"只改注释"那一轮也判成了红）。加 pid 之后互不相干。
tag="host-$(date +%s)-$$"
log="$out/$tag.log"
sum="$out/$tag.sum"

# `--tests`：只跑测试靶（本 crate 只有测试靶，没有 lib 那一路），不碰文档测试。
# **一次调用跑八个靶**——这也是"收成一台"的收益：`env` 只编一遍（原先八台各编一遍）。
echo "== cargo test crates/protocol-case" >> "$log"
cargo test --manifest-path crates/protocol-case/Cargo.toml \
  --target x86_64-unknown-linux-gnu --tests >> "$log" 2>&1
code=$?
if [ "$code" -ne 0 ]; then
  echo "host: FAIL cargo test 退出码 $code（$log）"
  grep -a -E '^error|FAILED|panicked' "$log" | tail -5
  exit 1
fi

# 每个靶的读数：`Running tests/<靶>.rs` 那一行起，紧跟的那条 `test result:` 就是它。
# 字段位置：`test result: ok. 23 passed; 0 failed; …` ⇒ $3=ok./FAILED. $4=过 $6=败。
awk '
  /^ +Running tests\// { t = $0; sub(/^ +Running tests\//, "", t); sub(/\.rs .*/, "", t); cur = t; next }
  /^test result: /     { if (cur != "") { print cur, $3, $4, $6; cur = "" } }
' "$log" > "$sum"

total=0
bad=0
# 基线：**改判据就要改这里的数**（口径见头注"为什么要有基线"）。
while read -r name want; do
  seen="$(awk -v n="$name" '$1 == n' "$sum")"
  if [ -z "$seen" ]; then
    echo "host: FAIL 靶 $name 没有汇总行（它压根没跑？）"
    bad=1
    continue
  fi
  state="$(printf '%s' "$seen" | awk '{print $2}')"
  have="$(printf '%s' "$seen" | awk '{print $3}')"
  failed="$(printf '%s' "$seen" | awk '{print $4}')"
  if [ "$state" != "ok." ]; then
    echo "host: FAIL 靶 $name 汇总不是 ok.（$state）"
    bad=1
  fi
  if [ "${failed:-0}" -ne 0 ]; then
    echo "host: FAIL 靶 $name 有 $failed 条失败"
    bad=1
  fi
  if [ "${have:-0}" -ne "$want" ]; then
    echo "host: FAIL 靶 $name 用例数 ${have:-0} ≠ 基线 $want —— 删了判据就把基线改小并说明理由，加了判据就改大"
    bad=1
  fi
  echo "  $name: ${have:-0} 例（基线 $want）"
  total=$((total + ${have:-0}))
done <<'TARGETS'
operator 23
line 11
judge 24
roster 20
board 11
quay 12
judgement 10
supply 7
TARGETS

if [ "$total" -ne 118 ]; then
  echo "host: FAIL 总用例数 $total ≠ 118 —— 靶数或判据数变过（逐靶读数见上）"
  bad=1
fi

if grep -q 'FAILED' "$log"; then
  echo "host: FAIL 有用例失败：$(grep -a 'FAILED' "$log" | head -1)"
  bad=1
fi

if [ "$bad" -ne 0 ]; then
  echo "host: FAIL（日志 $log）"
  exit 1
fi
echo "host: $total 例全过（树 + 线 + 门禁 + 名册/盟籍 + 板 + 会话 + 编排 + 供单，日志 $log）"
exit 0
