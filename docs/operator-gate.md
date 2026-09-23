# Operator × Principal × Coalition — 门禁那一刀

> 状态：**第一刀已落地**（见下面的"落地程度"）。每一节末尾的「裁决」是要人拍板的那一格
> （`[x]` = 已定，`[ ]` = 待签）；这一刀八格全取推荐位，故 §0–§2 基本是已定。
>
> ## 落地程度（真机读数）
>
> | 件 | 状态 |
> |---|---|
> | 判据 `operator::judge`（三格裁决 + 三个注入事实，两个号泛型） | ✅ `crates/protocol/src/operator/judge.rs` |
> | 裁决 → 线上那一格 `operator::gate`（`Code` / `Control` / `Blind` / `verdict`） | ✅ `crates/protocol/src/operator/gate.rs` |
> | 线上两格新码 `DENIED=8` / `UNJUDGED=9` | ✅ `operator/call.rs`（值与 `gate` 那一份由编译期断言钉住） |
> | 宿主台第三台（`judge-case`） | ✅ `scripts/host.sh` 三台共 **42 例全过**（**下一刀之后 25 例 / 48 例**——它同时编 `judge` + `gate` + `ledger`） |
> | 树那一侧：认门牌 → 开门禁 → 判 `land`/`find`/`trim` | ✅ `programs/src/supervisor/operator/server.rs` |
> | 装配那一侧：身份服务**自己**把门牌交给持树者 + 递一格号 | ✅ `principal/server.rs` + `operator/bridge.rs` + `service.rs` |
> | 真机门 | ✅ `examine` **3/3**、`soak` **10/10**（既有 11 条 `tree part=0 …` 与 3 条 `list` 一字未变） |
> | 下一刀 | [`operator-rule.md`](operator-rule.md)：规矩那一格的形状（`Is` / `Under` / `In` 通线） |
> | **真机负证（一）**：没身份（`prog-probe-denied` + `Program::bind = false`） | ✅ `probe: tree land=8 seek=err:1` + `probe-denied: denied as expected` |
> | **真机负证（二）**：有身份但那一格归别人（`prog-probe-owner`，撞 `uart` 的 `/device/uart`） | ✅ `probe-owner: tree land=8 before=5 after=id=5` + `probe-owner: owner rule held` |
> | **真机正证**：主人**不在场** ⇒ 那一格重新可落（`prog-probe-lease` 落完 `/sys/lease` 就死，`probe-owner` 接手） | ✅ `probe-lease: landed, leaving` + `probe-owner: lease land=0 id=7 (owner gone ⇒ take-over)` |
> | **规矩那一格**（`land` 帧尾第 51 字节） | ✅ 这一刀只有"改"那一轴（`Owner` = "归落牌那一位"），服务侧的账只登记它。**"用"那一轴是全局默认 `Public`**——下一刀把它落成逐格的事实 |
> | `Desk` 从定长数组改成可增长（`Vec` + `try_reserve` 报 `Full`） | ✅ 两次撞满常数（8 → 12 → 门禁这一刀又满），第三次改成可增长 |
>
> **还没做的**（见 §8）——**下一刀做掉了前两条**，见 [`operator-rule.md`](operator-rule.md)：
>
> - ~~逐条目的规则（今天所有条目共用 `Rule::Public` 这个默认值）~~ → **已做**：`Ledger` 逐格记
>   "用"那一轴（`Is` / `Under` / `In` 在真机上各有正负证）；
> - ~~`Rule::In` 要的结盟那一枚门牌~~ → **已做**：盟册那一枚由 coalition 自己交给持树者；
> - `seek` / `part` / `list` / `name` 进不进闸口 → **裁决：不进**（规矩挡的是钥匙，不是目录）。
>
> 照实记一笔：这一刀留下的 `Publishers`（只记 `Owner` 的那些）与 `container_of`（O(树) 的全树
> 递归）在下一刀里**整体撤掉**——它们正是"账按一把钥匙记"那个缺陷的两处补丁。

---

## 0. 功能模型

今天的事实（逐条有读数）：

- 树的**唯一**出口是"把名字译成一枚号，再把那一枚经会话授给客人"（`crates/protocol/src/operator/mod.rs`）；
  `seek` / `find` 之外没有"只查不拿"的路——**查属性本身就是授权**。
