# pie — 权柄模型（Pie · 权限代数 · 派生 · 级联）

> 路径约定：`文件:行` 相对 `kernel/src/`。它管的资源实体在 [mail.md](mail.md)，
> 判权发生在 [switcher.md](switcher.md) 的 envcall 入口。

## 1 · 语义定位

`work/unit/gate` 是**能力模型层**：任务持有什么资源、各什么权限、能否转授/收窄/收回
（`work/unit/gate/mod.rs:1`）。内核在这里管三件事：**授权判定**（`Need`/`allows`/`covers`）、
**派生边**（`sire`）、**级联撤销**（`cull`）。

| 管 | 不管 |
|---|---|
| 权限位、派生关系、级联、寿命的唯一强引用 | 资源实体与 IPC 数据面（在 `mail`）、策略与命名（在协议） |

**gate → mail 是单向边**（`gate/mod.rs:6`）：门闩持资源实体的 `Arc`，资源不知道门闩存在。
这条边让「资源寿命 ＝ 能力寿命」成立——没有全局资源表，最后一份门闩消失即回收。

## 2 · 结构

| 文件 | 职责 |
|---|---|
| `gate/pie.rs` | `Pie<M>` / `AnyPie` / `Need` / `allows` / `covers` / `GateError` / `new_pie`（token 自 1 递增，`pie.rs:41-44`） |
| `gate/snap.rs` | 全世界快照 + `vestor` / `heirs` / `vestable` / `find` |
| `gate/accord.rs` | 转授子集——**唯一写派生边的地方**（`accord.rs:34-44`） |
| `gate/narrow.rs` | 就地单调收窄（sire / token / meta 都不动） |
| `gate/cull.rs` | 级联撤销 `cull` + 退出钩子 `doom` |
| `gate/right.rs` | **存在权**判定（`Nole` 载体，`right.rs:22-33`） |
| `gate/revoke.rs` | 收回授出的一棵子树 |
| `gate/release.rs` | 自释自己持有的一份 |

泛型与资源一一对应：`Pie<HoleMeta>` / `Pie<PoleMeta>` / `Pie<NoleMeta>`；`AnyPie` 的三个
variant 就是运行时 tag，故不需要 marker trait 或 `PieKind`（`pie.rs:1-5,100-108`）。
`Permission` 的单一真相在 `crates/env/src/permission.rs:16-28`，本层 re-export。
`PieToken` 是 `usize` newtype，`0` ＝ 无效哨兵（`crates/env/src/wire/handle.rs:14-17`）。
`Pie.sire: Option<usize>` 存的是**父门闩的 token**，不是 task id（`pie.rs:50-52`）。

## 3 · 权限代数

四位（`permission.rs:16-28`）：`READ`（观察/接收/重读）、`WRITE`（修改/投递）、
`VEST`（复制给其他 Task，自身权限不变）、`BACK`（只能 Vest 回 grantor）。

**BACK 压制 VEST 的自由性**：只要带 BACK，目标恒被守门为 `target == vestor`——即便同时带
VEST 也取最严（`permission.rs:23-27`、`snap.rs:95-102`）。

| 动作 | 动谁的什么 | 规则 |
|---|---|---|
| `Accord` | **别人的表**：克隆 `Arc<Meta>` 造新 token，`sire = Some(src.token())` | 子集必须非空且 ⊆ 自身（`pie.rs:93-95`）；src 权限不变 |
| `Narrow` | **自己那份的位**：就地改写、单调 | 空子集一律 `Denied`——「清空」的语义归 `Release`（`narrow.rs:23`） |
| `Revoke` | **我授出的那棵子树** | 鉴权 ＝「这枚的 `sire` 在我表里」（pie 只能复制不能转移，故等价于"我授出的"，`revoke.rs:39-46`） |
| `Release` | **我持有的那枚及其全部后代** | 不需要任何权限位（`release.rs:1-7`） |

## 4 · 寿命与所有权

- **强引用只有一处**：`Pie.meta: Arc<M>`（`pie.rs:54-58`）；`Task.pies: SpinLock<Vec<AnyPie>>`
  是唯一持有者（`work/unit/task.rs:143-147`）。没有全局资源表。
- **`Seal` 只置死 + 唤醒等待者，不摘表项**（`envcall/pie.rs:209-212`、`mail/hole.rs:269-272`；
  Nole 无等待者故只置死，`mail/nole.rs:74-79`）：内存由引用归零回收，且持有者仍须 `Release`
  收尾——若 `Release` 也判存活，封印后的表项就永远摘不掉（`envcall/pie.rs:369-370`）。
