# `supply` 的类型搬家 —— **结构门**

> **裁决（2026-09-24）**：走**甲**（`Access` / `Policy` 挪进 `env`）；帧靶**新开
> `protocol-case` 的 `supply` 靶**；`env` 里**新开 `crates/env/src/wire/access.rs`**。
>
> **落地读数**：
> - ✅ 类型搬家：`env/src/wire/access.rs`（两个类型 + 两个掩码 + 全部 `impl`，133 行）；
>   `runtime/src/core/port.rs` 少 128 行、改成 `pub use env::{Access, Policy};` ⇒
>   **22 个调用点一行未改**（`cargo check --workspace --all-targets` 0 error）
> - ✅ `driver/supply/call.rs` 那一行 `use` 改成 `env` ⇒ **这一份本来就全纯** ⇒
>   **不用拆 `frame.rs` + `call.rs`**（与 `line` 那一份同形：零切分，直接编进宿主靶）
> - ✅ `protocol-case` 的 `supply` 靶：7 条判据（本章 §4 那份提案照做）
> - **照实记**：写判据时我猜 `class_block` 对超长类名答 `Err(TooLong)`——**实测是截断到
>   `NAME_LEN - 1`**（当场红）。这一格现在钉住"截断"这个口径，并顺手钉住它的下场：
>   **两个只有尾巴不同的长类名会撞成同一格**（今天没有这么长的类名，故只记口径、不改结构）。
>
> 下面保留**裁决时**的原样（读数与分叉）。

> 上一道帧门的收口里写着"`driver/supply/call.rs` **切不动**：`Access` / `Policy` 长在
> `Want` / `Need` 的类型里 ⇒ **类型搬家单独立门**"。这一份就是那道门。
> 按 `design-pipeline` 的门纪律：结构没裁定之前不动源码。下面每一格都带**量出来的读数**。

## 1 · 卡在哪

`crates/protocol/src/driver/supply/call.rs`（350 行、32 项）里，**碰 `runtime` 的只有一行**：

```rust
use runtime::core::port::{Access, Policy};      // ← 第 7 行，全文仅此一处
```

而这两个类型**长在荷载的类型里**，不是可有可无的引子：

```text
  Want::new(key, kind, access: Access, policy: Policy)
  Want::access() -> Option<Access>      Want::policy() -> Option<Policy>
  Need::class(…, access: Access, policy: Policy)      Need::known(同上)
```

⇒ 供单/需求单的**每一格**都带着它们。宿主靶不依赖 `runtime`（它拖着那两处 riscv 内联汇编），
故这一份上不了宿主：**帧那一半**（`OP_SUPPLY` / `WANT_LEN` / `ORDER_CAP` / 编解 / 失败码表）
至今只有机器在跑。

## 2 · 这两个类型是什么（搬家的可行性）

`crates/runtime/src/core/port.rs` 共 286 行；`Access` 与 `Policy` 是**纯位视图**，约 130 行：

```text
  Access(Permission)   读写族两位   FETCH | STORE      ACCESS_MASK
  Policy(Permission)   传递族两位   VEST  | ONLY       POLICY_MASK
  impl BitOr / BitAnd / Not / bits / from_bits（只收本族位，混族与未知位一律拒）
```

它们的依赖**只有 `env::Permission`**（`env` 的底板类型）与 `core::ops` ⇒ **零 `runtime` 依赖**。
`port.rs` 里剩下的（`To` / `Port` / `ship` / `denied`）才是真正碰内核的那一半。

**为什么现在住在 `runtime`**：`ship`（授出那一手）是 `runtime` 的动作，它收这两个类型。
**为什么放 `env` 说得通**：`env` 里已经有 `Permission`（同一套位），而这两个类型就是它
"四位分两族"的那套视图；且 `protocol` **本来就允许依赖 `env`**、**不允许依赖 `runtime`**
——这正是"上不了宿主"的成因。

## 3 · 四条路