- 树的通道是**按需发的通行证**：拿到它的域就能 `land` / `trim` 整棵树
  （`programs/src/supervisor/service.rs` 的 `Program::operator` 注自己写着"本正文不做权限判断"）。
- Principal 与 Coalition 已经能答"这一位是谁""这两位在不在一起"，但**树一次都没问过它们**
  （`crates/protocol/src/operator/` 里连这两个名字都没出现）。
- 板的 `lookup` 已无真客人（`crates/protocol/src/system/board/mod.rs` 的照实记）⇒
  **按名找服务只有树这一条路**。

缺的不是能力，是**移交点上的那一次裁决**：

```text
客人 ──seek──▶ 名字
客人 ──find──▶ 号 ────────────────┐
                                  │  ← 裁决发生在这里（交出 / 落下之前）
树 ──▶ tree.find / tree.land ◀────┘
```

**裁决点**：`programs/src/supervisor/operator/server.rs` 的 `answer()`。
**不进核心**：`crates/protocol/src/operator/core.rs` 保持"不出现 `runtime::`"（理由见 §3）。

### 裁决

- [ ] 裁决点在适配层、核心一字不改。
- [ ] 闸口先只做 `land` / `find` / `trim` 三条；`part` / `list` / `seek` / `name` 四条先不判
      （前三条会改树或交出权柄，后四条只读结构）。

---

## 1. 结构：那条边怎么铺

### 1.1 拓扑

```text
装配者（System 域）
  │ ① 认下 principal 交给生我者的门牌（今天已有）→ 再转授一份给树
  │ ② 同样一条给 coalition
  ▼
树域（prog-operator，一枚线程）
  ├─ 常驻：两枚门牌 → Face::of → /sys/principal、/sys/coalition
  └─ 开闸判据：**树手里有没有门牌**（拿到就开）
```

### 1.2 为什么不由树自己 `seek`

树若走 `operator::open` → `seek("/sys/principal")` 去找它俩，就成了**自己的客人**：
会话、`Desk` 格、以及"树自己那次 `find` 由谁裁决"三重套娃。故只能由装配者**直接转授**。

`Face::of(entry)` 只需要 `opened_by` + 那枚门牌本身，**不另开会话**——"两枚号在手"就等于"永远有 face"。

### 1.3 开闸判据：树手里有没有门牌

装配单顺序是 `operator(1) → principal(2) → coalition(3) → …`。时间线（**照实记：这一格实测返工过一次**）：

| 步 | 谁 | 做什么 | 树手里的门牌 |
|---|---|---|---|
| 5 | System 起 principal | principal 自己：读 `Sire` → 上板 → 铸门牌 → **上树**（`part /sys` + `land /sys/principal` + `find` 验一遍）→ **把门牌直接交给持树者** | 有 |
| 6 | System 特判那一格 | `derive(ROOT)` 绑 principal 自己 → `derive(ROOT)` 绑**树** | 有 |
| 7 | System | 把**身份服务的号**经 16 字节那一帧推给树 ⇒ 树按"谁开的 + 入口记号"在本表里认出那一枚 | **有** |
| 8 | System 起 coalition | 它起手 `find /sys/principal` 拿 `Face`，再 `part /sys` + `land /sys/coalition` | 有（门禁生效） |

**判据 = 树手里有没有门牌**，不另加显式握手信号。两个理由：

1. 那一帧紧跟 principal 起来之后 ⇒ "没有门牌"与"装配期"**等价**；
2. 装配期那些动作（第 5 步 principal 上树挂门牌）**根本不经过 `answer()`**——装配者是发起 `Ship`
   的那一方，它把孔直接授进对方表里，主循环收到的不是一笔"从问话孔进来的请求"。

**没有门牌时遇到客人请求怎么办**：答 `UNJUDGED`（判不了），**不许**答 `Allow`——否则"协调服务没配上"
就退化成"没人守门"。装配诊断读数要能指出"死在本域没有门牌那一步"。

