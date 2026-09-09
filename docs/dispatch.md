# dispatch — 服务目录协议（Service Directory）

> 目录是一个**普通 Service**，不是内核特殊机制。它只有一个 req hole；Register /
> Unregister / Replace / Resolve / Enumerate / Connect 全是该 hole 上的消息。
> 内核面没有它的入口调用（原 `ServiceCall` / class 7 已删除）。

## 1 · 语义定位

```text
                 dispatch（服务目录）
                     │
        ┌────────────┼────────────┐
        │            │            │
     Resolve     Enumerate     Connect
        │            │            │
     「有/无」    「有哪些」    「把入口门闩给我」
```

目录**不负责**：启动服务、停止服务、调用服务、执行 operation、解释业务载荷。

**Disconnect 不在协议里**：它不改绑定表，只改调用方自己的权限表——它是调用方的
自释原语 `MailCall::Release`，与目录无关。

## 2 · 操作集（6 个，封闭）

受管对象只有一张**绑定表** `name → 接口`；对它的原子动作就是这 6 个：

| 操作 | 谁 | 读 | 产 | 失败 |
|---|---|---|---|---|
| `Register` | provider | name、入口门闩 | 绑定 +1 | `Taken`（名字已占）/ `Denied` |
| `Unregister` | provider | name、发起者 | 绑定 −1 | `NotFound` / `Denied`（非发布者） |
| `Replace` | provider | name、新门闩、发起者 | 绑定改写 | `NotFound` / `Denied` |
| `Resolve` | client | name | `Found` / `NotFound` | — |
| `Enumerate` | client | 游标 | 一页名字 | — |
| `Connect` | client | name | **调用方权限表 +1** | `NotFound` / `Denied` |

发起者身份 = 入口门闩的 `vestor`（`Pie` 既有字段），**不新增字段**。
`Unregister` / `Replace` 只管名字；已发出的权限靠资源死亡自然失效（见 §6）。

## 3 · 结构

```rust
pub struct Name { bytes: [u8; 32] }          // 定长、尾随 NUL、内容非空且不含 NUL
pub struct Binding { name: Name, entry: usize, owner: usize }  // entry = 入口门闩 token
pub struct Directory { bindings: Vec<Binding> }              // 名字唯一
pub enum DirectoryError { Taken, Unknown, NotOwner, NotGrantable }
```

目录**跑在 S 态域程序 `task-dir` 里**（`task/src/core/directory.rs` + `bin/supervisor/dir.rs`）：
内核不含它的任何代码。绑定里存的是入口门闩的 **token**——门闩一直留在目录自己的
权限表里，`Connect` 用 `mail::accord` 转授子集；`owner` 取该 token 的 `vestor`。

**接口 = 一份入口门闩**（不是 req/rep 两份）：目录只知道「服务有一个入口」，
不知道服务内部怎么执行。回信通道由**调用方自带**——与目录协议自身同构。

不变量做成义务：名字唯一（`bind` 返 `Taken`）、绑定必持门闩（token 必在目录权限
表里且带授与人）、非法名不可表达（`Name` 只能由 `new` 造出）。

## 4 · 线格式（64 字节，沿用现有 hole）

```text
请求
[0]      op      u8      1=Register 2=Unregister 3=Replace 4=Resolve 5=Enumerate 6=Connect
[1..33]  name    [u8;32] 目标名字 / Enumerate 游标（全 0 = 从头开始）
[33..41] entry   usize LE  Register/Replace：入口门闩的目录侧 pie token
[41..49] 保留    u64 LE  必须为 0
[49..57] reply   usize LE  调用方自带的回信 pie 的目录侧 token（0 = 无回复预期）
[57..64] 保留（0）

回复
[0]      status  u8      0=Ok 1=Found 2=Connected 3=NotFound 4=Denied 5=Taken
[1..33]  name    [u8;32] Found：Resolve 命中的名字 / Enumerate 的一页
[1..9]   entry   usize LE  Connected：目录转授给调用方的入口门闩 token
[9..17]  保留    u64 LE  必须为 0（原 owner 字段已删——见 §5）
```

`Enumerate` 按名字**排序**分页（HashMap 迭代顺序无保证，顺序必须是契约）：
`after` 之后的第一条，`NotFound` 即到头。

