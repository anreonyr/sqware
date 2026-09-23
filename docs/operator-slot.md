# Operator — 树：**号就是下标**

> 上一刀量出一条真洞：`prog-probe-deep` 一层层往下 `part`，**持树者自己死在第 117 层**
> （`user fault killed: tid=3`），命名空间整个消失。原因是四条私有助手（`look` / `holds` /
> `take` / `put_in`）**按深度递归**，而一台域的栈是 `TASK_STACK_SIZE`（16 KiB）；广度有闸
> （`PANE_CAP`）、一条路有闸（`ROAD_MAX`）、**深度一个闸都没有**。
>
> 这一刀把树从**嵌套结构**换成**一张按号排的表**：四条递归助手一并消失，深度从"调用栈上的东西"
> 变成"你建了多少格"（容量问题）。
>
> 本文件是这一刀的设计记录。四关各由用户裁决一次，`[x]` = 已定。

---

## 0. 一句话

**号 i ↔ `slots[i]`。**取一格是一趟查表，去一格是从父的 children 里摘一号——**不递归**。

而 core.rs 的头注里那句"换'按号排的一张表'那一案……**留到读数说话**"——读数来了，就是这一刀。

## 1. 功能模型：**公开面一条都不动**

七条原语的签名一字不变（`land` / `part` / `find` / `trim` / `list` / `seek` / `name`），
`EntryId` / `Where` / `Fail` 也不动。**这是纯表示层的改动**：同一个号、同一组操作，换一种存法。

⇒ 于是"既有的读数一字不改"成了这一刀**第一道**判据（见 §6）。

**照实记（后一刀）**：往下那一刀在这个结构上**加**了两样，不是改——第八条只读
`Operator::opens`（答"第 `n` 格是谁的门牌"、不上线），以及把注入的**两枚戳子**收成一格具名的组
`Stamps`（`Operator::new(stamps, unship)`）；两枚戳子同型，摆成位置参数时写反了编不过。见
[`operator-rule.md`](operator-rule.md) §7.5。

## 2. 结构

```rust
pub struct Operator {
    root:  Vec<EntryId>,          // 根那一层的孩子；根**仍然没有号**
    slots: Vec<Option<Slot>>,     // 号 = 下标；None = 墓碑
    next:  usize,                 // 铸号水位，只增（= slots.len()）
    vested_by: VestedBy,
    unship:    Unship,
}
struct Slot { name: Name, node: Node }
enum   Node { Pane(Vec<EntryId>), Tile(PieToken) }
```

四格要点：

1. **根仍是字段，不是槽**——"根没有号"那条类型义务照旧（`EntryId(0)` 仍是第一个真格子 `sys`）。
2. **`Pane` 里装孩子的号**（按登记序）。
3. **墓碑是 `Option<Slot>` 的 `None`**，不是给 `Node` 加一格 `Gone`："这一格死了"是**槽**的属性，
   不是"去处"的属性；而 `Vec` 的指针有 niche，`Option<Slot>` 不多占字节。
4. `Entry` / `Node` 这两个类型原来是 `pub`（`mod.rs` 里出 crate），但**全仓没有一个外部使用者**
   ——这一刀把它们从公共面撤掉：`pub use core::{EntryId, Fail, Operator, Unship, VestedBy, Where}`。

### 裁决

- [x] **不记 `parent`**：`unlink` 用一趟 O(槽数) 的扫（几十格）。我们没有 `..`（名字只从根往下），
      故 Linux 的 `dentry->d_parent` 那一格在我们这里**不是必需品**；记它反而多一份真相
      （`children` 与 `parent` 互为逆，要多维护一处）。
- [x] **墓碑不给手拍的上限**：分配失败如实答 `Fail::Full`（上一刀在宿主靶上量过那条判据有牙）。
      代价照实记在 §5。

## 3. 原语：四条递归助手 → 三个平函数

| 老一版 | 这一版 |
|---|---|
| `look(level, id)` **递归** | `slot(&self, id)` = `slots.get(id.get())?.as_ref()` —— **一趟查表** |
| `holds(level, id)` **递归** | 消失（"在不在"就是"取不取得到"） |
| `take(level, id)` **递归** | `unlink(&mut self, id)`：那一槽标 `None`，再从**某一个**父的 children 里摘 |
| `put_in(level, target, …)` **递归** | 消失；`put` 直接 `kids(at)?` 拿那一块 `Pane` |
| `put_here(kids, …)` | `put` 的后半段（`kids_mut(at)?.push(fresh)`） |
| `seek`（本来就迭代） | 形状不变，每段多一次 `slot(…)` 的读 |

外加两个只读助手：`kids(&self, at)`（那一块 `Pane` 的孩子，判据与老版的走法逐条对齐：
号不在 ⇒ `Unknown`、是 `Tile` ⇒ `NotAPane`）、`child(level, name)`（在一块里按名找号）。

**`name` 顺带从 O(全树) 变成 O(1)**（老版是一趟全树扫）。`find` / `trim` / `list` 同理。

## 4. 签名与不变量

`Operator::new` / 七条原语的签名**一字未改**（见 §1 的表；后一刀把 `new` 那一格换成 `Stamps`，
另加一条只读，理由见 §1 的照实记）。新增的四格不变量：

1. **号 = 下标**且**只增不减**（`trim` 只标 `None`）⇒ "号不复用"照旧；
2. `children` 里的每个号都指着一条**活槽**（`Some(Slot)`）且**恰好一个父**——老一版这是嵌套结构的
   **构造性事实**，这一版要靠每条写原语维护（**唯一新增的不变量**）；
