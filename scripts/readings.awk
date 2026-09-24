# 声明式读数对账（用户裁定"甲2"）—— **量具，不是门**：它把"哪条读数没人判"这件事
# 从 soak.sh 头注里那句"对账的法子是人工逐条对"变成机械的一步。
#
# 用法（soak.sh 每一轮末尾调它）：
#
#   awk -v asserts=<断言表> -v table=scripts/readings.txt -v report=0 -f scripts/readings.awk <日志>
#
#   `asserts`：**本门那批断言**，一行一条 `kind|pattern`
#              （由 soak.sh 从它自己的 `need` / `needE` / `need_absent` 里抽出来——不另抄一份）。
#   `table`：  读数表（`scripts/readings.txt`）：
#              `前缀 <TAB> auto|narrative|manual <TAB> 理由 [<TAB> 没判的那几行的形状]`
#   `report`： 1 = 只把统计打出来（标定时用），0 = 判红。
#
# 判据（四条）：
#   1) 日志里出现的每一个 `前缀:` 都要在表里（**新读数没人管** ⇒ 红）；
#   2) `auto` 档：这一前缀的**每一行**都要被某条断言匹配（新形状没人判 ⇒ 红）；
#   3) `narrative` 档：至少要有一条断言匹配 + 必须写理由 + **必须声明"没判的那几行长什么样"**
#      （第 4 栏那串形状）——本轮没判的行，只要有一行不在那串形状里 ⇒ 红。**这就是棘轮**：
#      叙事那一档也可以长，但得先写进表里才准长。
#   4) `manual` 档：判据不在断言表里（由别的机制判），只查理由在不在。
#
# **照实记（need 与 needE 的语义不同——这一格让整张表错了很久）**：soak.sh 里 `need()` 走
# `grep -q`（**BRE**：`(` `)` `{` `}` `+` `?` `|` 在那里都是**字面**），`needE()` 走 `grep -qE`
# （**ERE**：那几个是元字符）。第一版对账器把两者**都**当 awk 的 ERE 判 ⇒ 22 条带括号的 `need`
# 断言（`member: amid(me,c0)=true` / `policy: adopt(sub)=ok` …）在这里**永远不命中**（ERE 里
# `(sub)` 是"一个分组"，要的是 `policy: adoptsub=ok`）⇒ 27 行"其实判了"的读数被记成"没人判"：
# 表里写着 `member` 判 2/27、`policy` 判 2/15，真相是 **19/27** 与 **12/15**。修法：`need` 那一
# 族先按 BRE 把 ERE 元字符转义回本义（见 `bre2ere()`），`needE` 原样。**门一直是绿的**——因为
# grep 那一侧判对了；错的是这台量具，而它是定档位的依据。
#
# **照实记（行尾那个 `\r`）**：QEMU 串口回来的行尾带 CR，故 soak.sh 里的 `needE` 尾巴都得写
# `[[:space:]]*$` 才钉得住。对账器现在**统一把行尾 CR 去掉**再判，第 4 栏那些形状因此可以照常
# 写 `$`（不然每一条都得挂一个 `[[:space:]]*`）。
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
    # **同一个坑在本轮又露了一次**（这回在 `nore[]` 上）：白名单没写的那一档取回空串 ⇒ 判绿，
    # 故下面主循环里是 `nore[p] == "" || $0 !~ nore[p]`——**先判空串，再判正则**。
    np = 0
    na = 0
    while ((getline ln < asserts) > 0) {
        if (ln == "") continue
        i = index(ln, "|")
        if (i == 0) continue
        kind = substr(ln, 1, i - 1)
        pat = substr(ln, i + 1)
        if (kind == "need_absent") { absent[na] = pat; na++; continue }
        # `need`（`grep -q`，BRE）与 `need_absent`（`grep -q`，同）要转义；`needE` 原样。
        # （`need_absent` 只做登记：它真命中时，soak 那侧的 `missing` 早就让这一轮红了。）
        if (kind != "needE") pat = bre2ere(pat)
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
        nore[pref] = (n >= 4 ? f[4] : "")
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
    # QEMU 串口的行尾带 CR：先去掉，后面一律按"正常的一行"判。
    sub(/\r$/, "")
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
        # **棘轮**：`narrative` 档没判的行，必须落在表里声明的形状里。
        # （`auto` / `manual` 不数这一格：前者的口子在"没判"那一栏，后者的判据在断言表外面。）
        if (lv[p] == "narrative" && (nore[p] == "" || $0 !~ nore[p])) {
            stray[p]++
            if (ssample[p] == "") ssample[p] = $0
        }
        if (bsample[p] == "") bsample[p] = $0
    }
}

END {
    if (report == 1) {
        printf "%-16s %-10s %5s %5s %5s %5s  %s\n", "前缀", "档", "行", "判了", "没判", "出格", "例"
        for (p in seen) {
            printf "%-16s %-10s %5d %5d %5d %5d  %s\n", p, (lv[p] == "" ? "?" : lv[p]), seen[p], ok[p] + 0, bad[p] + 0, stray[p] + 0, (stray[p] ? ssample[p] : (bad[p] ? bsample[p] : ""))
        }
        for (p in undeclared) printf "%-16s %-10s %5d %5d %5d %5d  %s\n", p, "**没声明**", undeclared[p], 0, undeclared[p], undeclared[p], sample[p]
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
        if (lv[p] == "narrative") {
            if (ok[p] == 0) {
                printf "  前缀 `%s:` 声明为 narrative，但这一轮一行都没被判 —— 例：%s\n", p, bsample[p]
                fail = 1
            }
            if (nore[p] == "") {
                printf "  前缀 `%s:` 是 narrative，但表里没声明「没判的那几行长什么样」（第 4 栏那串形状）\n", p
                fail = 1
            } else if (stray[p] > 0) {
                printf "  前缀 `%s:` 有 %d 行既没人判、又不在声明的形状里（棘轮）—— 例：%s\n", p, stray[p], ssample[p]
                fail = 1
            }
        }
    }
    exit fail
}

# `need` 那一族（soak.sh 走 `grep -q`）是 **BRE**：ERE 里这几个是元字符，BRE 里是字面。
# 逐字符加一条反斜杠即可（gawk 实测 `\(` `\{` `\+` 都当字面）。
function bre2ere(s,   out, i, c) {
    out = ""
    for (i = 1; i <= length(s); i++) {
        c = substr(s, i, 1)
        if (c == "(" || c == ")" || c == "{" || c == "}" || c == "+" || c == "?" || c == "|") out = out "\\"
        out = out c
    }
    return out
}