**照实记（返工的那一步）**：第一版让**装配者**把 principal 那一枚门牌**再转授**给树。
真机在 `principal` 那一格报 `operator:coord-ship`，内核答 `-1 Denied`——而装配者手里那一枚的
权限位是对的（实测 `perm=0x3`：`FETCH|STORE`）、也在表里；`Accord` 那三道闸（覆盖子集 / 持
`VEST` / 形态一致）都不是原因。**改成"由身份服务自己交"**：它 `serve_tree` 之后把门牌直接
`ship` 给持树者（它本来就是树的客人），装配者只剩"递一格号"。改完一次通过。
——不是设计换了，是**第二手转授在装配窗口里带进了说不清的锚与来历**；一手交就没这问题。
（顺带：principal 交给装配者那一份从 `STORE` 改成 `STORE|FETCH`——`Accord` 的子集不许越界，
而装配者要把这一枚再往外交、树也要拿它读答话。）


### 1.4 门禁装在入口，不装进实现 ★

**这一条是"树自己那条出边不得撞自己的门禁"的确切含义。**

树要答一次裁决，必须能**出边**：往 `/sys/principal` 的门牌推一帧、从本趟借来的回信孔读回来。
若把门禁写成"**每次动树之前都判一下**"这种通用前置，就会自指：

```text
树要回答 ──▶ 得先判 ──▶ 判要问 principal ──▶ 而"问"这个动作也走那个前置 ──▶ 又要先判 …
```

所以门禁**只放一处**：`answer()` 的门口，也就是**只判"从问话孔进来的请求"**。
树的出边（`Face::resolve` / `heir` / `amid` 的 push/pull）在树域内部，是树自己的会话，
门禁看不到它。再加上"门牌由装配者转授进来"（不是树自己 `seek` 来的），
连"取门牌"那一步都不产生一次"树对自己的请求"。

> 纪律一句话：**闸门装在入口，别装进实现**。装进实现，任何内部动作（应答、重试、收场）
> 都会变成一次自问。

### 1.5 `part` 是幂等的——层级不是数据，是"各方挂载时涌现的"

装门禁要碰"谁能 `part`"，而这一格今天有两件事没写进正文，先补在这里。

**`part` 走两遍是对的。** `principal` 与 `coalition` 的 `serve_tree` 起手都是
`part(Where::Root, "sys")`——同一个名字、同一格。不是 bug：`Want::Pane` 在 [core.rs](../crates/protocol/src/operator/core.rs)
里是**幂等**的（`want == Want::Pane && matches!(node, Node::Pane(_))` ⇒ 答原号、什么都不动），
所以第二遍不铸新格、不换绑、不动号。真机读数钉着这一点：

```text
echo: list root=0,3          ← 根只有两格，没有第二块 sys
echo: list names=sys,device  ← 只有一块 sys
```

**为什么两台都非走这一遍**：

- `part` 的返回值是 `land` 的**容器坐标**（`Where::At(EntryId)`），而号只在**持树者的表**里铸或查
  ——没走过这一遍的人手里没有那个号，借不到；
- **顺序不许约定**：谁先上来，"我的父格存在"都得自己保证。这正是"加一条服务 = 装配单加一行、
  `service.rs` 一字不改"能成立的原因之一：父目录由服务**自己**声明，不需要中央机构预建拓扑。

**模型事实（值得写进正文）**：树里没有"目录"这个对象——只有"某个名字上现在是一块 `Pane`"。
`/sys` 存在，是因为有人用 `part` 把它**变成**了一块 `Pane`。所以"三台都挂 `/sys`"在树里读不出来
是"声明了三次"还是"一次"——**幂等把它抹平了**。

**窄口子（这一刀必须答）**：`Want::Pane` 碰到那一格是 `Tile` 时会走默认分支——**换掉它**
（旧的那枚 `Unship`），答原来那个号。即：若哪天有人把 `/sys` 落成一枚砖，下一位 `part /sys` 的会
**无声地**把它换成空 `Pane`，而它自己以为只是"分了个目录"。今天不发生（`/sys` 一直是 pane），
但门禁一问"谁能 `part`"，这一格就从"没人想过"变成"必须答"。

**省不省这一遍**：不省。甲（装配期预建目录）会把拓扑变成装配器知道的事，撞上上面那条性质；
乙（`part` 加"已经在了"标志）要动定长 9 的号帧（`ID_REPLY_LEN`），为省一次幂等调用改帧不值。
丙（现状：谁挂谁声明）= 多一次几十字节的往返，换掉整个中央目录管理。

### 裁决