## 5 · 身份与授权

```text
调用方 ──启动期握手（Pier）──▶ 目录请求门闩（**dir 亲授**；Owned 取回 owner = 目录 task id）
调用方 ──UnsealHole + Accord──▶ 目录：回信 hole 的对端 token（写进 [49..57]）
目录   ──gate::accord──▶ 调用方权限表（子集 R|W）
服务 id = 入口门闩的 owner（资源开辟者，服务自己 UnsealHole 出来的）
```

- **身份不来自消息体**：目录按请求取「`[49..57]` 那枚回信 pie 的 `vestor`」——
  内核在 `Accord` 时赋值，消息体伪造不了。没带有效回信 pie 即无身份（`caller = 0`）：
  `Register` 不看身份，`Unregister`/`Replace`/`Connect` 一律拒绝。
- **反方向用 `owner` 而不是 `vestor`**：调用方认服务时，门闩可能经手多次（root 分发、
  目录转授），`vestor` 每次都会改写成中间人；`owner` 挂在资源上，任意副本同值。
- **硬规则：服务必须自开入口 hole**（`UnsealHole` 自己那份）。若由他人代开，
  `Owned(entry).owner` 指向代开者，调用方会把回信 hole 授给错的人。
- **授权只用已有原语**：`Connect` 就是 `gate::accord` 转授子集；注册资格就是
  「能把门闩交出来」——不需要新的 capability 类型。
- 授权链 `service →(VEST) 目录 →(R|W) 调用方`；目录不带 BACK，故可自由代授。

## 6 · 存活级联（Unregister 为什么不用回收权限）

服务死 → 其 hole 的 `Arc` 全部 drop → `HoleMeta::drop` 置 Dead → 所有下游 pie 的
`Weak` 失败 → 操作返回 `Dead`。**存活级联是自动的**，所以 `Unregister` 只管名字；
目录不需要维护「连接实例」。

## 7 · 原语

目录的 6 个操作**一个都不对应内核原语**——它们是协议消息。落到底层只用已有原语
（`Accord` / `Revoke` / `Push` / `Pull` / `UnsealHole` / `Narrow`）。

新增的两个原语与目录无关，补的是权限模型自身的洞：

| 原语 | 一句话 | 补的洞 |
|---|---|---|
| `MailCall::Collect { index } -> (PieToken, Permission, TaskId)` | 报出我持有的第 index 份（含其 `vestor`）——**唯一的枚举手段** | 用户态此前**无法自省自己的权限表** |
| `MailCall::Owned { token } -> (TaskId, TaskId)` | 我持有的这枚门闩：`vestor`（谁授的）+ `owner`（资源谁开的） | 此前**只能看门闩的来历，看不到资源的来历**（转手即丢） |
| `MailCall::Release { token }` | 放下我自己的一份**及其全部后代**（Pole 同步 unmap） | 此前**没有任何自释路径**（`revoke` 只允许授与人收回） |

配对：`Unseal*` ↔ `Seal`（动资源）；`Accord` ↔ `Revoke`（他人）；`Collect`（枚举）↔
`Owned`（查证）↔ `Release`（放下，自己）。

### 7.1 派生关系与级联撤销

**能力面只存一条边**：每枚门闩记 `sire`（父门闩的 token；`None` = 原始自持）。
向上的「授与人」与向下的「子门闩」都不存，而是同一张**全世界任务快照**上的查询：

| 查询 | 一句话 |
|---|---|
| `gate::vestor(pie, snap)` | 授与人 = `sire` 所在任务的 id（原始自持 → 无） |
| `gate::heirs(token, snap)` | 子门闩 = `sire == token` 的那些（持有者 + 子 token） |
| `gate::vestable(pie, dst, snap)` | BACK 守门：带 BACK 只能授回 `sire` 的持有者 |

快照由 `scheduler::core::snap()` 提供、boot 经 `gate::install` 注入——**gate 不依赖
scheduler**（依赖倒置）。`gate` 与 `envcall` 的分工：核心收算法、适配层拍快照。

