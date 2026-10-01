---
name: comment-cleanup
description: >
  注释清理：把源码里的历史叙述、重复叙述、自指叙述、结构耦合叙述一次性精简到
  "短、局部、稳定、不自指、不导航、不讲历史" 的注释。注释描述当前代码，不描述代码历史。
  本任务只处理注释；不重构、不动 API、不为注释新建抽象。
---

# 注释清理（Comment Normalization）

**何时用**：源码能跑、但注释大量是开发期残留——历史叙述（"那一刀"、"以前"、"照实记"）、自指（"本文件"、"上面"）、术语堆叠（"这一格/那一手/本域"）、跨模块引用导航（"见 `crate::xx::yy`"）、模块名作主语（"`system` is..."）。读者脱离 commit 历史就看不懂注释。

**何时不用**：

- 架构 / 结构 / 命名问题 —— 用 `design-pipeline`
- 已有能跑但"绕、理不清、组织不清" —— 用 `flash-refractor`
- 仅修复个别错别字、风格不一致 —— 直接改，不必走这个流程

## 0 · 核心原则

> **注释描述当前代码，不描述代码历史。**
> **不为了注释建立耦合。**
> **不写自指注释。**
> **不写源码结构说明。**
> **不记录代码放置理由。**

五条原则可压缩为一句：**代码表达结构，注释只补充代码无法直接表达的稳定语义。**

## 1 · 与其它 skill 的边界

| skill | 关心什么 | 不关心什么 |
|---|---|---|
| `comment-cleanup`（本） | 注释的字、链接、引用 | 代码结构 / API / 行为 |
| `flash-refractor` | 字段、调用关系、格数 | 注释内容 |
| `design-pipeline` | 操作、结构、原语 | 历史叙述 |

三者可独立使用。本 skill 与 `flash-refractor` 的边界：本 skill 只动注释、不动字段；`flash-refractor` 只动字段、不动注释历史叙述。

## 2 · 三轮递进（每轮只输出一个判据）

```
1 粗清       —— 删"照实记/那一刀/以前/今天..."这类历史叙述段
2 按规则重写 —— 模块名作主语、跨模块引用导航、ASCII 目录树
3 精修       —— 删孤儿注释 / 残句 / 半删表格 / 重复函数名的 ///
```

每轮结束都是用户裁决门。`#1` 容易过、用户几乎无需决策；`#2` 是核心，要列出每条规则的命中数；`#3` 是 detail polish，常需要 `git diff` 后逐个文件评审。

## 3 · 删除哪些注释（按规则清单）

### §A 历史叙述（删除）

以下词汇出现在 /// 或 //! 行内，整段（直到空行或非注释行）删除：

```
以前 / 原先 / 之前 / 后来 / 曾经 / 第一版 / 旧版
已经删掉 / 已经移除 / 回收 / 退场
这一刀 / 那一刀 / 照实记 / 从前 / 曾写反 / 换过 / 收掉
搬到 / 搬过去 / 搬过来 / 并进 / 并到 / 并入
那时候 / 那一阵 / 彼时 / 当时 / 旧孔时代 / 旧装配表 / 旧 line / 旧 echo / 旧 uart
改成 / 换成 / 用户裁定 / 用户裁决 / 用户拍板 / 项目裁定
证据链 / 负证 / 这次 / 本次 / 这次 / 本轮
逐份量 / 残枝 / 拆毒 / 回炉 / 拆掉
```

注意：**不能只删孤字**。"轮"或"刀"单独出现（如"无轮询"、"切刀"）是合法词汇——**只删复合短语**（"第 N 轮"、"这一刀"、"那一刀"等）。

### §B 自指注释（删除）

以下短语出现在 /// 或 //! 行内，整段删除：

```
本文件 / 本节 / 本模块 / 本段 / 本处 / 这里 / 此节 / 此段 / 此层
下面这一 / 上面那一 / 前面已经 / 接下来
```

跨多行的孤儿块（`///` 块的尾部出现 `:xxx` 残句、或 `——而` 断开孤行）也要整段删。

### §C 跨模块引用作为注释导航（改纯文字）

`[\`crate::foo::Bar\`](...)` 与 `[\`super::xxx\`](...)` 形式的引用，改为普通文字描述（"由 `foo` 域持有"、"组装期读"），不保留链接。判断标准：**删掉链接后注释仍然清楚？清楚 → 删。**

理由：源码结构改变时，注释不应牵连。

### §D ASCII 目录树 / 缩进伪树（删除）