- [x] 树的两枚协调门牌由**装配者直接转授**（不经树门禁、不经 `seek`）。 ← 已定
- [x] 域划分 / plan 顺序 / 特权级都不动。 ← 已定
- [x] 门禁只在 `answer()` 门口判；树的出边不经它。 ← 已定（§1.4）
- [x] 开闸判据 = **树手里有没有门牌**；没有门牌时客人请求答 `UNJUDGED`，不许答 `Allow`。 ← 已定（§1.3）
- [ ] `part` 那条窄口子（会静默顶掉一块 `Tile`）要不要在门禁里显式答拒（`NotAPane` 那一格已有判据，
      但 `part` 走的是换绑分支）；还是先在正文里记一笔。

---

## 2. 装配期 vs 运行期：为什么"编排域不绑身份"不咬人 ★

**这一条是"编排域自己不绑身份"的确切含义。**

### 2.1 事实：装配者确实没绑

装配器的原话（`programs/src/supervisor/service.rs` 的 `assemble` 头注）：
"其后的每一条都在 `start` 里、**放行之前**拿到 `derive(root)` + `bind(rep, p)`。
**装配者自己（编排域这一枚）不绑**：它是写名册的那一个，不是被写的那一个。"

### 2.2 事实：规则遇到这种 TID 只能拒

`principal::Face::resolve(tid)` 对没绑的 TID 答 `None`（`None` = 没绑，**不是失败**）。
门禁拿到 `None` 只有两个选择：放行或拒绝。**必须拒绝**——否则"不去绑身份"就成了一条
**通用后门**（任何域只要不绑，就绕过所有规则）。故：

> **`resolve(who) == None` ⇒ 拒绝。**

### 2.3 两条合起来：装配期不经门禁，所以不需要装配者有身份

| | 装配期 | 运行期 |
|---|---|---|
| 谁 | 装配者（未绑身份） | 各客人（`derive(ROOT)` + `bind`） |
| 动作 | 建域、`Ship`/`Accord` 直接授孔（提示孔、答话路、门牌） | `seek` / `find` / `land` / `trim` 问树 |
| 经不经树 | **完全不经** | 经，且每次裁决 |
| 门禁 | 看不到（不经过 `answer()`） | 每次都判 |

装配期之所以不经树，不是给它的特例豁免，而是**它没有理由经过**：它要的"对端是谁"自己就知道
（它自己起的那一枚），它要交的孔是直接 `port::ship` 到对方表里的。

**推论（两条限制，不是 bug）**：

1. **门禁规则不能拿"编排域"当主人**：规则想判"这一位是不是装配者"，答案永远是 `None` ⇒ 拒绝。
   要用它当主人，得先给它身份；而它的身份只能由 principal 或 root 补——又回到启动顺序那一格。
2. **将来若装配者要读树，它会当场被拒**：门禁上线后若有人让装配器也 `operator::open` → `find`
   去拿点什么，它每一步都会被拒。踩到的人该改装配路径，**不要**给规则开"TID == 装配者就放行"的洞
   （那等于把 root 装回来）。

### 裁决

- [x] `resolve(who) == None` ⇒ 拒绝（不许有"没绑就放行"的后门）。 ← 已定
- [x] 装配期的一切走直接转授，不经门禁；编排域不绑定身份。 ← 已定
- [ ] 要不要在正文里明写"编排域不是树的客人"这一条边界（建议写，否则下一刀会有人去踩）。

---

## 3. 签名

### 3.1 线上码（`crates/protocol/src/operator/call.rs`）

```text
现有： OK=0 UNKNOWN=1 NONEMPTY=2 NOTATILE=3 NOTAPANE=4 FULL=5 DEAD=6 BAD=7
新增： DENIED=8        ← 你没资格（终态，别重试）
       UNJUDGED=9     ← 现在判不了（对面没答上来；可重试）
```

**为什么两格而不是一格**：`UNKNOWN` 已经背着"没铸过 / 剪掉了 / 剔死了"三种"长得一样"的情形；
把"你不许"塞进去，客人分不出"该重试"还是"该放弃"。而"身份服务挂了"与"你没资格"必须是**不同的下一步**。

`Fail`（核心那六格）**不动**：这两格只活在适配层与线上。

### 3.2 树的适配层（`programs/src/supervisor/operator/server.rs`）

```rust
/// 一次裁决要的四样：谁在问、问哪一个条目、要干什么。
fn judge(who: TaskId, at: Target, act: Act) -> Ruling;  // Allow | Deny | Unjudged
```