| 原语 | 一句话 | 说明 |
|---|---|---|
| `gate::cull(task, token, snap)` | 摘掉一枚及其**全部后代**（沿 `sire` 反查闭包） | 与结构面 `messenger::cull` 同构：那边沿 `heir`、这边沿 `sire` |
| `gate::revoke` | 鉴权（该枚的 `sire` 在我表里）→ `cull` | 语义由「一枚」加深为「子树」 |
| `gate::release` | 定位 → `cull` | 「放下这份，以及经它授出的一切」 |
| `gate::doom(tid)` | 退出钩子：任务名下每枚门闩各自 `cull` | 与 `messenger::doom` 同表（`boot.rs` 的 `EXIT_HOOKS`） |

**`Revoke` 的鉴权与 `Spawn` 是同一句话**：能在我表里查到它的 `sire` ⇒ 我是 sire ⇒
我能撤销。（与旧的 `vestor == me` 等价——pie 只能复制、不能转移——但本地可判，
不吃快照。）

**`Release` 级联**换来「父在则子在」：一枚门闩只能连同它的子树一起消失，故
`sire` 永不悬空，也**不需要** seL4 的 `RevokeFirst`。

**ABI 零变更**：`Accord` / `Revoke` / `Release` / `Collect` / `Owned` 的形状与线上
字节都不动；`sire` / `cull` / `doom` 全是内核内部。

**在途语义**：撤销只作用于能力、不作用于在途数据——已 push 进 hole 槽的消息，
对侧仍可取（与 Solaris `door_revoke` 的「进行中的调用允许完成」一致）。

### 7.2 资源寿命与封印

**资源寿命 = 能力寿命**：门闩持资源实体的**唯一强引用**（`Pie.meta: Arc<M>`），
最后一份门闩消失即回收。因此**没有全局资源表**（原 `memo` 模块已删）：

| `memo` 的职责 | 之后 |
|---|---|
| 发资源号 | 搬进 `hole.rs`（只剩 `HoleMeta` 的**等待键身份**要它） |
| 注册 / 移除（保活） | 消失——`Arc` 就是保活，引用归零即回收 |
| 按 id 找对象 | 消失——唯一消费者 `Seal` 用调用方自己那枚门闩的 `Arc` |

由此三条性质自动成立：**撤销/放下/任务消亡都不再泄漏**（`cull` 摘掉最后一份即
回收）；**开辟者消亡 → 它开的资源随之回收**（`doom` 摘其门闩 → 引用归零）；
`PoleMeta::drop` 自己撤映射、还物理帧，`HoleMeta::drop` 自己唤醒等待者。

**封印只归开辟者**（`MailCall::Seal` 的鉴权 = `meta.owner() == caller`，O(1)）：

- 他人 `Seal` → `Denied`；已封印再 `Seal` → `Dead`。
- 主人 `Seal` 只**置死 + 唤醒**，不回收——内存由引用归零回收。于是主人有两档：
  `Seal`（立即失效，他人拿到 `Dead`）／`Release`（放下并级联，他人拿到 `Denied`）。
- `Dead`（-2）因此**只**由 `Seal` 产生。

**门闩必须在锁外 drop**：最后一份 drop 会跑 `Meta::drop`（唤醒 / 撤映射 / 还帧），
在 `Task.pies`（L3）锁内 drop 即 3→3 嵌套。`gate::cull::take` 的返回值因此由调用方
在锁外释放。

**快照是唯一的全局视图**：`snap`（`[Weak<Task>]`）+ 一条 `sire` 边，替代了原来的
`memo` 表。资源表若需要，也从 `snap` 派生（遍历各任务的门闩、按 `meta` 去重），
不新开结构。

### 7.3 发送者盖章（Hole 消息带来源）

**问题**：目录原先按请求体 `[49..57]` 的回信 token 求 `vestor` 认人。内核没撒谎
（`Owned` 如实回答「这枚门闩谁授给我的」），但协议信了**一段可猜的整数**——pie
token 是全局连续小整数，猜中别人的 token 即可冒充其身份（可利用面：`Unregister`
解绑、`Replace` 换绑别人的名字）。

**修法**：身份改由**内核盖章**——`Push` 时内核把推者的 task id 与消息**同锁同写**
进槽，`Pull` 一并交回收方：