| 路 | 做法 | 代价 | 调用点 |
|---|---|---|---|
| **甲** | `Access` / `Policy`（含两个 `*_MASK` 与全部 `impl`）挪进 `env`；`runtime::core::port` 里 `pub use env::{Access, Policy};` | `env` 多两个类型（与 `Permission` 同族）；`port.rs` 少 130 行 | **一行不改**（22 个文件都走 `runtime::core::port::…`，转出照旧）；只有 `supply/call.rs` 那一行 `use` 改成 `use env::{Access, Policy};` |
| 乙 | 不动类型：`supply` 的帧那一半改用**裸 `u8` 位**（`access: u8` / `policy: u8`） | 线上那一层失去**类型义务**（"混族不可表达"掉在报文层）——本仓明确嫌过这种退化 | 帧与类型两可之间多一层转换 |
| 丙 | 宿主靶里给 `runtime::core::port` 打一个**类型桩**（自己写一遍位布局） | **两处编**：门测的是桩，不是真类型——而这一处的语义**恰恰就是位布局** | 零（但门是假的） |
| 丁 | 不做，`supply` 的帧继续只有机器管着 | 清单里多一格永远"上不了宿主"的欠账 | 零 |

**推荐甲**：它与已经用过两次的那条路同款（宏独立成一份源、帧拆 `frame.rs` + `call.rs`，
两次都是"**一处编、调用点零改**"）；且搬的是**已经与 `env::Permission` 同层**的两个位视图，
不是把内核概念往底板里塞。

## 4 · 甲之后紧跟的一步（同一刀做完）

`supply/call.rs` 挪完就**全纯**（32 项，碰 `runtime` 的那一行没了），照帧门那套拆：

```text
  driver/supply/frame.rs   纯：码（OP_SUPPLY / Kind / Want / Need / 帧长 / 上限）+ 编解 + 失败码表
  driver/supply/call.rs    适配：剩下那点（若有）
```

**帧靶落在哪**：`driver/supply/core.rs` 只有类型（五格失败域）、**没有任何 `#[cfg(test)]`**
⇒ 编进哪一台都不会"重复跑判据"。三个选项：

```text
  ① 新开 `protocol-case` 的 `supply` 靶（名字最诚实：它钉的就是 supply 那一门）
  ② 并进 `protocol-case` 的 `line` 靶（都是 driver 那一层的协议；但那一台的名字会名不副实）
  ③ 并进 `protocol-case` 的 `judgement` 靶（那一台钉的是编排域的账/判定/配给——supply 是另一件事）
```

**推荐 ①**：与 `line` 靶 / `board` 靶同一条命名法（一台钉一门），代价只是多一个
`Cargo.toml` + 一个测试靶。

## 5 · 悬而未决（要你拍板）

1. **走甲 / 乙 / 丙 / 丁哪一条**（我推荐**甲**）。
2. **帧靶落在哪一台**（我推荐**新开 `protocol-case` 的 `supply` 靶**）。
3. `env` 里那两个类型的**住处**：与 `Permission` 同一个文件，还是新开
   `crates/env/src/wire/access.rs`？（我推荐**新开一个文件**：`permission.rs` 已经不小，
   而"两族视图"是另一件事——名字就叫 `access.rs`，两个类型都在里面。）

## 6 · 我推荐的顺序（认了就这么走）

```text
  一、搬类型：env 新开 access.rs（两个类型 + 两个掩码 + 全部 impl）；
      runtime::core::port 改成 pub use env::{Access, Policy};（22 个文件的调用点不动）
  二、改那一行 use：protocol/src/driver/supply/call.rs
  三、拆 frame.rs + 新开 `protocol-case` 的 `supply` 靶：往返 + 三种"不成形" + 帧长/上限 + 失败码表两端
  四、门：host（+一条新台）+ 机器那八道全跑（搬的是程序侧编进去的源码） + 牙口变异数条
```
