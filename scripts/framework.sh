#!/bin/sh
# 框架档的门 —— 内核内用例 + 挂起自检（`--features framework`）。
#
# # 为什么要有这一门（它第一次跑就照出了洞）
#
# 这一档此前**没有任何门开它**：`grep --features scripts/` 只命中 `fast.sh` 的
# `FAST_FEATURES` 变量（一个空壳）。而 `framework` 是**内核 crate 的 feature**：
# 不开它，`framework/` 与 `health/` 的用例、以及 `weak` 的出身槽位账与挂起自检
# **根本不进编译**。于是：
#
# **照实记**：hart 号那一刀（`dbaa2cf`）在 `fetch.rs` 的 WFI 钩子上传的是 `HartId`，
# 而钩子那头 `ipi::wfi_entry` / `wfi_exit` 仍收 `usize` —— 两处类型洞。当时所有门全绿
# （`cargo check --workspace --all-targets` 与 `build --release` 都不开这个 feature），
# 直到本门第一次把这一档编出来才响。教训与前一句正是同一件事：**没有门的档 = 没有编译过的档**。
#
# # 判据（三条一起）
#
#   1) 运行器末行汇总 `[case] cases N ok M fail 0`，且 **N ≥ 1**。零用例要红：`.tests`
#      段没有引用会被链接器丢掉，症状是**静默零用例**（框架头注点名"比失败更坏"）。
#   2) 无 `[case] FAIL`、无 `[panic]`。用例失败走 panic 通道（报"哪一例 + 哪一行"再停机），
#      故失败轮**留不下**汇总行。
#   3) `task: all tasks exited, system halted`。**通过 = 放行启动**（用例跑完接着起 shell），
#      故本门要的是"用例过了**并且**整机照常收尾"——只跑了用例就停机那一版是错的
#      （见 `runner::run` 的照实记）。
#
# # 档位与喂键
#
# `--profile framework --features framework`：profile 只管构建参数、feature 管代码，两者
# 要一起给（根 `Cargo.toml` 的注）。该 profile 继承 `harden` ⇒ `debug_assertions` 开着，
# 故账里的 `debug_assert` 与自检里的断言**真的在跑**。
#
# 喂键与 `soak.sh` 同一教训：**等 `echo: ready` 再喂**，不按钟表喂——定时喂键会与启动期
# 赛跑（`soak.sh` 文件头那两次假红记的就是这件事）。框架档启动更慢（多一批用例），
# 故等它的上限给到 60 秒。
#
# 用法：
#   scripts/framework.sh [轮数]        # 默认 1 轮
#   scripts/framework.sh 3             # 连跑三轮
# 退出码：全过 0，有不过 1。日志落在 target/framework/framework-<时间戳>-<轮>.log。
set -u

# ── 环境对齐（必须）：显式**关掉** icount ──
#
# 与验收门 / 压测台 / 共享组台同一条理由：`boot.nu` 默认 `-icount auto,sleep=on` 会让
# WFI 里的核被 IPI 叫醒等额度（毫秒级）。本门虽不判"到得快不快"，但**判据要可比**。
QEMU_ICOUNT=
export QEMU_ICOUNT

rounds="${1:-1}"
out=target/framework
mkdir -p "$out"
tag="framework-$(date +%s)"
pass=0
i=1
while [ "$i" -le "$rounds" ]; do
  log="$out/$tag-$i.log"
  # 喂键器：等 shell 起稳（`echo: ready`）再喂 `exit`；上限 60 秒兜底照喂。
  # 第二条 `exit` 是保险（等 shell 之后本轮往往在第一条就收尾，第二条会撞上断开的
  # 管道——那是预期的收尾，故 stderr 闭掉）。
  (
    waited=0
    while [ "$waited" -lt 60 ] && ! grep -q "echo: ready" "$log" 2>/dev/null; do
      sleep 1
      waited=$((waited + 1))
    done
    echo exit
    sleep 3
    echo exit
  ) 2>/dev/null | timeout 180 cargo run --profile framework --features framework > "$log" 2>&1
  summary="$(grep -a '^\[case\] cases ' "$log" | tail -1)"
  ncases="$(printf '%s' "$summary" | sed -n 's/^\[case\] cases \([0-9]*\) .*/\1/p')"
  nok="$(grep -a -c '^\[case\] ok ' "$log")"
  if [ -z "$summary" ]; then
    echo "round $i: FAIL 无用例汇总行（$log）"
  elif [ "${ncases:-0}" -lt 1 ]; then
    echo "round $i: FAIL 零用例（$summary）—— 段被丢了，比失败更坏"
  elif grep -q '\[case\] FAIL' "$log"; then
    echo "round $i: FAIL 用例失败：$(grep -a '\[case\] FAIL' "$log" | head -1)"
  elif grep -q '\[panic\]' "$log"; then
    echo "round $i: FAIL 内核 panic"
  elif ! grep -q "task: all tasks exited, system halted" "$log"; then
    echo "round $i: FAIL 无停机行（用例过了，收尾没到）"
  else
    pass=$((pass + 1))
    echo "round $i: $nok 例逐行 ok · $summary"
  fi
  i=$((i + 1))
done
echo "== 通过 $pass/$rounds（日志 $out/$tag-*.log）=="
[ "$pass" -eq "$rounds" ]