- `resolve(who)` → `None` ⇒ **Deny**；
- 规则来自**资源主人**（那枚门闩的 `owner`），**不进树**；
- `heir` / `amid` 各一次 envcalls；
- 对面超时 / 答不上 ⇒ **Unjudged**。

### 3.3 装配侧（`programs/src/supervisor/service.rs` + `operator/bridge.rs`）

```rust
// bridge::attach 多带两枚（与 tip 同形状：只传不认领）
pub fn attach(
    quay: &mut Quay,
    client: TaskId,
    host: TaskId,
    millis: usize,
    tip: &mut Option<PieToken>,
    principal_entry: &mut Option<PieToken>,   // 新
    coalition_entry: &mut Option<PieToken>,   // 新
) -> Result<(), &'static str>
```

### 裁决

- [ ] `DENIED` / `UNJUDGED` 取 8 / 9（与既有 0..7 相邻，不重排）。
- [ ] `judge` 的签名按上面那样（`Act` 三格）。
- [ ] `attach` 是加两个形参，还是把"装配期交的几枚"收进一个结构体（今天已 5 个形参）。

---

## 4. 为什么裁决不进核心

核心那两个注入事实（`VestedBy` / `Unship`）是**同步纯函数**：它们答"这枚还答得出吗""放下它"。
而裁决要**发一次 envcalls**（`Resolve`），同步闭包做不到；何况 `core.rs` 不出现 `runtime::`
是它的可机械检查纪律，也是 `crates/operator-case` 那台宿主门能存在的原因。

所以：**核心答"结构上允不允许"（六格 `Fail`），适配层答"这一位许不许"（两格新码）**。

### 裁决

- [x] 核心不动，`Fail` 不加格。 ← 已定
- [ ] 宿主台怎么证：判据要写成一个**能被宿主台编到的纯函数**（住 `server.rs` 之外的小模块），
      否则 `operator-case` 看不见它。

---

## 5. 配的台子

| 台 | 证什么 | 落点 |
|---|---|---|
| 宿主台 | "被拒 ⇒ 那一格**没被占**" | 照 `crates/operator-case` 现有假表形状，新增纯 `judge` 模块 |
| 真机负证 | `land=DENIED` **且** 随后 `seek=UNKNOWN`（第二格才证"拒绝不是换绑"） | ✅ 已落地：`programs/src/user/probe_denied.rs` + `Program::bind = false`（装配者**不绑它**），读数 `probe: tree land=8 seek=err:1` |
| 既有门不许退化 | `scripts/soak.sh` 那 11 条 `tree part=0 dir=… land=0 find=0 got=true` 与 3 条 `list` 一字不变 | 默认策略必须是"**没规则 ⇒ 放行**"，否则整机装不上 |
| 顺序回归 | 装配顺序与 `echo: seq=0` 不变 | 同 soak |

---

## 6. 做之前先认下的坑

1. **`Desk::CAP = 12` 要重算**：树自己成为 principal / coalition 的常驻客人（今天 6 位常驻，还够）。
   处理办法见 §7。
2. **树自己那条出边**必须能绕过自己的门禁（§1.4）。
3. **编排域自己不绑身份**（§2）：规则对"没绑的 TID"答拒绝，装配者走直接转授那条路。
4. **一次 `find` 的成本**：探活 + `Accord` 之上再加 1～2 次 envcalls；`Rule::Allow` 是唯一能省掉第二次的规则。
5. **`clan` 不上线**：想用"同一支"这种谓词得先把它发上线（核心有，线上不发）。
6. **`heir(a,b)` 与 `amid(p,c)` 入参方向相反**——最容易写错、又最难在宿主台发现的一格。

---

## 7. 顺带：`Desk::CAP` 从定长数组挪到可增长（待签）

### 今天

```rust
guests: [Option<Guest>; Desk::CAP]   // programs/src/supervisor/operator/desk.rs
pub const CAP: usize = 12;           // 5 常驻 + 4 会同时在场的临时 + 3 格余量
```

`CAP = 12` **不是架构常数，是量出来的**（那份照实记写着 8 → 12 的全过程：结盟那一刀带来
第五位常驻与又多一位会死的，8 格卡满 ⇒ 最后上树的 `echo` 的 `admit` 答 `Full`，
而客人自己不知道 ⇒ 整机收不了场）。它**已经失败得体面**：满了打一句 `operator: desk full`。

### 换 `Vec` 可行（三格都对得上）

