# port — Hole 的通讯协议（Access · Policy · To · ship · Port）

> 路径约定：`文件:行` 相对仓根。权柄模型在 [pie.md](pie.md)，数据面三件套在 [mail.md](mail.md)，
> 服务目录协议在 [dispatch.md](dispatch.md)，空载荷门铃在 [bell.md](bell.md)。
>
> **状态：§1–§6 全部已实现**（`Access` · `Policy` · `To` · `ship` · `Port`，以及 §5 的
> 变长孔）。**§10 是本轮的分层裁决**：往返与帧格式从本层搬了出去。判据见 §9；
> 剩余边界与下一轮的事见 §8。
>
> **落地位置**：`crates/runtime/src/core/port.rs`（四个类型 + `ship` + `Port` 的六个方法）、
> 各协议 `crates/protocol/src/*/wire.rs`（**自己的帧**，含地址槽）、五家客户端
> `crates/protocol/src/*/client.rs`（**自己的一次往返**）、
> `crates/protocol/src/console/session.rs`（开会话的握手）。

## 1 · 语义定位

`Port` 不是内核对象，是**镜像侧的机制**：把**两枚 Hole 门闩配成一对**（一枚推、一枚收）
并记住对面是谁。内核侧**零改动**（除 §5 的消息长度），ABI 除 §5 外零改动。

| 管 | 不管 |
|---|---|
| 授出（`ship`）、对端坐标（`To`）、**两枚孔的配对**（`Port`） | 一次往返的时序与**帧格式**（在 `crates/protocol`）、权柄的判定与级联（在内核 `gate`）、槽本身（在内核 `mail`） |

**与 `Channel` 的关系**：`Channel` 是本设计的**前身**，它留下三条教训、两条病根：

| | `Channel` | `Port` |
|---|---|---|
| 坐标 | 只存 `at_peer`（号） | `To { peer, seed }`——**两半成对**（号只在那一张表里有意义） |
| 收尾 | `revoke(peer, at_peer)?` 后 `release()` | `shut` 只 `release()`——**级联已含对端那枚**（`gate/cull.rs` 沿 `sire` BFS 反查全世界） |
| 句柄 | `mine()` 交出整枚门闩（可 `accord` 出永不撤的副本） | 协议**拿不到句柄**：只给 `seed()` 一个数 + 两个动词 |
| 往返 | 每个协议各写一遍 | ~~`Port::call` 一处~~ → **仍归各协议**（`ask`，5 行）；但"校来源"沉进了 `pull`，**漏不掉** |

## 2 · 两个族

单一真相在 `crates/env/src/permission.rs`（四位分两族）。本层把两族**分成两个类型**，
于是混族在编译期不可表达——`Accord` 收一个混合 `Permission` 时，调用方可以把 `CAGE`
写进"读写"该在的位置。

| 族 | 类型 | 取值 | 问 |
|---|---|---|---|
| 读写族 | `Access` | `NONE` `READ` `WRITE`（`READ \| WRITE`） | 对端对这份资源**能做什么** |
| 传递族 | `Policy` | `NONE` `VEST` `CAGE`（`VEST \| CAGE`） | 这一枚**能怎么流动** |

`Policy` 四个取值读作：

```text
NONE         分  双方各持一份，对端不能再授出
VEST         借  双方各持一份，对端可以再授出
CAGE         让  我失去这一枚，对端不能再授出
VEST | CAGE  给  我失去这一枚，对端可以再授出（粘性：它再授出的必带 CAGE）
```

两问相乘十六格，其中三格是废话——**分轴才看得见它们**：

| 格 | 子集 | 为什么废 |
|---|---|---|
| `Access::NONE + Policy::NONE` | ∅ | 空子集，`ship` 本地拒 |
| `Access::NONE + Policy::CAGE` | `C` | 只有形态声明：无事可做、也传不出去 |

`Access::NONE + Policy::VEST`（`{V}`）是**真格子**：纯票据，`root` 把它授给监护线程
（建域权，`programs/src/bin/supervisor/root/main.rs:835`）。

## 3 · 授出：`ship`

```rust
pub fn ship<P: AnyPie>(pie: &P, peer: TaskId, access: Access, policy: Policy) -> EnvResult<To> {
    let subset = access.bits() | policy.bits();
    if subset.is_empty() { return Err(denied()); }   // 空集本地拒，不发 envcall
    let seed = pie.accord(peer, subset)?;
    Ok(To::new(peer, seed))
}
```

**这就是"授出"的全部机制。** 调用点从此写不出裸子集——今天全仓 27 处 `accord` 手写子集，
其中 `R|W` 出现在"对端只会写"的地方（回信孔、ack 孔），一份子集写错就是一份多余授权。

**为什么泛型于 `AnyPie`**：生产路径上三种资源都有授出点——

| 授出点 | 授出的是什么 |
|---|---|
| 各处回信孔 / 控制孔 | `HolePie` |
| `programs/.../root/main.rs:210`（`hand_over`） | `PolePie`（设备门闩） |
| `programs/.../root/main.rs:835`（`build_right`） | `NolePie`（建域权） |