`/// ```text` ... `/// ``` ` 块，含 ├ └ │ ┌ ┐ 字符或深缩进（`//!` 后接 2+ 空格），整段删除。例外：真正承载运行时契约（"操作码 9 = DENIED"那类字节序表）可保留——但这类通常不是树形，是表，且写在码旁。

### §E 时间性语言（改写或删）

```
今天 / 今天只 / 今天仍 / 今天还 / 当前这一步
本轮 / 上一轮 / 下一轮 / 上一版 / 下一版
```

若该句话确实是"今天为什么这么写"，改成"由历史原因，..."或不写。若只是"现状描述"，直接删。

### §F 人为术语堆叠（改写）

```
这一格 / 这一手 / 这一族 / 这一面 / 这一头 / 这一趟 / 这一段 / 这一支
那一格 / 那一手 / 那一族 / 那一面 / 那一头 / 那一段 / 那一支 / 那一端
本域 / 持有者那 / 装配者那 / 编排者这一侧 / 持有者那一侧
```

直接换成普通动词或具体名词：
- `本域那一格` → `这一台`
- `装配者这一侧` → `装配期`
- `持有者那一边` → `对方`

### §G 模块名作主语（重写）

`//! system — 编排域的实现侧：运行时装配上下文...`
→ 改为 `//! 运行时装配上下文（Assembly）及其组件。`

匹配模式（按出现概率排）：
1. `<module_path> — **topic**（paren）rest` —— 例如 `protocol — **代码表**:...`
2. `<module_path> — **topic**rest`
3. `<module_path> — rest`
4. `<module> 的那一半 —— rest` —— 例如 `control 的帧那一半 —— 帧、码、记号、状态`
5. `<module> 的<rest>` —— 例如 `protocol 的那一族`
6. `<module_path>: rest` / `<module_path>：rest`
7. `<module> is/provides/does rest`

写重写函数时，pattern 5（中文所有格 + 长破折号）最容易写错。要点：

- 用贪婪匹配（greedy）、让 pattern 自然停在 `（` 或 `；` 处
- 排除字符用 **Unicode literal**（`—`、`（`、`）`），不要用 `\xXX` 转义（Python 3.14 在 raw f-string 中 `\x` 行为不同）

```python
# 错误（Python 3.14 raw f-string 中 \xe2\x80\x94 被解释成 \xE + 2）：
pat = rf'^{NAME}\s*\xe2\x80\x94'
# 正确（直接用字面 Unicode）：
pat = rf'^{NAME}\s*—'
```

### §H 模块头目录树（删除）