1. `programs/src/lib.rs` 有 `extern crate alloc;`；
2. `desk()` 虽写着 `const fn`，唯一调用点是运行期的 `let mut desk = desk();`——改 `fn` 无痛；
3. `Slot` 是"同一次 `admit` 的返回值、当轮现查现用"，`Vec` 下标位移不破坏它。

**唯二的附加成本**：`server.rs` 里那处 `[(0usize, TaskId::new(0)); Desk::CAP]` 栈数组要一起改
（或直接 iterate `unarmed()`）；以及 **`push` 不许裸奔**——照 `principal` / `coalition` 的模板
`try_reserve` + 报 `Full`。

### 但换容器不等于去掉上限

`Vec` 只去掉**编译期常数**，没去掉"有多少位客人"这个事实。建议三件一起做：

```text
① guests: Vec<Option<Guest>>                         备不下报 Full
② 上限策略 = 计划表里 operator:true 的条数 + 余量     装配期算一次，进 Desk::new
③ 一处峰值读数                                       何时该调，由读数说，不由事故说
```

### 裁决

- [ ] 换 `Vec`（含 `waiting` 那处）。
- [ ] 上限从"手拍的 12"改成"按装配单算 + 余量"。
- [ ] 加一格峰值读数。

---

## 7.5 两条**实测**教训（都写进代码注了）

**① `find` 不能看主人。** 第一版把"归落牌那一位"也挂在 `find` 上，`uart` 一声明归自己，
`echo` 当场取不到 `/device/uart` —— 机器还在、控制台没人读（`examine` 0/3）。**读是公开的，
写才归属主**：`Rule::Owner` 管的是**改这一格**（换绑 / `trim`），不是**用这一格**。
今天的分工：`find` 只看身份（[`may`]），`land` 看那一格的主人 + 身份，`trim` 两者都看。

**② `land` 也必须问身份。** 漏了这一支，负证客人当场落牌成功（读数 `probe: tree land=OK id=7`）
——"往命名空间里挂东西"这件事本身要求来的是个**已绑身份**，否则没身份的任务就能往树上塞条目。

**④ 主人那份账要按**坐标**记，不能按号记。** 第一版记 `(EntryId, TaskId)`，而 `land` 那一支
拿 `tree.seek(&[name])` 去查**根下**同名——门牌在 `/device` 底下，查不到，于是**整道判据被跳过**，
`probe-owner` 当场顶掉了 `uart` 的牌子（读数 `land=OK id=5`）。改按 `(Where, Name)` 记之后一次通过。
两个理由：**号只在树里认得出**（门口这一问发生在动树之前），而 **`land` 换绑不动号**——
号也分不出"这一格换过人"。

**③ 帧加一格的兼容法。** `land` 从 50 字节变 51，规矩那一格**读不到就按 `Public` 走**
（`Rule::from_wire` 的兜底）——一个陌生的规矩字节不该把整句问话判成 `BAD`。

---

## 8. 不在这一刀里