只收 `HolePie` 就表达不了后两条。`AnyPie` 是仓里"权柄操作与资源种类无关"的现成载体
（`crates/runtime/src/env/mail.rs:225-238`）。

### 按方向授出（不是"角色"）

| 对端实际做什么 | `access` | `policy` |
|---|---|---|
| 推（回信、ack、请求） | `WRITE` | `NONE` |
| 推 + 再分发（服务入口给目录） | `WRITE` | `VEST` |
| 只再授（纯票据） | `NONE` | `VEST` |
| 读（读端**转移**给别的线程） | `READ` | `CAGE` |
| 读 + 写（自环、自检） | `READ \| WRITE` | `NONE` |

**R 只可转移，不可复制**：授出 `READ` 不带 `CAGE` ⇒ 源枚仍可用 ⇒ 两个读者。
单读者不是内核机制，是**位表的推论**（`CAGE` 存在 ⇒ 源枚不可用；`CAGE` 粘性 ⇒ 链洗不掉）。
`docs/mail.md` 的"回信被偷读"因此从纪律变成**从未授出**。

## 4 · `Port`：两枚孔的配对

```rust
pub struct Port { to: To, entry: HolePie, reply: HolePie }

impl Port {
    pub fn open(entry: &HolePie) -> EnvResult<Port>;   // 自建回信孔 + ship(WRITE, NONE)
    pub fn borrow(peer: TaskId, entry: HolePie, reply: HolePie) -> Port;  // 回信孔是**对端开的**
    pub fn seed(&self) -> PieToken;                    // 写进帧的那一格
    pub fn push(&self, frame: &[u8]) -> EnvResult<()>; // 推给**对端那枚**孔（满则等）
    pub fn pull<'a>(&self, buf: &'a mut [u8], within: usize) -> EnvResult<&'a [u8]>; // 收 + **校来源**
    pub fn shut(self) -> EnvResult<()>;                // 只放下回信孔；`entry` 不碰
}
```

**四个动词与 `HolePie` 同名同形**——这一层**不加新动词**。它多出来的只有三件事：
**推的是哪一枚、收的是哪一枚、收的时候校不校来源**（第三条是不可替代的那一条）。

**两个构造入口**：`open` 是"我自己铸回信孔"（一问一答的协议够用）；`borrow` 是"回信孔由
**对端**开、我借来用"（`console` 的会话握手走这条）。后者不是多余的路：对端退场时内核的
寿命边会封印**它开的**孔（`gate::doom`），等在上面的本端**当场拿到 `Dead`**；而 `open`
那一支里服务被打死那扇门不死，本端永久挂在 `Busy` 上，「对端还没回」与「对端已经没了」
不可区分。

**`pull` 返的是"恰好那一帧"**（不是缓冲全长）：长度钉死在帧边界上，§9.3 那个病
（"载荷缓冲按上界给、尾部多余的零被算进名字里"）从此写不出来。

**`to` 不跨进 `protocol`**：协议拿到的是裸 `PieToken`（`seed()`），`peer` 只被 `pull` 用来校
来源。`To` 因此在 `protocol` 层不可见——`Port::to()` 那类访问器仍不立（零调用者的访问器
不留，`programs/src/uart.rs:63-65` 有先例）。

### 4.1 一次往返住在协议里

编帧 → 推 → 有界等 → 校来源 → 解帧，这五步**每家协议自己写**（5 行），因为它要用到自己的
帧格式：

```rust
// crates/protocol/src/uart/client.rs 的 ask —— 最小的样子
let (frame, n) = req.encode(self.port.seed());
self.port.push(frame.get(..n).ok_or_else(denied)?)?;
let mut out = [0u8; CAP];
Status::decode(self.port.pull(&mut out, ACK_WAIT_MS)?)
```

**旧账不会回来**："五处都漏了核对来源"这条漏不掉——**核对在 `pull` 里**，调用方绕不过去。
"五家各写一遍"的成本仍在（每家 5 行），换来的是 `Duet`（10 个关联项）、机制里的地址槽
默认值、以及 `Port` 上那 100 行会话/握手代码一并销掉（裁决见 §10）。

**命中不了来源 ⇒ `Denied`，该会话应弃用**（迟到的真回复仍可能落槽污染下一次 `pull`——
与 `HolePie::pull_timeout` 的既有契约同款，`crates/runtime/src/env/mail.rs:335-336`）。

### 4.2 帧格式：地址写在哪、写几条，全归协议

四个类型（`ADDRESS_LEN` / `ADDRESS_AT` / `put_address` / `address_of`）现在**每家 wire.rs
各有一份**，偏移各写各的：

| 协议 | `ADDRESS_AT` | 哪几条帧带地址 |
|---|---|---|
| uart / doom / irq / console | `1`（紧跟 `op`） | 前三条**每条都带**；控制台**只在 `Open` 上带** |
| dispatch | `0`（帧首） | 每条都带（帧首是唯一让"裸询问"与"服务调用载荷"逐字相同的位置） |

**曾经这个偏移在机制里有一份默认值**（`ADDRESS_AT = 1`）加一个 per-protocol 覆盖点，代价
是一处调用点要从两个 crate 各拿一半知识（`echo.rs` 取协议的偏移 + 机制的取数函数）——§9.3
那次连错三轮就是这个形状逼出来的。

