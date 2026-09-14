# port — Hole 的通讯协议（Access · Policy · To · ship · Port）

> 路径约定：`文件:行` 相对仓根。权柄模型在 [pie.md](pie.md)，数据面三件套在 [mail.md](mail.md)，
> 服务目录协议在 [dispatch.md](dispatch.md)，空载荷门铃在 [bell.md](bell.md)。
>
> **状态：§5（Hole 的动态长度）已实现**，判据见 §9 末段；`ship` / `To` / `Port` / `Duet`
> （§2–§4、§6）设计已裁、**未实现**。

## 1 · 语义定位

`Port` 不是内核对象，是**镜像侧的机制**：把一枚 Hole 门闩的一次往返收成一个类型。
内核侧**零改动**（除 §5 的消息长度），ABI 除 §5 外零改动。

| 管 | 不管 |
|---|---|
| 授出（`ship`）、对端坐标（`To`）、一次往返（`Port`）、报文对的形状（`Duet`） | 报文的布局与语义（在 `crates/protocol`）、权柄的判定与级联（在内核 `gate`）、槽本身（在内核 `mail`） |

**与 `Channel` 的关系**：`Channel` 是本设计的**前身**，它留下三条教训、两条病根：

| | `Channel` | `Port` |
|---|---|---|
| 坐标 | 只存 `at_peer`（号） | `To { who, token }`——**两半成对**（号只在那一张表里有意义） |
| 收尾 | `revoke(peer, at_peer)?` 后 `release()` | `close` 只 `release()`——**级联已含对端那枚**（`gate/cull.rs` 沿 `sire` BFS 反查全世界） |
| 句柄 | `mine()` 交出整枚门闩（可 `accord` 出永不撤的副本） | 协议**拿不到句柄**：只给 `Duet` 的两个纯函数 |
| 往返 | 每个协议各写一遍 | `Port::call` 一处 |

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
pub fn ship<P: AnyPie>(pie: &P, who: TaskId, access: Access, policy: Policy) -> EnvResult<To> {
    let subset = access.bits() | policy.bits();
    if subset.is_empty() { return Err(denied()); }   // 空集本地拒，不发 envcall
    let token = pie.accord(who, subset)?;
    Ok(To::new(who, token))
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

## 4 · 一次往返：`Port` + `Duet`

```rust
pub struct Port { to: To, entry: HolePie, reply: HolePie }

pub trait Duet {
    type Req; type Rep;
    const REQ: usize; const REP: usize;
    fn encode(req: &Self::Req, to: PieToken, out: &mut [u8]);   // 布局归协议：地址写哪它说了算
    fn decode(buf: &[u8]) -> EnvResult<Self::Rep>;
}

impl Port {
    pub fn open(entry: &HolePie) -> EnvResult<Port>;                    // Reserve(owner) + unseal + ship(WRITE, NONE)
    pub fn call<W: Duet>(&self, req: &W::Req, within: usize) -> EnvResult<W::Rep>;
    pub fn close(self) -> EnvResult<()>;                                // reply.release()；entry 不碰
}
```

`call` 的五步归口（**今天五个客户端各写一遍，且五处都漏了第三步**）：

| 步 | 谁写 | 今天 |
|---|---|---|
| 把回信地址填进报文 | `Port::call` → `W::encode` | 五家各自手戳 `msg[REPLY_AT..]` |
| `push`（满则等 = 背压） | `Port::call` | 五家 |
| **校验回复来源**（`pull_from` vs `to.who()`） | `Port::call` | **一处都没有**（`pull_from` 的调用者全在服务侧与自检里） |
| 有界等待 | `Port::call`（`within`，`usize::MAX` = 永久） | 五家各带自己的常量 |
| 解码 | `W::decode` | 五家 |

**命中不了来源 ⇒ `Denied`，该会话应弃用**（迟到的真回复仍可能落槽污染下一次 `call`——
与 `HolePie::pull_timeout` 的既有契约同款，`crates/runtime/src/env/mail.rs:335-336`）。

**`type Req` / `type Rep` 成对绑定**是选 trait 而非闭包的理由：闭包版可以把 A 协议的
`encode` 和 B 协议的 `decode` 一起递进去，编译器不响。trait 版配错不可表达。

**`to` 不跨进 `protocol`**：`encode` 收的是裸 `PieToken`，`who` 只被 `call` 用来校验来源。
所以 `protocol` 层看不到 `To`——`Port::to()` 这类访问器**不需要存在**（零调用者的访问器不留，
`programs/src/uart.rs:63-65` 有先例）。

## 5 · 消息长度：舍弃 `mtu`

**动机**：`UnsealHole { mtu }` 把"这条孔上的消息最大多长"钉死在开孔时刻，而那个数其实是
**协议自己的报文尺寸**（handshake 9、console 64、doom 41、irq 49、uart 64）——协议知道，
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
| `To` 的两半成对 | 号在错的表里查（`Channel` 的病根） | 类型：`To { who, token }` 私有字段，只能由 `ship` 造 |
| 混族不可表达 | 把 `CAGE` 写进读写位 | 类型：`Access` 与 `Policy` 是两个类型 |
| 空集不成立 | 授出一枚什么都没有的枚 | `ship` 本地拒（不发 envcall） |
| 回信地址与来源校验成对 | 收到别人的回复而不自知 | `Port::call`（今天五处都没做） |
| 收尾只 `release` 一次 | 冗余的 `revoke` 失败会短路后续清理 | `Port::close`（级联已含对端副本） |

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
| `Port` 只留 `send`/`recv`（往返归协议） | **否** | 往返的契约（有界、来源校验）不是协议语义；搬上去等于把 `Channel` 重造一遍 |
| `ReadWrite` 作通信角色 | **否** | 两个读者；只留给自环与自检 |
| `Access`/`Policy`/`To` 进 `crates/env` | **否** | 那是内核也依赖的 ABI crate——`protocol` 从 `env` 拆出来的先例记着："284 行内核永远读不到的协议" |
| 命名 `Wire` | **否** | 与 `crates/env/src/wire/mod.rs` 的 `trait Wire`（字段 ↔ usize）撞名；改 `Duet` |
| 命名 `Ferry` / `at` / `give` | **改** | 定为 `Port` / `To` / `ship`（同族：`dock` `moor` `Quay` `Pier`） |

## 8 · 已知边界

1. **读者死后写者往虚空推**：读者消亡只让它那一枚 `R` 消失，写者仍持副本 ⇒ 孔不死、
   无人读，写者继续 `push` 成功。（既有边界，本设计不引入也不治；席位也治不了。）
2. **`entry` 的所有权不在 `Port`**：`Port` 只借入。今天 `Service::disconnect` 释放 entry、
   `Console` 不释放——这是**策略**，不进结构。`Port::close` 失败**不阻断** entry 释放
   （今天 `channel.close(self.owner)?` 的 `?` 会一起跳过）。
3. **五个协议的回信地址偏移不统一**（dispatch `[49..57]`、console `[8..16]`、doom `[33..41]`、
   irq `[9..17]`、uart `[8..16]`）：本轮保留。要不要统一是协议层的事。
4. **`irq` 是另一种形状**：它每次调用新开一枚回信孔（`crates/protocol/src/irq/client.rs:75-82`），
   而 console/dispatch 是"开一次、长期用"。前者对"迟到回复污染"免疫。走 `Port` 后它成为
   `open`/`call`/`close` 每条一趟（envcall 数不变，每请求多两行）。
5. **`Dock` 只占名，`Bell` 已另立**：`Dock` ↔ Pole（共享内存，无背压、同步在契约外）仍是空位；
   `Bell` ↔ Nole 已单独裁决成文（[bell.md](bell.md)：空载荷门铃——内核一位"有待取之事" +
   听者面，`AnyPie` 不加变体）。本轮只做 Hole。
6. **`narrow` 与 `revoke` 闲置**：生产路径上分别零调用者与仅 `Port::close` 一处
   （后者本轮摘掉）。ABI 保留（`Accord` 的唯一反向 / 权限代数的完整），账目单列。
7. **15 行 port/ring 残留注释**：`work/mod.rs:7` 等 11 个文件在描述一个已被 Pole 取代的
   共享内存 IPC（`DockMeta`/`RingMeta` 代码已删）。`kernel/src/work/mail/pole.rs:75` 的
   `tag!(Ring, …)` 是**唯一还活着的化石**（Pole 的物理帧仍打这个标签）。
8. **判据未立**：见 §9。

## 9 · 判据与验证（实现后）

- **端到端**：`req echo`（五家走新路径：`Directory::open` → `Connect` → `Port::call` → 回复）
  与 `name`（注册 / 注销 / 非预约者被拒）、`spoof`（身份盖章）三条既有自检不许退。
- **新机制**：一条 `ship` 自检——对 16 格各授一枚、用 `Collect` 读回 `permission` 断言
  （给自己授一枚即可读回位，`programs/src/bin/user/shell.rs` 的 `lend` 自检同款手法）；
  另断言两处：`ship(Access::NONE + Policy::NONE)` 本地拒（不发 envcall）、
  `Port::call` 收到非 `to.who()` 的回复返 `Denied`。
- **§5 已落地的判据**：`hole` 自检多打一行 `hole: len short=1 long=1 nofit=1`，门断言它
  （`scripts/examine.nu` 的 `MARKERS`）：
  - `short` —— 1 字节的消息进得去出得来；
  - `long` —— **同一条孔**再装 600 字节也进得去出得来（孔不再按 unseal 时的 mtu 预分配）；
  - `nofit` —— 缓冲装不下时 `pull` 被拒、**槽一个字节都不动**（`peek` 仍报 600，换够大的
    缓冲仍取得回整条）。
