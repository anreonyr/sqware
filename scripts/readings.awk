# 声明式读数对账（用户裁定"甲2"）—— **量具，不是门**：它把"哪条读数没人判"这件事
# 从 soak.sh 头注里那句"对账的法子是人工逐条对"变成机械的一步。
#
# 用法（soak.sh 每一轮末尾调它）：
#
#   awk -v asserts=<断言表> -v table=scripts/readings.txt -v report=0 -f scripts/readings.awk <日志>
#
#   `asserts`：**本门那批断言**，一行一条 `kind|pattern`
#              （由 soak.sh 从它自己的 `need` / `needE` / `need_absent` 里抽出来——不另抄一份）。
#   `table`：  读数表（`scripts/readings.txt`）：`前缀 <TAB> auto|narrative <TAB> 理由`
#   `report`： 1 = 只把统计打出来（标定时用），0 = 判红。
#
# 判据（三条）：
#   1) 日志里出现的每一个 `前缀:` 都要在表里（**新读数没人管** ⇒ 红）；
#   2) `auto` 档：这一前缀的**每一行**都要被某条断言匹配（新形状没人判 ⇒ 红）；
#   3) `narrative` 档：至少要有一条断言匹配（且表里必须写了理由）——这一档是"打了但只判其中
#      几条形状的叙事行"，理由要能指回源头。
#
# **照实记（为什么分两档）**：实测一轮 soak 的日志里带前缀的行有 164 条，而 soak 的断言是 90
# 多条——像 `system: gone guest state=Dead ousted=true heir=18→17 wait=now` 这种**动态叙事**
# 每轮的号都不一样，判据钉的是"这一族在不在、形状对不对"，不是逐行。一刀切"逐行都要判"只会
# 造一台永远红的机器（红得没意义），故把"逐行"与"逐族"分开，且**逐族那一档必须写理由**。

BEGIN {
    FS = "\t"
    # **照实记（这一行是量出来的坑）**：awk 里**未初始化的变量当下标是空串，不是 "0"** ——
    # 第一版没写这两个 0，于是 `apat[0]` 从没被赋值 ⇒ 它取回空串 ⇒ `$0 ~ ""` **匹配一切**
    # ⇒ 整台对账器"全绿"（连手写一条谁都不匹配的行也判绿）。照实记在 `docs/harness-gate.md`。
    np = 0
    na = 0
    while ((getline ln < asserts) > 0) {
        if (ln == "") continue
        i = index(ln, "|")
        if (i == 0) continue
        kind = substr(ln, 1, i - 1)
        pat = substr(ln, i + 1)
        if (kind == "need_absent") { absent[na] = pat; na++; continue }
        apat[np] = pat
        np++
    }
    close(asserts)
    while ((getline ln < table) > 0) {
        if (ln ~ /^#/ || ln ~ /^[ \t]*$/) continue
        n = split(ln, f, "\t")
        pref = f[1]
        gsub(/^[ \t]+|[ \t]+$/, "", pref)
        lv[pref] = f[2]
        why[pref] = (n >= 3 ? f[3] : "")
        declared[pref] = 1
    }
    close(table)
    # 构建噪声（rustc 的话，不是程序的读数）——soak.sh 头注那条口径。
    noise["warning"] = 1
    noise["help"] = 1
    noise["error"] = 1
    noise["note"] = 1
}

{
    # 两种读数形状：`名字: …`（程序打的）与 `[case] …`（**用例协议**，`cases.rs` 那个运行器打的）。
    if ($0 ~ /^\[case\] /) {
        p = "[case]"
    } else if ($0 ~ /^[a-z][a-z0-9-]*: /) {
        p = $0
        sub(/: .*/, "", p)
    } else {
        next
    }
    if (p in noise) next
    seen[p]++
    if (!(p in declared)) {
        undeclared[p]++
        if (sample[p] == "") sample[p] = $0
        next
    }
    hit = 0
    for (i = 0; i < np; i++) {
        if ($0 ~ apat[i]) { hit = 1; break }
    }
    if (hit) {
        ok[p]++
    } else {
        bad[p]++
        if (bsample[p] == "") bsample[p] = $0
    }
}

END {
    if (report == 1) {
        printf "%-16s %-10s %5s %5s %5s  %s\n", "前缀", "档", "行", "判了", "没判", "例"
        for (p in seen) {
            printf "%-16s %-10s %5d %5d %5d  %s\n", p, (lv[p] == "" ? "?" : lv[p]), seen[p], ok[p] + 0, bad[p] + 0, (bad[p] ? bsample[p] : "")
        }
        for (p in undeclared) printf "%-16s %-10s %5d %5d %5d  %s\n", p, "**没声明**", undeclared[p], 0, undeclared[p], sample[p]
        exit 0
    }
    fail = 0
    for (p in undeclared) {
        printf "  前缀 `%s:` 没在 scripts/readings.txt 里声明（新读数没人管）—— 例：%s\n", p, sample[p]
        fail = 1
    }
    for (p in seen) {
        if (p in undeclared) continue
        if (lv[p] != "auto" && lv[p] != "narrative" && lv[p] != "manual") {
            printf "  前缀 `%s:` 的档位不是 auto / narrative / manual（现在是「%s」）\n", p, lv[p]
            fail = 1
            continue
        }
        if (lv[p] != "auto" && why[p] == "") {
            printf "  前缀 `%s:` 是 %s，但表里没写理由\n", p, lv[p]
            fail = 1
        }
        if (lv[p] == "auto" && bad[p] > 0) {
            printf "  前缀 `%s:` 有 %d 行没被判（auto 档要逐行）—— 例：%s\n", p, bad[p], bsample[p]
            fail = 1
        }
        if (lv[p] == "narrative" && ok[p] == 0) {
            printf "  前缀 `%s:` 声明为 narrative，但这一轮一行都没被判 —— 例：%s\n", p, bsample[p]
            fail = 1
        }
    }
    exit fail
}