## 5 · 消息长度：舍弃 `mtu`

**动机**：`UnsealHole { mtu }` 把"这条孔上的消息最大多长"钉死在开孔时刻，而那个数其实是
**协议自己的报文尺寸**（startup 9、console 64、doom 41、irq 49、uart 64）——协议知道，
孔却替它记着，还多出一份重复常量（`crates/runtime/src/env/mail.rs:24` ↔
`kernel/src/work/mail/mod.rs:34`，`docs/mail.md` §10.4 记着它"无编译期绑定"）。

**注意**：`Push { token, msg, len }` **已经有 `len`**——"用参数标记消息长度"这半边今天成立。
卡住动态长度的是另外三处：槽的容量（unseal 时预分配）、`len ∈ [1, mtu]` 的闸、
收方的缓冲义务由 `mtu` 界定。

### 5.1 变更（已决）

| 项 | 今天 | 之后 |
|---|---|---|
| `PieCall::UnsealHole { mtu }` | 带参数，`1..=4096` | **无参**（与 `UnsealNole` 同形；只有 `UnsealPole { bytes }` 还带尺寸——页对齐是物理约束） |
| `HoleMeta.mtu` | 字段 + 唯一校验点 | 删除 |
| 槽的容量 | `Vec::with_capacity(mtu)`，push 只拷不放 | **随消息长**：envcall 已在锁外备好 staging Vec，**`mem::swap` 进槽**——零拷贝、无锁内分配、长度随消息 |
| 取消息 | 按 `max` 预分配暂存，再把槽拷进去 | **整条移出槽**（`try_take` ＝ `mem::take`，`hole.rs:229-246`）——零拷贝、锁外零分配；收方拿到的就是那条消息自己那块内存，拷给用户之后随调用释放 |
| `MailCall::Pull { max }` | `max ≥ 1` 且 `≤ mtu` | `max == 0` ⇒ **只报长度、不动槽**（与 `Wait { millis: 0 }`「只探测不挂起」同一形状，不新增 variant）；内核侧的新原语是 `hole::peek` |
| `HOLE_MTU_MAX` | 用户态与内核各一份重复常量 | 删除（**不降为任何全局兜底**，见 §5.2 F4） |

**每个协议的自述尺寸（`MTU = 9` / `MSG_LEN = 64` / `REFER_MTU = 41`）留在各协议里不动**——
那本来就是它们该在的地方。孔不再替协议记尺寸。

### 5.2 后果（四条，末条是"不立"）

**F1 · 槽怎么长（无分叉）**：`envcall/mail.rs` 的 push 路径**已经**把用户内存拷进一个
锁外分配的 staging `Vec`（`try_reserve` + `resize`，失败答 `OoM`）。把这份 staging
**移进槽**（`core::mem::swap`）替代"拷进槽"：零拷贝、无锁内分配，长度自然随消息。
槽的旧 buffer 随本次调用返回调用方（丢弃或复用）。

**F2 · 收方怎么知道长度（已决）**：`Pull { max: 0 }` ⇒ 返槽中消息长度、不动槽
（内核原语 `hole::peek`；runtime 侧是 `HolePie::peek`）。
收方因此**总能把槽清空**（内存允许时），不必再靠 `mtu` 猜缓冲大小。

**F3 · 收方装不下怎么办（已决：解药是 F2）**：今天 `buf` 装不下 ⇒ `Denied` 且**不动槽**
（`kernel/src/work/mail/hole.rs:217-219`）。`mtu` 一去，发送方就能推一条收方装不下的消息，
槽被占死：push 永远 `Busy`、pull 永远 `Denied`。但 **F2 已经解掉它**——收方问得出长度，
就能备出装得下的缓冲，槽因此**总能被排空**。**不新原语，也不改 `Pull` 的"要么全取、
要么一个字节都不动"契约。**

**F4 · 要不要一个全局上限（已决：不立）**：草稿曾把 `mtu` 顺带当成"一个域把内核堆吃穿"的
兜底，于是要补一个机器上限 `HOLE_MSG_MAX`。**这条兜底是假的**：`UnsealHole` 是**预分配**
（`Vec::with_capacity(mtu)`，`kernel/src/work/mail/hole.rs:86-88`）——一枚空孔当场就占 `mtu`
字节，于是 `mtu ≤ 4096` 限的是**一枚空孔的代价**（与"一条消息能有多大"无关；这笔白占已在
§5.1 的"随消息长"里一并销掉）。真正的界是**分配器的** `try_reserve` → `OoM`——同一条路今天
已经在走（push 侧 staging）。故不立上限：一条消息的上限就是"当时还分配得出多少"。

余下的边界（收方出于自身策略不肯分配那么大的缓冲 ⇒ 槽留占）记入 §8。

