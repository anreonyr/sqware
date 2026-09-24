# 宿主那一门 —— **结构门**（测试与运行环境分开）

> **裁决（2026-09-24）**：
>
> - 走**丙**：八个编外 case crate 收成**一个** `crates/protocol-case`（八个测试靶，靶名 = 原
>   crate 名去后缀）。
> - 走**甲**：5 条"纯常量读数"用例交给编译器（`const _: () = assert!(…)`；`supply` 那条整条删）。
> - 外加一句（用户原话）：**"我希望测试和运行环境分开，而不是交叉在一起"** ⇒ 34 条住在
>   `#[cfg(test)]` 里的用例**搬出运行时源**；其中与台里**更强的同名判据**重复的那 14 条**删掉**。
>
> **落地读数**：用例 **137 → 118**；编外 crate **8 → 1**（八份 `Cargo.toml` + 八份
> `Cargo.lock` + 八个 `target/` ⇒ 各一份）；`env` 由"编八遍"变"编一遍"；`scripts/host.sh`
> 一轮 **0.08 s**。逐靶基线写在 `scripts/host.sh` 里——**它才是权威**（23 · 11 · 24 · 20 ·
> 11 · 12 · 10 · 7）；靶与源码的对应关系写在 `crates/protocol-case/Cargo.toml` 头注。

## 1 · 这一门原先长什么样（"太多"的三种读法，先量再判）

| 项 | 数 |
|---|---|
| 宿主用例 | **137**（八个 case 台里 103 + 埋在协议源码 `#[cfg(test)]` 里 34） |
| 断言 | **725**（每条用例 5.3 条） |
| 用例体 | 2403 行；连注释与脚手架共 3229 行 |
| 编外 case crate | **8 个**，各带一份 `Cargo.toml` + `Cargo.lock` + `target/` |
| 机器那一侧 | `soak.sh` 的 `need` 断言 + 两把量具 75 条变异（宿主 47 + 机器 28） |

**能证实的三条"多"**：

1. **5 条纯常量读数**：4 条"记号不相撞"（`line` / `operator` / `board` / 三条路的回信孔）+
   `supply` 那条长度表。全是常量比常量——`Mark::of` / `Mark::get` 都是 `const fn`，而
   `driver/supply/call.rs` 里**本来就有** `const _: () = assert!(WANT_LEN == 32)`。
2. **34 条埋在两处家**：只有"某个靶恰好 `#[path]` 编了那份源码"才跑得到；靶一改，它们会
   **静默消失**而门照样报绿（原先每台只查"≥ 1 例"，不查总数）。
3. **8 个 crate 的脚手架抄了八遍**：`#[path]` 头 + `serial()` + 假表，外加八份锁与八个
   `target/`。

**减不掉的那一条**：剩下 132 条是"一条契约一条"——`the_supply_failure_table_is_lossy_on_purpose…`
一条里就 13 条断言，量的是码表双射与两个 `None` 分不分得开。按本仓的规矩（**"没有门的档 =
没有编译过的档"**），划掉一条就是少一道判据。故这一刀的口径是：**只减机器，不减判据**——
去掉的那 19 条（14 重复 + 5 常量）都有更强或等价的落点。

## 2 · 34 条搬出来时，哪 14 条是**删**而不是搬

`operator/judge.rs` 那 8 条与 `operator/gate.rs` 那 6 条，和 `judge` 靶里那几条是**同一件事**，
而台里的版本**更强**（例：源码里那条只能证"没看到 `Unjudged`"，台里换成了**会记数**的谓词桩，
直接量"一次都没问"）。故删掉，只把**独有的三处**并进对应用例：

1. 三格码 ↔ **线上那一格**（`Code::Ok/Denied/Unjudged/Blind → WIRE_*`）⇒ 并进
   `the_verdict_maps_onto_the_wire_cells`；
2. "服务在、但这一问没答"那颗 `Control` 桩（四问全 `Err` ⇒ 判不了）⇒ 并进
   `an_unreachable_roster_is_unjudged_never_denied`；
3. `Blind` 对 `Under` / `Opens` 也答 `Blind`（不只是 `Public` 那一格）⇒ 并进
   `no_face_at_all_is_blind_and_never_allow`。

`principal` / `coalition` 那 13 条进 `roster` 靶的两个 `mod`（两份正文都有 `A` / `book()`，
同住一层会撞名，各自的 `use` 也就各归各的）；`board` 那 7 条进 `board` 靶（它那批假表与
助手整段跟着搬，只把 `core::sync::atomic` 改成 `std::sync::atomic`——靶里 `core` 那个名字被
板上正文的模块占了）。

## 3 · 照实记（三处取舍与两处踩到的坑）

- **编译红 ≠ 门红**：那 5 条常量挪到编译期之后，改动它们得到的是 `E0080`（编译红），而
  `scripts/teeth.py` 把"编译红"算作**量具无效**（红必须红在一条跑着的判据上）。这不是丢牙口：
  它们本来也不在量具那 47 条里；丢的是"红在哪一条用例"这个读数。实测：把 `board` 的
  `ASK_MARK` 改成 `Mark::of("operator-ask")` ⇒ `E0080: assertion failed`（已还原）。
- **一私有字段读数**：`principal` 那条"起点始终是当前那一格的祖先"原先直接读
  `Principal` 的私有字段（`b.roster[0].origin` / `.current`）——住源码里才够得着。搬出来之后
  改成**同一不变量的可观察读法**（同一个测试下面那两句：`waive` 回的是起点 `Some(p)`，而此刻
  `current` 已经是 `q`）。
- **量具的账没动**：`teeth.py` 的账键是 `名字|被改的源码文件|锚点指纹`，被改的**永远是协议源码**
  （路径与锚点这一刀都没碰）⇒ 47 + 28 条**一条都不用重跑**（复刻 `key_of` 逐条验过）。
- **踩到的坑一**：合并那一刀顺手把八份靶过了 workspace 的 rustfmt——它们原先编外，
  `cargo fmt --all` 从来没扫到它们，故 diff 里有一批**只是换行**。以后改靶要补一句
  `cargo fmt --manifest-path crates/protocol-case/Cargo.toml`（记在那个 crate 的头注里）。
- **踩到的坑二**：删用例时先写的那版脚本把 `#[test]` 属性留在了原地（切点算错一行），
  四个靶当场"`the test attribute may only be used on a free function`"——**编译红**，不是门红。
  已修，并在 `scripts/host.sh` 加了**逐靶基线**这一条：以后少跑一条也红。

## 4 · 下一刀若还想收

门槛变了：现在是"**一个 crate、八个靶**"，加一门只是加一个 `tests/<靶>.rs`（`Cargo.toml`
不用动），但**基线要跟着改**（那是刻意的：用例数变过就该在 diff 里被看见）。
`scripts/host.sh` 头注那张"还有哪些核没有门、为什么"的清单照旧有效——第 2 类（拖 `runtime`
的那几份 `*/client.rs` 与 `session/call.rs`）仍是唯一欠账，而它欠的不是"没门"，是"上了会重复
跑判据"。
