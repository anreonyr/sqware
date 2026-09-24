# 帧上宿主 —— **结构门**

> **裁决（2026-09-24）**：宏走**甲**；先做**零风险三份**（`line` / `principal` / `coalition`）；
> `supply` 那两个类型**先不挪，单独立门**。
>
> **落地情况**：
> - ✅ 甲：`fail_codes!` 搬进 `crates/protocol/src/fail_codes.rs`（`#[macro_use]` + `#[macro_export]`
>   两样都要 ⇒ **调用点一行未改**；各宿主靶 `#[macro_use] #[path] mod fail_codes;` 写在帧模块之前）
> - ✅ `line`：**零切**（那一份本来就全纯）⇒ 编进 `line-case`，加 5 条帧判据
> - ✅ `principal` / `coalition`：各拆成 `frame.rs`（纯）+ `call.rs`（适配，首行 `pub use super::frame::*;`）
>   ⇒ 两片编进 `principal-case`，加 8 条帧判据
> - ☐ `operator` / `board`：**待下一刀**（要真切：`frame.rs` + `call.rs`）；门已裁定，做法照上面两份
> - ☐ `supply`：明写"切不动"（`Access`/`Policy` 长在 `Want`/`Need` 的类型里）；类型搬家单独立门
> - ➖ `session`：**没有帧**（它本身就是运行时那一层）
>
> 下面保留**裁决时**的原样（读数与分叉），不改写成今天的数字。

> 这一份是**门**，不是实现：按 `design-pipeline` 的门纪律，"纯核心与适配分离"这一步的结构
> 没裁定之前不动源码。下面每一格都带**量出来的读数**，好让判断在你自己的推理里出现。

## 1 · 为什么要动它

五份 `*/call.rs` 里的**帧形**（`pack_*` / `unpack_*` / 读答话那几只）今天**只有机器在跑**。
而机器那几道门走的是**顺路**：客户端编一帧、持有者解一帧，形状对了就继续。下面这些格子
机器**一格都走不到**：

```text
  短一帧 / 长一帧 / 动作码不对 / 判别号不认识的坐标
  失败码表的两端（表外那一格、读不懂的码——两个 None 不是同一件事）
  记号与名字相不相撞（"面不相撞"那条纪律）
```

上一轮那张清单把这一类记成"上不了宿主"。这一轮**逐项量过**，结论比清单细：**三份不用切结构、
一份切不动、两份要真切**——所以要先定"切法"。

## 2 · 量出来的底账

量法：把每份 `call.rs` 的条目逐个切出来，看**它的正文**有没有提到 `runtime` / `session::call` /
`mail` / `port`（文件级 `use` 另算，故 `supply` 那一行要人工核）：

| 文件 | 纯项 | 正文里碰运行时的 | 能不能直接上宿主 |
|---|---:|---:|---|
| `driver/line/call.rs` | 13 | **0** | ✅ **本来就全纯**（只 `use env` + 同层 `core::Fail`） |
| `driver/supply/call.rs` | 32 | 0（**但文件级 `use runtime::core::port::{Access, Policy}`**） | ❌ `Access`/`Policy` **长在 `Want`/`Need` 的类型里**（`new`/`access`/`policy`/`class`/`known` 都带它们） |
| `principal/call.rs` | 26 | 1 | ⚠️ 只差**挪走一行** `pub use crate::session::call::opened_by;` |
| `coalition/call.rs` | 30 | 1 | ⚠️ 同上（一行） |
| `system/board/call.rs` | 23 | 5 | ⚠️ 要切（三个别名 + `board()` + `ship`） |
| `operator/call.rs` | 62 | 6 | ⚠️ 要切（`tree()` / `ship` / 三个别名 / `map_*`） |
| `session/call.rs` | 1 | 12 | ➖ **没有帧**（它本身就是运行时那一层，只有一句 `UNSEAT`） |

## 3 · 撞到的那一格（这一刀真正卡住的地方）

`fail_codes!` 是 `protocol` 的 `#[macro_export]` 宏，而**每一份 `call.rs` 正文里都有一次调用**
（那张"失败域 ↔ 线上那一格"的表）。宿主靶**不依赖 `protocol`**（拖 `runtime`，编不过）
⇒ **任何**包含 `call.rs` 的靶今天都编不出来——连"本来就全纯"的 `line` 那一份也编不出来。

四条路：