## 6 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 同一条孔上 `R` 的**可用**持有者恒为一个 | 两个读者各拿一半消息 | 位表（授出 `READ` 必带 `CAGE`）+ `CAGE` 粘性 |
| `To` 的两半成对 | 号在错的表里查（`Channel` 的病根） | 类型：`To { peer, seed }` 私有字段，只能由 `ship` 造 |
| 混族不可表达 | 把 `CAGE` 写进读写位 | 类型：`Access` 与 `Policy` 是两个类型 |
| 空集不成立 | 授出一枚什么都没有的枚 | `ship` 本地拒（不发 envcall） |
| 回信地址与来源校验成对 | 收到别人的回复而不自知 | `Port::pull`（**不可能被调用方绕过**） |
| 回信孔的**开者是对端**时，握手回执走**本端私有的孔** | 推回请求孔 = 推进对端自己的收件箱，被它吸回去 ⇒ 客户端等满上界 | `console::session`（自建私有孔 + 地址槽递出）+ 服务端 `grant` 只推给它 |
| 「开一条会话」只发生一次 | 开两条会话、表每开一次漏一格，第 5 次表满 | `console::session::open` 把首帧的答复一次收齐 |
| 收尾只 `release` 一次 | 冗余的 `revoke` 失败会短路后续清理 | `Port::shut`（级联已含对端副本） |
| 回信地址只在协议说的那条上报 | 服务把不该有的地址当坏报文（控制台就是这样：`Write` 带地址 ⇒ `UnexpectedField` ⇒ **没有回复**，客户端白等满上界） | 各协议自己的 `Query::encode`（`console::wire` 只对 `Open` 写那一格） |

## 7 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| 单读者 | **不立内核席位** | 只有内核能原子判"谁先读"看似成立，但位表已经卖：授出 `READ` 必带 `CAGE` ⇒ 可用读者恒为一个。席位要多一个字段、一个错误码、一条腾座路径 |
| 内核槽携带回信地址 | **否** | 用户态自己做得到（五份 `wire.rs` 今天就在做）。判据：**内核只提供"用户态做不到"的东西** |
| `Pull` 返三件（长度 + 发送者 + 回信地址） | **否** | 随上一条一起销 |
| 位随线形传 | **否** | 位是**自省的**（`Collect` 返 `permission`）；传位是把本地可查的事实复制到线上 |
| 按角色分型的视图句柄 | **否** | 类型矩阵（角色 × 种类）膨胀；授出端的错误才是高发处，那里已由 `Access`/`Policy` 挡住 |
| O12「枚举我授出」 | **不做** | 会话化 + `release` 级联之后 `close` 不再需要它；唯一动机"交出前先清场"无消费者 ⇒ 死代码（与 `docs/pie.md` §8 第 11 条否 `caged` 同理由） |
| 内核从 `heir` 推导回信地址 | **否** | `heir` 只记**带 `CAGE` 的独占交出**且"至多一个"，是锚不是列表；扩展它等于把锚退化成列表 |
| 把两条孔在机制层配成 endpoint | **否** | 配对要回答"谁先建谁后建"，一对多副本会让它变一对多映射——新状态与新失败模式 |
| `send` 收报文偏移（机制填、协议定布局） | **否** | 把"偏移"从 `wire.rs` 拉到调用点，布局知识搬出它该住的地方 |
| ~~`Port` 只留 `send`/`recv`（往返归协议）~~ | **翻**（本轮，见 §10） | 当年否它的理由是**那两条契约**（有界、来源校验）"不是协议语义"。本轮不搬那两条——它们留在 `Port::pull` 里；搬走的只是**它没提到的那部分**（帧格式 + 一问一答的时序）⇒ 理由不成立，裁决可翻 |
| `ReadWrite` 作通信角色 | **否** | 两个读者；只留给自环与自检 |
| `Access`/`Policy`/`To` 进 `crates/env` | **否** | 那是内核也依赖的 ABI crate——`protocol` 从 `env` 拆出来的先例记着："284 行内核永远读不到的协议" |
| 命名 `Wire` | **否** | 与 `crates/env/src/wire/mod.rs` 的 `trait Wire`（字段 ↔ usize）撞名；当时改叫 `Duet`——本轮连 `Duet` 一起删了（见 §10） |
| 命名 `Ferry` / `at` / `give` | **改** | 定为 `Port` / `To` / `ship`（同族：`dock` `moor` `Quay` `Pier`） |
| `To` 的两半叫 `peer` / `seed` | **成对**（用户裁决） | 号只在那一张表里有意义 ⇒ 名字里必须带上是**谁**的表；`seed` 读作"我种在对端表里的那一枚"，回复沿它回来。本仓其余种出去的号（`Quay`/`Pier`/`Referred`）对端由**方向**给出，故号可以单飞——**回信孔不行**（谁都能往里推），这正是 `Port::pull` 要校 `from != to.peer()` 的根 |
| ~~`Duet` 的报文容器~~ | **已销**（本轮） | 当年要它是因为"通用往返必须让协议自述帧形状"。往返搬回协议之后，"报文多大"就是各协议自己那块 `[u8; CAP]`，不需要向任何人申报 |
| 服务调用（dispatch 的 8+56 载荷）的形状 | **落在 `Service::call` 自己身上**（无需另立类型） | 那条协议没有自己的请求类型（载荷不透明）⇒ 不为"转手"单造一个类型；信封那 8 字节由 `wire::put_address` 填 |
| 控制台的回信地址 | **只在 `Open` 上报** | 它是 per-session 不是 per-request：服务记进会话表，此后照表推 |
| 自检怎么"读回刚授出的那一枚" | **扫本任务表**（`Collect` + `Reserve`） | `Collect` 是唯一的枚举手段（`startup::moor` 同款）；`Port::to()` 那种访问器仍不立 |
| 生产路径上的裸 `accord` | **全部改走 `ship`**（28 处） | §3 的动机即此；`root::hand_over` 顺手泛型于 `AnyPie`——原先门铃（一枚 `Nole`）走的是 `PolePie::from_token`，那是个类型谎 |
| 自检里的裸 `accord` | **留守**（`shell` 的 `lend`/`cascade`/`spoof`/`name`） | 它们测的是**位表本身**：`accord` 是那个原语，换成语义层就把被测对象换掉了 |
| 握手那条"开一条会话"的请求 | **发一次，答复随握手回来** | 首帧既是握手帧又是开门请求，服务端建会话时就把它答了；让调用方"紧接着再发一遍"是**两条会话**——实测表每开一次漏一格，第 5 次表满 |
| 握手那一枚回执走哪条孔 | **本端为这次握手开的私有孔**（句柄经地址槽递出） | 请求孔是单槽信箱 + 两个方向的消费者：服务自己的请求循环会把回执吸回去（实测 50% 开不成）。也是这一格地址存在的唯一理由 |
| `Port::entry_token`（比对"会话还对不对得上实例"） | **删** | 零消费者；而入口号本来也不是身份——目录的 `Connect` 每次转授**一份新副本**，那个号会因为完全无关的原因变化。会话算不算数由机制回答（回信孔被封印 ⟺ 实例没了） |
| 开会话的握手住哪 | **`crates/protocol/src/console/session.rs`**（本轮搬） | 它全仓只有 console 一个消费者，且要懂 console 的帧（首帧的地址槽、认领号那一格）。§10 |
| 第二个构造入口叫什么 | **`borrow`**（用户裁决；原名 `adopt`） | 它说的正是与 `open` 的唯一差别——**孔的命挂谁身上**：`open` 自铸 ⇒ 命随本端；`borrow` 借来 ⇒ 命随对端（对端退场即封印）。`adopt` 语义也不错（"收下别人开的孔"）但**出族**、且与 `open` 不成对；`claim`（认领，与"认领号"同族）是次选；`borrow` 与 `core::borrow::Borrow` 的联想是它的代价（本仓有"撞名即否"的先例，见"命名 `Wire`"那一行）——裁决时认了这个代价 |
| 机制里的地址槽默认值 | **删**（本轮） | 四件（`ADDRESS_LEN`/`ADDRESS_AT`/`put_address`/`address_of`）各协议 wire.rs 各一份；机制只有"两枚孔配成一对"这一件事 |
| 启动期握手住哪 | **`crates/protocol/src/startup.rs`**（已搬） | 它就是一条协议——父域 ↔ 子域、线形定死、两条报文、每条孔单一发送者，与 `Refer`/`Referred` 是同一条理由。搬走之后 `core` 的 Mail 封装**恰好三件**（`port`/`dock`/`bell`），且 `runtime/src/lib.rs` 那句"本 crate 不认识任何协议"第一次是真的。`dock(child)` 同时改名 `berth(child)`，把 `dock` 这个词让给 Pole 的封装（[dock.md](dock.md)） |