- **门闩必须在锁外 drop**：最后一份 drop 会跑 `Meta::drop`（唤醒 / 撤映射 / 还帧，都在 L3
  或更外层），锁内 drop 就是自嵌套（`pie.rs:56-57`、`cull.rs:24,55,70`）。
- **两个身份不可混用**：`vestor` 是**门闩**的来历（`sire` 的持有者，转手即改写）；`owner` 是
  **资源**的来历（任意副本共享同一事实，转手不丢）。`Seal` 只归 `owner`（O(1)，
  `envcall/pie.rs:224-226`）——目录协议正是靠 `owner` 认服务的（见 [dispatch.md](dispatch.md) §5）。

## 5 · 派生与级联

`sire` 是**单向边、构造期定型、无 setter**；「谁授给我的」与「我授出了哪些」都不另存，
而是同一张快照上的查询（`pie.rs:12-13`、`snap.rs:2-6`）。快照 ＝ 全世界任务的 `Weak<Task>`
列表，由适配层拍、boot 注入（`scheduler/core/table.rs:129-132`、`boot.rs:161-163`）；
死条目升级失败即跳过，故查询无副作用、无需清理（`snap.rs:9-10`）。

`cull` 的次序是硬约束：摘根 → BFS 沿 `sire ∈ frontier` 逐层反查 → **先把 Pole 强引用取出、
后 drop 门闩** → 全部摘完后**无锁**撤映射（`cull.rs:25-39,46-87`）。三处共用它：
`revoke` / `release` / `doom`（`cull.rs:4`）。

**两条级联，两个方向，都挂在退出钩子表里**：

```text
messenger::doom   沿 heir  → 扑杀整棵血缘子树（结构面，见 task.md §6）
gate::doom        沿 sire  → 反查我授出的全部能力（权柄面）
```

`gate::doom` 自己取快照按 id 找任务（此刻 `reap` 仍持强引用），**不查调度器**
（`cull.rs:89-105`）——依赖倒置保持 gate 不依赖 room。

## 6 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 资源寿命 ＝ 能力寿命（无全局资源表） | 泄漏或提前回收 | `pie.rs:54-58`、`mail/hole.rs:259-262` |
| 门闩在锁外 drop | 锁内跑 `Meta::drop` ⇒ L3 自嵌套 | `cull.rs:24,55`、`envcall/pie.rs:78-79` |
| 派生只存一条边 | 两处状态不一致（子在而父亡） | `snap.rs:2-6`、`cull.rs:6` |
| 权限单调（非空 ∧ ⊆） | 提权 | `pie.rs:93-95`、`narrow.rs:23-26` |
| 父在则子在（摘一枚必连同子树） | `sire` 悬空，需要 `RevokeFirst` | `release.rs:6-7` |
| BACK 压制 VEST | 委托权越过授与人意向外流 | `permission.rs:23-27`、`snap.rs:95-102` |
| 快照只出 `Weak`，查询不增删 | 死任务挡查询、需要清理 | `snap.rs:22-23`、`table.rs:103-106` |

## 7 · 裁决账

| 裁决 | 定论 | 理由要点 |
|---|---|---|
| 关系只存一条边 | 只存 `sire` | 一条关系只存一次，另两向是查询（`pie.rs:12-13`） |
| 存在权用**类型**而非 `Permission` 的一位 | `Nole` 承载 | 位与资源同轴，做成位会让任何资源顺带携带它，「这是不是那枚」再也答不出来——**类型是身份，位不是**（`right.rs:6-13`、`mail/nole.rs:21-26`） |
| `Revoke` 鉴权不看 `vestor` | ＝「`sire` 在我表里」 | 与 `Spawn` 同一句话；本地可判、不吃快照（`revoke.rs:3-5`） |
| 不做 seL4 的 `RevokeFirst` | 摘一枚必连同子树 | 「父在则子在」换来 `sire` 永不悬空（`release.rs:6-7`） |
| 空子集不是一次 `Narrow` | `subset` 必须非空 | 清空的语义归 `Release`（`narrow.rs:23`） |
| 级联归 `cull` | 三处共用 | 与 `messenger::cull` 同构同名，那边沿 `heir`、这边沿 `sire` |
| `UnsealNole` 铸币权收在 S 态 | 非 S 态 → `Denied` | 否则任何 U 域自铸一枚就自授建域权（`envcall/pie.rs:129-135`） |
| 快照由适配层拍、boot 注入 | gate 不依赖 scheduler | 依赖倒置（`snap.rs:26-39`） |

## 8 · 已知边界

1. **注释与代码不一致**：`gate/mod.rs:12` 写 `Need::{Read,Write,Grant,Build}`，实际枚举只有
   三个（`pie.rs:31-38`）——建域权**不走 `allows`**，走「自己表里有枚活着的 `Nole`」+ S 态
   兜底（`right.rs:23-28`、`envcall.rs:387-397`）。