| 位置 | 变化 |
|---|---|
| `MailCall::Pull` | 返回 `(实际长度, 发送者 TaskId)`（a0 仍是长度、a1 是发送者） |
| `HolePie::pull_from` | 用户态新增；`pull` 保持返长度（丢弃发送者） |
| `HoleMeta.slot` | 从 `Vec<u8>` 变成 `{ buf, from }`——来源与消息同一次写入 |

目录据此：`caller = pull 回来的 sender`；回信地址仍取报文里的 token，但**必须是该
sender 授给目录的那一枚**（`Owned(reply).vestor == caller`），否则丢弃回复——防
「替他人收信」。

**为什么不是「不可猜的 token」**：那只是把猜中概率降低，且需要内核秘密与真熵源；
而身份本就不该来自报文——它应来自**不可伪造的 syscall 上下文**（与 `sire`/`owner`
由内核在 `Accord`/`Unseal` 时赋值是同一条原则）。

## 8 · 引导（启动期握手 + 根转达）

内核是根授予的源头；**root 域**（`bin/supervisor/root`）负责产生所有子域，机制不变
（父任务把权限交给子任务），但顺序与过去不同：

```text
root:
  1. 逐子域串行：dock(child)（开上行孔 mtu=9 + Accord(child, R|W|VEST)）
     → Build + Spawn(Held) → Hatch
     → Quay::pull（子域控制孔在父侧的句柄；校验 Owned(句柄).vestor == child）
  2. 客户端要目录能力时：Refer{who} → dir 控制孔；Referred{token} ← dir 上行孔
     → Pier{token} → 子域控制孔
  ※ root 全程不持任何服务孔（见 docs/root.md §10）

dir:   moor() 认上行孔 → UnsealHole 自建请求门闩 H（**只自己持**）
       → UnsealHole 自建控制孔 C → Accord(root, R|W) → Quay{C 在父侧的句柄}
       → Spawn 控制线程（Held）→ Accord(H/C/上行孔 三枚副本给它) → Hatch
       → 主线程服务循环；控制线程 pull(C) → H.accord(who, R|W) → Referred
echo:  moor() → UnsealHole 自建控制孔 → Accord(root, R|W) → Quay{句柄}
       → UnsealHole 自建入口门闩 → Pier::pull → dir_id = Owned(门闩).owner
       → Accord(entry, dir_id, R|W|VEST) → UnsealHole 自造回信 hole + Accord(dir_id, R|W)
       → Register（回信 token 写 [49..57]）
shell: moor() → UnsealHole 自建控制孔 → Accord(root, R|W) → Quay{句柄}
       → Pier::pull → Directory::open(门闩)
```

- **没有任何整数身份进报文或启动参数**：目录 id 由 `Owned(门闩).owner` 从资源事实推出。
- **子域启动参数为空**；`Spawn` 的 args 只剩内核给 root 的清单视图。
- **控制孔与服务孔必须分两条孔**：Hole 是单槽信箱，同一条孔上既 push 又 pull 会把自己
  刚写的消息读回来（实测死锁，见 `docs/root.md` §6.4）；B 之后更要求「父域拿不到服务孔」。
- **dir 有两个线程**：请求孔 `H` 与引入孔 `C` 必须同时有人听（`Wait` 一次只等一条孔），
  故控制面单开一个线程——见 `docs/root.md` §10。

## 9 · 已决 / 被否

| 决策 | 定论 |
|---|---|
| `ServiceCall`（class 7） | **删除**——它是入口策略，不是原语 |
| 根授予 | 逐级委托（`Accord`）+ 启动期握手（`Quay`/`Pier`）+ 自省（`Collect`/`Owned`）；不做公开 id |
| 接口形态 | 一份入口门闩（A3）；回信由调用方自带 |
| 一个名字几个 provider | 1 个 |
| 名字权限 | v1 不做管理接口；`Register` 资格 = 持有门闩 |
| Disconnect | 不进协议 = `Release` |
| 目录状态 | 无连接实例；Unregister 只管名字 |

## 10 · 已知边界

1. ~~**回信 pie 的 token 可猜**~~ —— **已修，见 §7.3**：身份不再来自报文，改由内核在
   `Push` 时盖章的发送者决定；回信地址还加了「必须由该发送者授出」的一致性检查。