## 8 · 已知边界

1. **读者死后写者往虚空推**：读者消亡只让它那一枚 `R` 消失，写者仍持副本 ⇒ 孔不死、
   无人读，写者继续 `push` 成功。（既有边界，本设计不引入也不治；席位也治不了。）
2. **`entry` 的所有权不在 `Port`**：`Port` 只借入。今天 `Service::disconnect` 释放 entry、
   `Console` 不释放——这是**策略**，不进结构。`Port::shut` 失败**不阻断** entry 释放
   （`disconnect` 里两件事各算各的，`closed?` 在 `released` 之后）。
3. **各协议的回信地址偏移不统一**：**这是有意的**（本轮改判）：偏移归协议，机制不再持默认值。
   uart/doom/irq/console 用 `1`，dispatch 用 `0`（帧首）——各有各的理由，见 §4.2。

3b. **四家还没有"对端没了"的判据**：`dispatch`/`doom`/`irq`/`uart` 都是一问一答，走
    `Port::open`（回信孔**自己**铸）⇒ 失败表现为有界的 `Busy`，「对端还没回」与「对端已经
    没了」在那四条路上仍不可区分。只有 console 走 `borrow`（回信孔由**对端**开）⇒ 服务被打死
    时它当场拿到 `Dead`。要不要让那四家也走那条路，是下一轮的题——本轮只把这条路的**代码**
    搬到了它唯一的消费者那里（§10）。