- ~~甲档坑（按记号认领、转授失败不答 `OK`、`try_reserve`）——树的内部正确性，另一刀。~~
  → **已复核并收口**（下面这一节是复核的照实记：**原记账三处，两处要改**）。

  | 原记账 | 复核结果 |
  |---|---|
  | `try_reserve` | **真**，而且是"**同一句话，三处纪律不一致**"：树这一处只有**条数**闸（`PANE_CAP = 16`），分配失败走的是 `handle_alloc_error`（**abort**，客人连一句答话都收不到）；而 `Desk::admit` 与 `Ledger::grow` 都是 `try_reserve → Full`。**已收**：`core.rs::put_here` 加 `try_reserve`（答 `Fail::Full`），`server.rs` 的 `settle` 快照也从裸 `collect()` 改成先 `try_reserve`（备不下就报一句 + 这一轮不动）。**宿主台上量得到**：`crates/operator-case` 换了一台**可关掉的分配器**（线程局部旗帜 ⇒ 不打搅别的用例），`a_pane_that_cannot_be_grown_answers_full` 一次通过；**撤掉那一行它就 SIGABRT**（实测 `memory allocation of 256 bytes failed`）——那一格判据是真有牙的。 |
  | 转授失败不答 `OK` | **不准**：`answer` 那一支写的是 `tree.find(…).and(grant)`，`grant` **从 `f326e7a` 那天起就在**（`git log -S` 查过）。真正的缺口在更深一层：`session::call::ship` 是 `port::ship(…).map_err(\|_\| ())`——**失败的原因在会话层就丢了**，于是树只能把"我授不出去"借 `Unknown` 的壳（客人读到的是"那一格不在树上"）。**记着**：要真分得清，得动会话层那一格的失败域（`ship` 现在返 `Result<PieToken, ()>`）——而收益有限：授不出去多半是因为那位客人已经没了，它读不到这一格。 |
  | 按记号认领 | **真，但三处的底气不一样**：`reply_of`（记号 `LINK`）由 `Quay::seat` 的同名判据兜着——"同一位、同一记号只可能有一枚"（`session/core.rs`）；而 `ask_of`（`ASK`）与 `find_face`（`ENTRY`）走的是**裸 `unseal_hole`**，**没有同名闸**——一个域调两次就是两枚。三处原先各写一遍同一段扫表、**都取第一枚而从不看有几枚**。**已收一半**：三处合成一个 `claim(记号, 谁, 一句话)`，扫全表、命中两枚就**报一句**（行为仍是取第一枚）。 |

  **没做 fail-closed，也没造那台负证客人**——照实记：这条判据**天生量不出一个确定的东西**。
  两枚孔的出现与持树者查表之间有**天然竞态**：持树者在装配窗口里**每 1ms 查一次**，而两枚孔
  之间只隔两个 envcalls（微秒级）。于是"报一句"是 ≈99.99% 出现（那 0.01% 就是一个间歇红的
  门），"拒"同样间歇**且**那位客人从此没人给它挂孔（持树者会永远停在"还有人没挂上"那一档，
  并每轮重复报同一句）。**间歇红的门比没有门更坏**，故这一刀只把"赌"变成"**看得见的赌**"。

  **干净的关法是另一刀**：让 `ask_hole` 与入口那一枚也走**有名有姓的泊位**（`Quay::seat` 那条
  路已经有同名闸），把"只可能有一枚"从**纪律**变成**构造**。
- `part` 碰到一块 `Tile` 时静默顶掉它那条（§1.5 的窄口子）——**只在正文记一笔**，不改行为：
  今天 `/sys`、`/device` 一直是 `Pane`，为一个不会发生的状态动换绑分支不值。
- ~~规则的形状（`Allow` / `Is` / `Under` / `In` 怎么表达、能不能组合）——闸口跑通后再谈。~~
  → **已做**：闸口跑通了，下一刀（[`operator-rule.md`](operator-rule.md)）把"用"那一轴落成
  逐格的事实，`Is` / `Under` / `In` 三条判据在真机上各有了正证与负证。
- 死亡道（板那条 `gone-<名字>` 树一条都没认领）。
- 别名（同一枚 Pie 挂两个名 = 两条独立条目）。
- **深度**（`seek` 封顶 8 段，树本身无上限）——**已量，是真洞**：
  `prog-probe-deep` 一层层往下 `part`，**持树者自己死在第 117 层**（`user fault killed: tid=3`，
  实测读数与三格分析见 `programs/src/user/probe_deep.rs` 的头注）。原因是四条私有助手
  （`look` / `holds` / `take` / `put_in`）**按深度递归**，而一台域的栈是 `TASK_STACK_SIZE`
  = 16 KiB；广度有闸（`PANE_CAP`）、一条路有闸（`ROAD_MAX`）、**深度一个闸都没有**。
  下场比 abort 更坏：**命名空间整个没了**（同一台探针在那一手之后连最浅的一问都答不出）。
  修法是下一刀：**号就是槽的下标**（树从嵌套 `Vec<Entry>` 换成扁平表 + 墓碑），四条递归助手
  一并消失，深度从"语法策略"变成"容量问题"（与 seL4 的"地址是整数"、DNS 的 255 字节同一格）。
- ~~落牌那一方退了之后，它声明归自己的那一格永远顶不掉~~ → **已解决**（`claimable`）：
  问一次"那一枚还答得出吗"（`vested_by`，与探活同一句话——退场钩子会封印该域开的资源），
  答不出 ⇒ 那一格**重新可落**。**"看出来的"，不是"被通知的"**（与树的惰性剔死同一条形状）。
  这是"规矩属于**活着的**主人"的确切含义；一位**活着的**持有者照样顶不掉（由 `probe-owner` 证）。