| 路 | 做法 | 代价 |
|---|---|---|
| **甲** | 宏挪成**自己一份源**（`crates/protocol/src/fail_codes.rs`），`#[macro_export]` 留着不动，协议与各宿主靶**同读这一份** | 结构改动：`lib.rs` 加一行 `mod`；**调用点一行不用改**（`#[macro_export]` 的效果不变）；各靶加一行 `#[path] mod`（顺序在帧模块之前） |
| 乙 | 宿主靶里**抄一份宏**（约 30 行） | 零结构改动，但**两处编**（本仓明确嫌过：线上码表已经吃过一次"三处各抄一份"的亏） |
| 丙 | 那张表**留在运行时那半**（帧上宿主、表不上） | 零结构改动；但那格判据仍然只有机器管着——**而那正是吃过亏的一格** |
| 丁 | 整份 `call.rs` 上宿主 | 编不过（拖 `runtime`） |

**推荐甲**：一处编、调用点零改、且"表的判据"跟着帧一起上宿主。

## 4 · 决策点（要你拍板的三个）

### 4.1 切法：`frame.rs` + `call.rs`

```text
  <协议>/frame.rs   纯：码（含线上那几格）、帧形、pack/unpack、读答话、失败码表
  <协议>/call.rs    适配：内核那几只手的别名、tree()/board()、ship、map_*（会话失败域 → 本域）
                   第一行 pub use super::frame::*;   ⇒ 调用点（各 client / server）一处不用改
```

判据：`protocol` 里的路径 `operator::call::Ask` / `board::call::LOOKUP` … **照旧**（glob 转出）。

### 4.2 帧的门落在哪一台（**不新开台**）

| 帧那一份 | 并进哪一台 | 为什么是它 |
|---|---|---|
| `driver/line/call.rs` | `line-case` | 它要 `env` + `line::core::Fail`，两者都在那台 |
| `principal/call.rs` | `principal-case` | 那台已编 `principal/core.rs`（要挪走一行 `opened_by`） |
| `coalition/call.rs` | `principal-case` | 同上（两本册子同住一台那条理由照旧） |
| `operator/call.rs` | `judge-case` | 那台已编 `core` + `judge` + `gate` + `ledger`——**帧要的依赖一个不缺** |
| `system/board/call.rs` | `board-case` | 那台已编 `board/core.rs` |
| `driver/supply/call.rs` | —— | **切不动**（见 §2） |
| `session/call.rs` | —— | **没有帧** |

**为什么不新开一台 `frame-case`**：那台要把各协议的 `core.rs` 也编进去（帧认得 `EntryId` /
`Rule` / `Board` …），于是那些 `#[cfg(test)]` 判据会在**第二台里再跑一遍**——本仓明说过不这么干
（`judge-case` 头注：*"反过来把 `judge.rs` 引进 `operator-case` 会把上面这些再跑一遍"*）。

### 4.3 宏那一条（§3 的甲乙丙丁）

## 5 · 被否的选项（记下来，免得下一轮重走）

- **整份 `call.rs` 上宿主**：拖 `runtime`（那两处 riscv 内联汇编在宿主编译器上编不出来）。
- **新开 `frame-case`**：重复跑 `core.rs` 那批判据（理由见 §4.2）。
- **宿主靶里抄宏**（乙）：两处编。
- **只上帧、不上表**（丙）：把吃过亏的那一格留在门外面。
- **顺带给 `supply` 打 runtime 的桩**：桩要造 `Access`/`Policy` 两个**真类型**的行为，
  那是"用桩假造内核语义"——桩量得了账，量不了语义（`line-case` 头注那条口径）。

## 6 · 悬而未决

1. **现在做几份**：全做（含 `operator` / `board` 两份真切的），还是先做**零切的 `line`** 与
   **只挪一行的 `principal` / `coalition`**（三份，风险几乎为零，能把"帧上宿主"的样子跑通）。
2. **宏走甲乙丙丁哪一条**（我推荐甲）。
3. **`supply` 那两个类型**（`Access` / `Policy`）要不要顺带从 `runtime` 挪到 `env`？
   那是更大的结构改动（内核与驱动两侧都碰），可以单独立门。
4. **帧的判据钉多细**（我的提案，四组）：往返一致；三种"不成形"（短 / 长 / 动作码不对）；
   表的两端（每格映射 + 表外码 + 读不懂的码）；记号不相撞。

## 7 · 我推荐的顺序（认了就这么走）

```text
  一、甲（宏独立成一份源，调用点零改）—— 一次结构改动，五份都受益
  二、line（零切）→ 帧靶的第一台：往返 + 三种不成形 + 表的两端 + 记号
  三、principal / coalition（各挪一行）→ 同样的四组判据
  四、operator / board（真切：frame.rs + call.rs）→ 同上
  五、supply / session —— 明写"不上宿主"的理由（前者切不动、后者没有帧），清单到此闭合
```

第 3 条（`supply` 的类型搬家）与第 4 条（判据的细度）**这一门里只需表态"先不做 / 照提案做"**。