4. **`irq` 是另一种形状**：它每次调用新开一枚回信孔（`crates/protocol/src/irq/client.rs` 的
   `Line::ask`），而 console/dispatch 是"开一次、长期用"。前者对"迟到回复污染"免疫。
   每请求 `open`/`push`/`pull`/`shut` 一趟，envcall 数不变。旧版在收尾处 `seal`（让驱动的
   迟到 `push` 拿 `Dead`）；现在的收尾是 `shut`，而驱动每请求只推一条回执 ⇒ 迟到那条落在
   空槽里、下一请求已换新孔，不会把它挂住。
5. ~~**`Dock` 只占名，`Bell` 已另立**~~ —— **两件都已落**：`Bell` ↔ Nole 见 [bell.md](bell.md)
   （空载荷门铃——内核一位"有待取之事" + 听者面，`AnyPie` 不加变体）；`Dock` ↔ Pole 见
   [dock.md](dock.md)（借映 → 视图；起点与长度成对；`Shut` 不过存活闸）。三件至此各包一种
   primitive，`runtime::core` 里除任务本地原语外不再有第四件。
6. **`narrow` 与 `revoke` 闲置**：生产路径上**都是零调用者**了（`revoke` 原先那一处就是
   `Channel::close`，随旧类型一起摘掉；自检里 `cascade` 仍走一次）。ABI 保留
   （`Accord` 的唯一反向 / 权限代数的完整），账目单列。
7. ~~**15 行 port/ring 残留注释**~~ —— **已清**（本轮）：11 个内核文件里描述"已被 Pole 取代
   的共享内存 IPC"的注释都改说了当前的结构（`Pole` 的共享页视图）；`pole.rs` 那处
   `tag!(Ring, …)` 是唯一**还活着**的化石（Pole 的物理帧在分配器类目表里仍叫 ring），
   也一并改名为 `Kind::Pole`（读数里打成 `pole`）。
8. **判据**：见 §9（已立，三档都跑）。

9. **`Channel` 已删**，靠泊那半（今 `protocol::startup::berth`）也改走 `ship`：它仍是"开上
   行孔 + 授出 + 时序义务（必须早于 `Hatch`）"那三件事的归口，只是不再手写子集。

10. ~~**`Refer` / `Reserve` / `Referred` 住在 `runtime::core::handshake`**~~ —— **已搬**：
    它们是父域 ↔ 目录控制线程的报文（目录协议的语义），现住
    `crates/protocol/src/dispatch/control.rs`；`handshake.rs` 只剩 `Quay`/`Pier` 与
    `dock`/`moor`。`protocol → runtime` 那条单向边因此不再被反向借用。

    **上一轮把同一件事做完了**：`grant`/`borrow`（客户端 ↔ 服务端的会话握手报文）当初也住在
    `handshake.rs`，判据一模一样（那是 console 会话的语义），已搬进
    `crates/protocol/src/console/session.rs`。`handshake.rs` 294 → 185 行，只剩域构建那半。

    **这一轮连那半也搬走了**：`Quay` / `Pier` / `berth` / `moor` 是**父域与子域的协议**
    （线形定死、两个方向、单一发送者），整块搬进 `crates/protocol/src/startup.rs`；
    `runtime::core::handshake.rs` 删除。两件事同时成立：
    `runtime/src/lib.rs` 那句"**本 crate 不认识任何协议**"第一次是真的，而 `core` 剩下的
    封装恰好三件（`port` / `dock` / `bell`）。`dock(child)` 让出 `dock` 这个词给 Pole 的
    封装，改名 **`berth(child)`**（给泊位 ↔ `moor` 系泊，成对）。

11. ~~**`Port::adopt` 这个名字还没裁决**~~ —— **已裁决**：改为 [`Port::borrow`]，理由是它
    说的正是与 `open` 的唯一差别——**孔的命挂谁身上**（`open` 自铸 ⇒ 命随本端；`borrow`
    借来 ⇒ 命随对端，对端退场即封印）。同一次动作里 `console::session` 那个私有
    `fn borrow` 改名 `pull_grant`（名字让给了 `Port::borrow`，且它做的事就是"在那枚私有孔上
    拉一条 `Grant`"）。

## 9 · 判据（已实现，门里在跑）

三档（默认 / harden / 框架）都跑同一套：`scripts/examine.nu` 的 `MARKERS` 里三条与本设计
有关，**都只可能由本设计成立才能打出**。

### 9.1 · 授出与往返：`ship: cells=15 empty=1 source=1`

`ship` 命令（`programs/src/bin/user/shell.rs` 的 `ship_probe`），三个数各有各的牙：

| 读数 | 断言 | 牙（反向验证：去掉什么它会变） |
|---|---|---|
| `cells=15` | 十六格（`Access` 四取值 × `Policy` 四取值）里**十五格**授得出，且 `Collect` 读回的 `permission` 与该格的 `access \| policy` **逐格相等** | 把 `ship` 的 `subset = access \| policy` 改错一位（如漏掉 `VEST`），对应那一格读回不符 ⇒ 15 → 14 |
| `empty=1` | 第十六格（`Access::NONE + Policy::NONE`）**本地拒**（`Denied`，不发 envcall） | 去掉那条 `subset.is_empty()` 的早返 ⇒ 空集被送进 `Accord`，返回值不再是"本地拒" ⇒ 1 → 0 |
| `source=1` | `Port::pull` 收到**非 `to.peer()`** 推来的回复时返 `Denied` | 去掉来源校验 ⇒ 冒名那条被当成回复收下 ⇒ 1 → 0 |