2. **名字可抢注**：v1 没有名字权限；任何能造门闩的任务都能注册新名字。
3. **Unregister/Replace 已实现但不在常规演示里**：echo 自注册路径已由「注册后立刻
   自注销」临时验证（身份取 echo 自身，返回 Ok）；v1 shell 没有对应命令，故不常驻。
4. `Enumerate` 一次一个名字（64 字节装不下列表）。
5. **`Join` 可能在退出钩子跑完之前返回**：目标一旦置 `Reaped`，`target_dead` 即为
   真、当场返回；而 `clear_loop` 的钩子（`messenger::doom` / `gate::doom`）可能在
   它之后才执行。语义上「目标已退出」没错（钩子是清理路径、不阻塞退出），但
   **「join 返回 ⇒ 收尾已完成」不成立**。依赖收尾完成的调用方需有界等待
   （`shell` 的 `cascade`/`reclaim` 自检即如此）。

## 11 · 实现中发现并修复的内核缺陷（与本协议无关，但拦住过验证）

1. **`__utrap` 保存顺序错**（`kernel/src/runtime/switcher/trampoline.rs`）：
   `csrr t0, sscratch` 排在 `sd x5, 0x58(sp)` 之前，把用户 `t0` 就地覆盖成用户
   `sp`，再当「用户 t0」存进帧——**每次用户陷阱返回后 t0 都是栈地址**。表现为
   `pc=0x4`、栈数据当返回地址、`a7` 变成 `0x20CC0` 一类随机崩溃（基线 `req` 同样
   3/3 崩）。修法：先存 x5，再用 t0 做 scratch。
2. **纯 `.bss` 段装载被拒**（`kernel/src/work/unit/loader.rs`）：
   `filesz = 0` 的 `PT_LOAD`（链接脚本让 `.data`/`.bss` 各自成段，合法且常见）
   被 `attach_map` 的 `pages == 0` 判为 `NotAligned`，整块装载失败。修法：无文件
   实体时跳过 `attach_map`，整段走懒登记。
3. **`warpper` 内联后返回值错**（`crates/env/src/ucall.rs`）：
   asm 块被内联进调用方时，调用方读回的 a0 恒 0；独立函数调用则正确。修法：
   `#[inline(never)]`（与仓库对裸 asm 的一贯纪律同源，见 `docs/ipc.md` §13.10 A.2）。
4. **hole 等待键取堆地址 → 陈旧唤醒闩被继承**（`kernel/src/work/mail/hole.rs`
   `key()` + `task/src/env/mail.rs` `pull_timeout()`）：`wait_sites` 的站点从不回收，
   而 `messenger::wake` 在「无等待者」时置 `pend = true`；成功走**裸 pull** 的调用方
   不会消费这个 pend，于是它一直留着。原键是 `HoleMeta` 的**堆地址**（会被分配器
   回收再利用），死 hole 的陈旧 pend 因此可能被落在同一地址的新 hole 继承；即便不
   继承，同一 hole 上也会出现「下一次 `wait` 立即返回『已唤醒』但槽是空的」。
   客户端 `pull_timeout` 把 `wait == false` 当成超时结论，于是**第二个请求偶发
   立刻报 `Busy`**（约 1/3 运行；目录侧其实正常，回复随后才落进槽里）。修法两处：
   ① 键改用 `ResourceId`（单调、永不复用）+ `(id << 1) | 方向位` 编码；
   ② `pull_timeout` 改按 deadline 循环（`clock()` 走完 `millis` 才算超时，
   `wait` 返 false 只当「醒了一次」）。

## 12 · 验证

```bash
$ ( sleep 3; printf 'dir\n'; sleep 2; printf 'req\n' ) | QEMU_TIMEOUT=25 cargo run --release
SQware shell
sq > dir
discover echo -> found
  echo
sq > req
req echo -> "ifmmp.tfswjdf..."     # hello-service 逐字节 +1，走新协议
```

`req` 路径：`Directory::open`（收下启动期握手配给的目录请求门闩，自造回信 hole 并
`Accord` 给目录）→ `Connect("echo")`（目录 `gate::accord` 转授入口门闩）→ 调用方
`Owned(entry).owner` 求服务 id → `UnsealHole` + `Accord` 给该 id（自带回信通道）
→ `Push`（前 8 字节回信 token）→ echo `+1` → `Push` 回信 → `Pull` →
`disconnect`（`Revoke` + `Release`）。