```rust
//! ```text
//!   control/grant/  ...
//!   hub/             ...
//! ```
```

文件头说明文件结构的 ASCII 目录树。代码目录本身就是地图，不需要在注释里复制一份。整段删除。

### §I 跨模块引用残句（删除）

当 `[\`crate::foo::Bar\`](Bar)` 被规则 §C 删括号后，剩 `:xxx`、`:name`、`:join` 这种行首 `:` 加单词的残句——**整段 /// 块含此残句就删**。

判定：

```python
re.match(r'^:\w+', s)  # 行首冒号后跟单词
```

### §J 半删表格（删除）

```rust
/// | 变体 | 是什么 | 生产者 | 读者 |
/// |---|---|---|---|
```

表格内容已删但表头还在。删表头/分隔线，或整段删除。

### §K 重复函数名（删除）

```rust
/// Returns the task.
pub fn task(&self) -> TaskId
```

### §L 注释为什么存在（删除）

```rust
/// This comment is here because ...
/// Keep this comment ...
/// This is important ...
```

直接陈述事实，不解释写注释的动机。

## 4 · 保留哪些注释（§6 三类）

唯一允许稳定存在的注释：

1. **类型语义**——一句说明这个类型代表什么、保证什么。

   ```rust
   /// A handle referring to a resource in another address space.
   ```

2. **当前不变量**——代码本身很难直接表达、但实现必须长期满足的约束。

   ```rust
   /// The entry task must remain alive because its Pie is owned by
   /// descendants through the operator tree.
   ```

3. **非显然的行为契约**——ownership / borrowing / lifetime / permission / error meaning / state transition / ordering requirement / failure semantics / safety invariant。

   ```rust
   /// `POLL` performs a single non-blocking attempt.
   ```

## 5 · 与代码无关的注释风格

- **优先简洁直接的英语**（或中文短句）。不写绕口令。
- **保留当前不变量**的注释用短句、单句。
- **API 文档第一句保留**（"X 是 Y"）。
- **不写模块名作主语的句子**（如 `/// system is ...`）。
- **注释脱离整个文件后是否仍成立？** 不成立 → 删。
- **移动这个文件后注释是否仍成立？** 不成立 → 删。

## 6 · 实施 — 三轮脚本模板

### 第一轮脚本（粗清）

```python
import sys, re
from pathlib import Path

HISTORY = ('以前', '原先', ..., '残枝', '拆毒')

def starts_block_with(line, words):
    """line 起始 /// 之后是否含历史叙述关键词"""
    s = line.lstrip()
    m = re.match(r'^(///|//!|//|/\*)', s)
    if not m: return False
    body = s[m.end():].lstrip()
    return any(w in body for w in words)

def process(path):
    text = path.read_text(encoding='utf-8')
    lines = text.split('\n')
    out, removed = [], 0
    i = 0
    while i < len(lines):
        line = lines[i]
        # 1) ASCII 目录树（含 ├ └ 或深缩进）
        if line.lstrip().startswith(('//', '///', '//!')):
            # 检测 ```text 块 + 含树字符或深缩进
            ...
        # 2) 表格残句
        # 3) /// 或 //! 行内历史叙述段
        if starts_block_with(line, HISTORY):
            # 找到连续段末尾（空行或非注释行）
            ...
        out.append(line)
        i += 1
    ...
```

**坑**：

- 不要在历史词里放孤字（"轮"、"刀"），会被"无轮询"、"切刀"误命中。要用复合短语（"第 N 轮"、"这一刀"）。
- `HISTORY` 词表要按"完整字面出现"判定（`in` 子串匹配），而不是 regex，否则漏掉。
- 段落删除要"直到空行或非注释行"，否则会跨段误删。

### 第二轮脚本（按规则重写）

```python
NAME = r'[A-Za-z_][\w]*(?:::[A-Za-z_][\w]*)*'
NAME_PHRASE = rf'(?:{NAME}(?:\s+{NAME})*)'

MODULE_HEADER_PATTERNS = [
    # 1. `system — **topic**（paren）rest`
    rf'^{NAME_PHRASE}\s*—\s*\*\*[^*]+\*\*\s*[（(][^）)]*[）)]\s*[：:]?\s*',
    # 2. `system — **topic** rest`
    rf'^{NAME_PHRASE}\s*—\s*\*\*[^*]+\*\*\s*[：:]?\s*',
    # 3. `system — rest`
    rf'^{NAME_PHRASE}\s*—\s*[^*\n]+\s*[：:]?\s*',
    # 4. `system is/provides ... rest`
    rf'^{NAME_PHRASE}\s+(?:is|provides?|does|...)\s+',
    # 5. `system 的那一半 —— rest`（中文所有格 + 长破折号）
    rf'^{NAME_PHRASE}\s*有?的[^—\n]{{1,30}}?——\s*([^—（）\n]{{1,30}})',
    # 6. `system: rest`
    rf'^{NAME_PHRASE}\s*[：:]\s*',
]

def module_subject_rewrite(line):
    prefix = '//!' if line.lstrip().startswith('//!') else '///'
    body = line.lstrip()[len(prefix):].lstrip()
    for pat in MODULE_HEADER_PATTERNS:
        m = re.match(pat, body, re.IGNORECASE)
        if not m: continue
        groups = m.groups()
        # 有 capture group 用 capture；无则用整体替换
        if groups and groups[0] is not None and len(groups[0].strip()) >= 1:
            body = groups[0].strip()
            break
        new_body = body[m.end():].lstrip()
        if len(new_body.strip()) > 5:
            body = new_body
            break
    else:
        return line
    # 二次清理：开头（xxx）装饰、末尾孤立标点
    ...
```

**坑**：

- **Python 3.14 raw f-string 转义**：`\xe2\x80\x94` 在 raw 字符串里不会被 re 模块正确解析（除非 raw 字符串前无前缀冲突）。直接用字面 `—`、`（`、`）`。
- **贪婪 vs 非贪婪**：捕获组用 greedy + 字符排除（`[^—（）]`）。非贪婪会过早停在第一个字。
- **mod_subject_rewrite 后再判孤儿**：原行被改写后，新行可能成孤儿（`:xxx`）。需要在 process_file 里"如果新行是 orphan，按 subsystem 走"——具体实现：subj 重写后，**不 append**新行，让下一轮迭代到原行时检查（但原行也改了）——或直接在 module_subject_rewrite 后判断 new_line 是否 orphan、是则丢弃。

### 第三轮脚本（精修孤儿）

```python
def is_orphan_comment(line):
    """/// 块中含 `:xxx` 残句 / `——而...` 断开孤行 等。"""
    s = line.lstrip()
    # 不带 /// 前缀的孤立残句（cross-module link 删剩的）
    if re.match(r'^:\w+', s):
        return True
    if not (s.startswith('///') or s.startswith('//!')):
        return False
    body = s.lstrip('/').lstrip()
    if body.startswith('——而') or body.startswith('——（') or ...:
        return True
    if body.startswith('——'):
        return True
    if body.startswith(':') and re.match(r'^:\w+', body):
        return True
    ...

def is_orphan_block(lines, start):
    """检测 /// 或 //! 注释块：若块内任一行是 orphan，整段删。
       块定义：连续 /// 行 + 中间的 `:xxx` 残句行。"""
    j = start
    has_orphan = False
    while j < len(lines):
        s = lines[j].lstrip()
        if s.startswith('///') or s.startswith('//!'):
            if is_orphan_comment(lines[j]):
                has_orphan = True
            j += 1
        elif re.match(r'^:\w+', s):
            # :xxx 残句行 —— 视作块的一部分
            has_orphan = True
            j += 1
        else:
            break
    return (start + j) if has_orphan else -1
```

**坑**：

- **正则在 raw f-string 中要避免 `\xe2`**——直接用字面 Unicode。
- **贪婪 vs 非贪婪**：检测 `:xxx` 时用 `re.match(r'^:\w+', s)`，不要 `^:\w+$`（会把尾部空格排除掉）。
- **孤儿块的起点不只是 `///`**： `:xxx` 行（无 `///` 前缀）也属于块。`is_orphan_block` 的循环必须检查两种。
- **`module_subject_rewrite` 后再判孤儿**：改写后的行可能成孤儿，必须让 process_file 在改写后再走一次 orphan 流程。

## 7 · 收工判据（逐条勾）

1. 每个 /// 都独立成段，没有残句或半删的表格。
2. 每个模块头 1–3 行，不出现 `system — ...`、`driver is ...`、`control 的...`。
3. 每个跨模块引用都改普通文字或不存在。
4. 没有 `今天/以前/那一刀/照实记` 等历史叙述词。
5. 没有 `本文件/本节/下面那一/本域` 等自指或术语堆叠。
6. 删光全部注释后，代码仍能读懂（§6 三类仍在）。
7. **净行数减少 ≥ 5%**（数据：每轮可减 8–15%）。
8. 注释 / 代码比 ≤ 0.35（粗清目标 0.4，精修目标 0.25）。
10. `cargo fmt --check` 与 `cargo check --workspace` 均绿。

## 8 · 禁止事项

绝对不要做：

- 改 API、改模块结构、改类型关系、为新成员添加 `pub`、为注释新建 `use` / `re-export`、移动代码
- 把历史注释"压缩后保留"
- 把开发日志换措辞继续写进源码
- 顺手做无关 refactor

**diff 应表现为：注释减少 + 链接减少 + 模块头重写。代码几乎不动。**

## 9 · 一句话交底

注释清理的成功不是"代码更漂亮"，是**"同一件事用更少的注释说得出来，并且量得出来"**。

## 10 · 经验值（sqware 三轮实测数据）

| 轮次 | 总行 | 注释 / 代码比 | 改动行 | 累计 −行 | 主要动作 |
|---|---|---|---|---|---|
| 基线 | 24 483 | 1.18 | – | – | – |
| 第一轮 | 16 235 | 0.30 | −8 542 | −8 248 | 粗清：删"那一刀/照实记"等整段 |
| 第二轮 | 18 436 | 0.30 | −2 942 | −6 047 | 按 §1-§17 重写模块头、改链接 |
| 第三轮 | 18 366 | 0.27 | −1 771 | −6 117 | 精修：删孤儿、残句、模块名作主语 |

三轮后约 −25% 总行数、−63% 注释行数。每一轮的判据：

- **第一轮**：纯"按词表删除段落"，最少用户裁决。
- **第二轮**：模块头重写 + 跨模块链接改普通文字，需用户对每条规则命中数裁决。
- **第三轮**：detail polish，几乎全自动化，但每改一处要跑 `cargo check`。

常见坑：

1. **f-string 中 `\x` 转义**：Python 3.14 raw string 行为变化。用字面 Unicode。
2. **历史词"轮"、"刀"孤字误命中**：要"第 N 轮"、"这一刀"复合短语。
3. **`module_subject_rewrite` 后行变孤儿**：必须在改写后立刻判孤儿，不能只在原始行上判。
4. **ASCII 目录树 `text` 块未闭合**：超过 30 行的未闭合块视为病态，截断。
5. **跨模块引用被删后剩 `:xxx` 残句**：必须把 `:xxx` 整段作为孤儿处理。
6. **`step2` 改写脚本会把代码行（无 `///`）当作注释行处理**：先判 `startswith('//')`，再处理。