第三条的布置值得记一笔：子线程开一枚孔（**它是那扇门的开辟者**）并把副本授给我 ⇒
`Port::open` 用 `Reserve(entry).owner` 认出的对端是**它**；随后**我自己**往自己的回信孔推
一条——内核盖的发送者是"我"而不是对端，故那一条必须被拒。自己推自己的孔在这里是**合法**
的（回信孔本就在我表里），它模拟的正是"别人往我的回信孔里塞一条"。
**这一格判据本轮一次都没改**：校验从 `take` 挪进 `pull`，牙还在同一处（自检里那一趟改成
`encode` → `push` → `pull`，见 `shell.rs` 的 `source_probe`）。

### 9.2 · 端到端：既有自检不许退

五家客户端全走了新路径（`Directory::open` → `Connect` → 自家的 `ask`（`push` + `pull`）→
校来源 → 回复）：

- `req echo` —— dispatch 的目录往返 + 服务往返（`Service::call` 那个信封）；
- `name` —— 注册 / 注销 / 非预约者被拒 / 死实例不锁名字；
- `spoof` —— 身份由内核盖章（它**故意**留在裸报文的层上：伪造回信地址正是它的被测对象，
  而 `Port` 的语义就是"写不出伪造的地址"）；
- `dir` / `kill echo` / `line …` —— 目录自省、他杀（doom 的 `Kill` → `Ack`）、中断线登记
  （irq 的每请求一趟）与 uart 的写往返。

### 9.3 · 地址槽偏移归**协议**（本轮把它彻底还给协议）

**当年**：`Port::call` 要把回信地址写进报文，就得知道**写在哪个偏移**，于是那个数收在
`runtime::core::port` 的一个模块常量里（`ADDRESS_AT = 1`，读作"紧跟首字段"），协议只能**覆盖**它。
而目录协议有**两张线形**：裸询问帧（客户端直接 `push` 进目录的请求孔）与服务调用载荷
（同一帧装进 `dispatch::Service` 的 64 字节信封，信封自己也要一格放回信地址）。
两张线形的帧字节必须**逐字相同**，否则同一次注册换条路就变味——实测的形状正是如此：
同一个名字一条路注册成功、另一条路 `Denied`，引导停在 `plic` 那里。

**当时的裁决**：偏移由协议声明（`Duet::ADDRESS_AT`），机制侧只提供**带偏移的口**
（`put_address_at` / `address_at`）。据此目录协议把地址槽声明在**帧首**
（`wire::ADDRESS_AT = 0`）——信封那一格只能落在载荷最前，帧首因此是唯一两边都成立的位置；
帧形随之定为 `[地址槽 0..8)][op 8][entry 9..17)[名字 17..帧尾]`，`CAP = 48`。

**本轮改判**：那半途的"机制给默认值、协议覆盖"也销掉了——四件（`ADDRESS_LEN` /
`ADDRESS_AT` / `put_address` / `address_of`）各协议 wire.rs **各一份**，机制侧一件不留。
判据是**调用点不再从两个 crate 各拿一半知识**：`echo.rs` 曾经一边取
`dispatch::wire::ADDRESS_AT`（协议的偏移）、一边取 `runtime::core::port::{ADDRESS_LEN, address_at}`
（机制的长度与取数），两半各在一处。现在它是 `dispatch::wire::address_of(msg)` 一句话。
帧形本身**一个字都没改**（`cells`/`spoof`/`req echo` 照旧）。

**收侧同一条裁决的另一半**（仍未变）：`Directory::serve` 收的必须是**带真实长度的那一段帧**。
载荷缓冲按上界给、尾部有多余的零，长度不钉住那些零就会被算进名字里。

**记录在案的教训**：这一处我从"读代码推布局"起步，连试三轮都回到同一个错处，而没有先把
**一帧的真实字节**打出来对账。下一处同类问题的第一根探针应当是"把 `wire[..n]` 的字节打出来"，
而不是再读一遍偏移常量。

### 9.4 · §5 的判据：`hole: len short=1 long=1 nofit=1`

`hole` 自检多打的一行，门断言它：


  - `short` —— 1 字节的消息进得去出得来；
  - `long` —— **同一条孔**再装 600 字节也进得去出得来（孔不再按 unseal 时的 mtu 预分配）；
  - `nofit` —— 缓冲装不下时 `pull` 被拒、**槽一个字节都不动**（`peek` 仍报 600，换够大的
    缓冲仍取得回整条）。

### 9.5 · 会话：`console: session opens=12 ok=12 closed=1`

`session` 命令（`programs/src/bin/user/shell.rs`）——**同一推者连开 12 条会话，逐条核回执**，
并关掉被顶替的那一条。三个数各有各的牙：