2. **文档过期**：`pie.rs:192` 说 `GateError::Dead` ＝「已 seal **或 Weak upgrade 失败**」，
   但两处 upgrade 失败都返 `Denied`（`accord.rs:33`、`revoke.rs:29`）；全仓 `Dead` 只由封印产生。
3. ~~**计数写错**~~ —— **已修（本轮）**（改称十二个操作）。原记录：`envcall/pie.rs:1,8` 说「十一个操作」，`PieCall` 现有 **12** 个 variant
   （`crates/env/src/fid.rs:288-356`）。
4. **「判定顺序只有一处」不成立**（注释已于本轮如实改写，**统一仍是待办**）：只有
   `open`/`shut`/`accord`/`reserve` 走 `resolve`；
   `seal`/`narrow`/`collect` 仍手写查找，`Release` 完全不走 `find`。**`narrow` 的顺序与
   `resolve` 相反**（`covers` 在 `alive` 之前）⇒「已封印 + 越权子集」报 `Denied` 而不是
   `Dead`——正是那条注释声称已消灭的现象。数据轴 `push`/`pull`/`wait` 同样是「先判权、
   后判存活」。
5. **「`Release` 是唯一不过存活闸的操作」与代码不符**：`Revoke` 与 `Collect` 同样一次都没读
   `alive`；而 `Reserve` 虽不经 `resolve`，却经 `AnyPie::owner`（以 `alive` 为闸）**会**报
   `Dead`。该说法只成立于「唯一**必须**如此的」——`Seal` 之后仍须能摘表项的只有它。
6. **撤 `BACK` 会使 `VEST` 摆脱守门**：`vestable` 只查当下是否含 BACK（`snap.rs:96-98`），
   而 `Narrow` 可以把 BACK 位摘掉（`narrow.rs:23-26`）⇒ BACK 是唯一「位越少自由度越大」的位。
7. **级联闭包 ＝ 快照上的闭包**：`cull` 逐层用 `snap::heirs`，而快照只在入口拍一次
   （`snap.rs:37-39`）；多核下与并行 `Accord` 之间的 TOCTOU（新子门闩不被本次撤销覆盖）
   在代码与注释里都没有交代。
8. **三条自检没有门**：`reclaim`/`spoof`/`name` 不在 `scripts/examine.nu` 的步骤表与任何
   marker 里（`:131-143`），只有人工跑过的输出留在 [dispatch.md](dispatch.md) §12。
9. ~~**`crates/env/src/permission.rs:3-9` 陈旧**~~ —— **已修（本轮）**。原记录：`RESTRICT`/`restrict(...)` 这个术语在代码里
   不存在（ABI 动词是 `Narrow`）；「内核侧 `from_bits_truncate` 还原」与拒绝式 unpack 相反。
   「Pole 的 subset 必须含 READ」这条不对称实际落在 `envcall.rs:55-65` 的 `subset_to_pte`，
   作用于 `Open` 与 `Narrow`。

## 9 · 判据与验证

- **`cascade`**（`programs/src/bin/user/shell.rs:326-472`，四段）：①三跳 A→B→C，撤 B 后 C
  必须失效；②无关分支 D 与本体 A 仍可用——覆盖「父在则子在」与「一条边」的精确性；
  ③`release` A ⇒ D 随之下线；④closure 退出 ⇒ 它授出的 Q 失效（退出钩子 `doom`；`Join`
  返真即已收尾，故当场断言）。
- **`reclaim`**（`:556-673`）：①`unseal + release` × 40000 不许耗尽帧池；②封印只归开辟者
  （他人 `Seal` → `Denied`；主人 `Seal` 后他人 → `Dead`）；③开辟者消亡 ⇒ 资源随之回收。
- **`spoof`**（`:675-794`）：`Push` 盖章后 `pull_from` 回来的发送者是自己；把 `1..=200`
  逐个当「猜中的回信 token」发 `Unregister` 全败；目录 id 经 `Reserve` 的 **owner** 求得而非
  `vestor`——身份不由报文自证。
- **`name`**（`:797` 起）：末两段是级联在真实服务上的落地——实例门闩消亡 ⇒ 目录侧副本随
  `sire` 级联摘掉 ⇒ 名字回到「无实例」。
- **门**：非默认档（harden / 框架）断言 `cascade: ok` 与 `stray: 3/3 illegal-id joins denied`；
  默认档的步骤表**不含** `cascade`（`scripts/examine.nu` 的 `STEPS_DEFAULT`）——它只在非默认档跑。