3. 根不进 `slots`；
4. **顺序照旧**：`list` 答登记序、号答铸出序。

**"先要位、再落格"**照旧（上一刀立的纪律）：`slots.try_reserve(1)` 与 `kids_mut(at)?.try_reserve(1)`
都成功了才 `push`——**半路失败不留半个状态**。

## 5. 已知边界（记着）

1. **铸过的号永远占一格坑**：`trim` 不能 `Vec::remove`（号是下标，一移后面全错位），故
   `part` + `trim` 循环能把槽表**单调整长**——CPU 换内存，失败时如实答 `Full`（不崩）。
   这一格**没有**给手拍的上限：那个数在仓里是一个新概念，而手拍的常数在这里已经撞满过三次
   （`Desk::CAP` 8 → 12 → 撞满）。
2. **`children ↔ 槽`是两条真相**：不变量 2 由写原语维护，不再是构造性事实。若哪天它被咬，
   补法是把 `parent` 加回来（Linux 那一格）——**今天不值**。
3. **`unlink` 是 O(槽数)**（一趟扫找父）：几十格，读不出差别。真到几千格再谈 `parent`。
4. **持树者是串行的，而"一个客人连打上千手"会把别的客人挤到超时**——照实记：这是这一刀
   顺带量出来的**真性质**（`probe-deep` 最初打 512 层 + 512 手剪回来，共约 1030 次同步往返；
   那一轮 `echo` 的 `name` / `trim` / `list` 三手连着 1 秒过期：`pname=- trim=7 op=7 seq=1`）。
   与这一刀无关（串行服务是老一版就有的），但**门里不该由探针制造它**——探针改成"每 16 手
   让一手"（**每层**一次 `room::sleep(1ms)`）。真要收这一格，是**调度/配额**那一族的事，不是树的事。

## 6. 判据（改前 / 改后各一条真机读数）

**改前**（同一台探针，不进装配单）：

```text
operator: tid=3                                   ← 持树者（临时一行自报，量完撤）
probe-deep: alive at 32 / 64 / 96
[ERROR] reserved region access: Store at VA(0x1bff8), pc=0x10028
user fault killed: tid=3 cause=15 stval=0x1bff8   ← 持树者自己被杀
probe-deep: tree tid=21 last=116 cap=512 code=7 after=err:7
                                  ↑ 第 117 手答不出   ↑ 命名空间没了
```

**改后**（手工跑那一趟）：


```text
probe-deep: alive at 48 / 96 / 144 / 192
probe-deep: tree deep=192 land=0 find=0 clean=1
echo: list root=0,3        ← 既有三条读数一字未动
echo: list names=sys,device
echo: list device=4,5,6
```

同一台探针：**打到 192 层（老死线 117 的 1.6 倍）、在最底落一枚、寻回来、从最底剪几层**。
探针的链建在 `/sys/deep` 下、不落根（落根会动那三条既有读数，而这一刀不该动它们）。

### 自动的那道门在**宿主靶**上（探针不进 soak）

**为什么**——照实记：把它塞进装配单试过四种排法，都不稳：

| 排法 | soak | 咬在哪 |
|---|---|---|
| 512 层 + 全剪回来，排在 `echo` 前 | **0/10** | 连打约 1030 手同步往返，把 `echo` 的 1 秒期限挤过期（`pname=- trim=7 op=7 seq=1`） |
| 256 层 + 每 16 手让一手 | **8/10** | 让手粒度不够：一次 16 手的突发在慢机上仍吃得掉那 1 秒 |
| 256 层 + **每层**让一手 | **0/10** | `echo` 全对，但**探针自己被带走**：`echo` 一退，编排域返回，把还在剪链的它收走（连 `exit` 行都没打出来） |
| 挪到装配单**最后** | **0/10「无停机行」** | 编排域等的是最后一条，而它 `board: false`——板看不见它的死，那一等没人应 |

故：**探针装得上电、不上电**（`INITRD_BINS` 里有条目、`PLAN` 里没有），真机那一对读数是**手工
跑的**；而**自动的回归门**在宿主靶上——`crates/operator-case` 的
`a_deep_chain_does_not_need_the_call_stack`：把测试线程的栈压到 64 KiB 再建 500 层链。
照实记：**退回递归版它当场 `fatal runtime error: stack overflow`（SIGABRT）**，这一版过。
那条门**有牙、且不抖**——一个已经被守住的属性，不该再让每次 `examine` / `soak` 为一个爱挤人的
探针付抖动的代价。

> 这一轮的教训照实记一句：**"读数必须进 soak"** 这条纪律的用意是"读数要有自动的门"，
> 而不是"每一台探针都要塞进装配单"。宿主靶那条门满足了它的用意，塞进装配单反而是拿门的稳定性
> 换一个已经有的东西。

### 顺带量出来的三条**装配单性质**（不管探针去哪都该记着）

1. **持树者是串行的**：一台客人连打几百手同步往返，会把别的客人的期限挤过期；
2. **装配单最后一条必须"上板"**（`board: true`）——编排域等的就是它；板看不见它的死，
   机器就**永不停机**；
3. **`echo` 一退，编排域返回，会把还在跑的子域一起带走**。

## 7. 不在这一刀里

- `parent` 那一格（§2 的裁决）。
- 墓碑的上限（§5 之一）。
- `fail_to_code` 那一族、线上帧、门禁、账——**一个字没动**（`Fail` 六格原样）。