| 读数 | 断言 | 牙（反向实跑） |
|---|---|---|
| `ok=12` | 「开一条会话 = 一次往返」且**表不随重开增长**（重开即换推者那一格） | 把"重开复用那一格"去掉 ⇒ 实测 `ok` 从 12 变 **7**（表宽 8）；旧形状（开一次占两格）第 5 次就没 |
| `closed=1` | `Close` 往返通、且**先取孔再清槽** | 清槽后再取孔 ⇒ 那句 `Ok` 丢掉、客户端等满上界 ⇒ `closed=0`（自检第一次跑就是 0） |

**实跑**（本轮落地后）：`EXAMINE_HARDEN=1 nu scripts/examine.nu` → **5/5**——默认档 3 轮
（16 步 / 22 marker）、harden 档（20 步 / 27）、框架档（20 步 / 28），三档都含上面四条读数。
**分层那一轮（§10）落完之后重跑，仍是 5/5，一条 marker 没动。**

## 10 · 分层裁决：往返与帧格式归协议（本轮）

### 10.1 · 判据

**一条需求该不该立一层，看它是不是"多个消费者共用、又不属于其中任何一个"的知识。**
拿它量本层原来的四件：

| 原来住在 `port.rs` 的 | 消费者 | 判 |
|---|---|---|
| `Access` / `Policy` / `To` / `ship` | **22 处、12 个文件、3 个 crate**（其中 11 处在 `programs/supervisor/*`：建域与配给的授出，与 Hole 无关） | **留**：跨三层共用，且与资源种类无关（`AnyPie`） |
| `Port` 的字段（两枚孔 + 对端坐标） | 6 处，全在 `crates/protocol` | **留**：这是"配对"本身 |
| 一次往返的**时序**（编帧 / 一问一答 / 解帧） | 同上 6 处 | **走**：它要用帧格式，而帧格式是各协议自己的 |
| `Duet`（10 个关联项）+ 地址槽 4 件 + 模块级默认偏移 | 同上 6 处 | **走**：它们存在只为"让一个不知道帧的通用件能用"——通用件一走，它们一并销 |
| `dial` + 认领号 + 私有握手孔 + `handshake::{grant,borrow}` | **1 处**（`console/client.rs`） | **走**：为一个消费者的需求做成了通用机制 |

### 10.2 · 落到哪儿

| 搬走的 | 新住处 |
|---|---|
| 一次往返（`ask`，每家 5 行） | `crates/protocol/src/{uart,doom,irq,dispatch,console}/client.rs` |
| 帧的编解码与地址槽 | 各协议自己的 `wire.rs`（`Query::encode(at)` / `Reply::decode`） |
| 开会话的握手（原来叫 `dial` / `grant` / `borrow`） | `crates/protocol/src/console/session.rs`（**新文件**） |

### 10.3 · 净变化

`port.rs` **392 → 227 行**（删 `call`/`take`/`dial`/`close`/`Duet` + 地址槽 4 件；
加 `seed`/`push`/`pull`/`shut`）；`handshake.rs` **294 → 185**（`grant`/`borrow` 搬走）；
新增 `console/session.rs` 164 行。**全仓净 −336 行**（20 个文件）。
envcall 序列与从前**逐字相同**，ABI 一字未动。

### 10.4 · 为什么"两家各写一遍"不是损失

旧账是"五处都漏了**核对来源**"。那条**漏不掉**了——核对沉在 `Port::pull` 里，调用方绕不过去。
剩下的复制成本是每协议 5 行 `ask`，换掉的是：一套 10 项的 trait、一份机制里的偏移默认值、
以及 `port.rs` 上那 100 行会话/握手代码。**"五家各写一遍"与"机制替它们写一遍"之间，
这一轮选了前者，因为后者要求机制知道它不该知道的东西（帧长什么样）。**

### 10.5 · 被否项

| 否掉的 | 理由 |
|---|---|
| 把 `Duet` 留在协议层当共用件（`protocol::port`） | 一个通用的"发一帧收一帧"必须让协议**把自己的帧描述给它** ⇒ `Duet` 原样长回来，只是换了一层住 |
| `Access`/`Policy`/`To`/`ship` 搬进 `crates/protocol` | **否**——但**当年那条理由已过期**：原话是"`handshake::dock`（runtime 内）要 `ship` ⇒ 会造出反向依赖"，而那一块已整块搬去 `protocol::startup`。结论仍站得住，靠的是 §10.1 的**正面判据**：22 处 / 12 文件 / 3 crate 共用（其中 11 处在 `programs/supervisor/*`：建域与配给的授出，与 Hole 无关），且与资源种类无关（`AnyPie`）；再加一条——`runtime::core::port` 自己（`Port::open`）就是它的消费者。**故这不是"搬不动"，是"不该搬"**，记在此处以免下一个人拿过期的理由当依据 |
| 同上搬进 `crates/env` | 裁决账已否（env 是内核也依赖的 ABI crate） |
| 保留 `dial` 作为机制的第二个开场 | 它只有 console 一个消费者，且要懂 console 的帧（地址槽装在私有握手孔上、认领号在 `NONCE_AT`） |
| 让各协议各写一份"8 字节 LE token"编解码 | 没做——但**没有另立共用模块**：四件各协议各一份，是"帧格式归协议"的直接代价（见 §9.3） |