## 13 · 文件清单

```
新增  crates/env/src/dispatch.rs            协议类型 + 编解码
新增  kernel/src/work/unit/gate/release.rs  自释原语
新增  task/src/core/directory.rs            目录注册表 + 协议适配（用户态）
新增  task/src/bin/supervisor/dir.rs        目录域程序（S 态：Collect 门闩 → 请求循环）
删   kernel/src/service/                   目录移出内核（原为内核闭包任务）
改写  kernel/src/boot.rs                    根授予 + 三域装载（shell / echo / dir）
改   crates/env/src/fid.rs                  +Collect/Release；删 ServiceCall/ServiceId
改   crates/env/src/wire.rs                 +FromPair (PieToken, Permission)
改   crates/env/src/ucall.rs                warpper #[inline(never)]
改   kernel/src/runtime/switcher/envcall.rs Collect/Release handler；删 class 7
改   kernel/src/runtime/switcher/trampoline.rs  __utrap 保存顺序修复
改   kernel/src/work/unit/loader.rs         纯 .bss 段装载修复
改写  task/src/env/service.rs               Directory 会话 + Service 句柄
改   task/src/env/mail.rs                   collect() / release() 封装
改   task/src/bin/shell.rs                  req 走新协议 + dir 命令
```

### 13.1 派生与撤销（本次新增）

```
新增  kernel/src/work/unit/gate/snap.rs       全世界任务快照 + 沿 sire 的查询
新增  kernel/src/work/unit/gate/cull.rs       级联撤销 cull + 退出钩子 doom
改   kernel/src/work/unit/gate/pie.rs         Pie.sire（唯一的派生边）
改   kernel/src/work/unit/gate/accord.rs      子门闩写 sire；去掉 current_id
改   kernel/src/work/unit/gate/{revoke,release}.rs  鉴权 + 级联
改   kernel/src/work/room/scheduler/core.rs   snap()（存活任务快照，O(存活)）
改   kernel/src/boot.rs                       EXIT_HOOKS 加 gate::doom + 注入快照
改   kernel/src/runtime/switcher/envcall.rs   Accord/Revoke/Release/Collect/Owned 取快照
改   task/src/bin/user/shell.rs               cascade 自检命令（三跳/无关分支/release/任务消亡）
```

### 13.2 资源寿命与封印（本次新增）

```
删   kernel/src/work/mail/memo.rs             全局资源表（寿命改由引用计数决定）
改   kernel/src/work/mail/hole.rs             ResourceId/alloc_id 迁入；Drop 接管唤醒；seal 只置死
改   kernel/src/work/mail/pole.rs             meta() 直接返 Arc；seal 只置死
改   kernel/src/work/unit/gate/pie.rs         Pie.meta: Arc<M>（唯一强引用）；删 resource/alive
改   kernel/src/work/unit/gate/{accord,cull,narrow}.rs  强引用克隆 / 锁外 drop / alive 按 variant
改   kernel/src/runtime/switcher/envcall.rs   Seal 鉴权 owner-only；各 arm 取 Arc
改   task/src/bin/user/shell.rs               reclaim 自检命令（引用回收 / 封印归属 / 开辟者消亡）
```

### 13.3 发送者盖章（本次新增）

```
改   crates/env/src/fid.rs                    Pull 返回 (长度, 发送者 TaskId)
改   crates/env/src/wire.rs                   +FromPair (usize, TaskId)
改   kernel/src/work/mail/hole.rs             槽带 from；try_push/try_pull 传递来源
改   kernel/src/runtime/switcher/envcall.rs   Push 盖章、Pull 回传 a1
改   task/src/env/mail.rs                     pull_from() / HolePie::pull_from()
改   task/src/bin/supervisor/dir.rs           caller = sender；回信地址一致性检查
改   task/src/core/service.rs                 Directory::reply_target()（自检用）
改   task/src/bin/user/shell.rs               spoof 自检命令（盖章 / 正向对照 / 猜 token）
```
