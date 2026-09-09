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
pub struct Binding { name: Name, entry: Pie<HoleMeta> }
pub struct Directory { bindings: HashMap<Name, Binding> }
pub type ServiceRegistry = SpinLock<Directory>;
pub enum DirectoryError { Taken, Unknown, NotOwner, NotGrantable }
```

**接口 = 一份入口门闩**（不是 req/rep 两份）：目录只知道「服务有一个入口」，
不知道服务内部怎么执行。回信通道由**调用方自带**——与目录协议自身同构。

不变量做成类型义务：名字唯一（`bind` 返 `Taken`）、绑定必持门闩（字段类型是
`Pie`）、非法名不可表达（`Name` 只能由 `new` 造出）。

## 4 · 线格式（64 字节，沿用现有 hole）

```text
请求
[0]      op      u8      1=Register 2=Unregister 3=Replace 4=Resolve 5=Enumerate 6=Connect
[1..33]  name    [u8;32] 目标名字 / Enumerate 游标（全 0 = 从头开始）
[33..41] entry   u64 LE  Register/Replace：入口门闩的目录侧 pie token
[41..49] 保留    u64 LE  必须为 0
[49..57] reply   u64 LE  调用方自带的回信 pie 的目录侧 token（0 = 无回复预期）
[57..64] 保留（0）

回复
[0]      status  u8      0=Ok 1=Found 2=Connected 3=NotFound 4=Denied 5=Taken
[1..33]  name    [u8;32] Found：Resolve 命中的名字 / Enumerate 的一页
[1..9]   entry   u64 LE  Connected：目录转授给调用方的入口门闩 token
[9..17]  owner   u64 LE  Connected：服务 owner task id
```

`Enumerate` 按名字**排序**分页（HashMap 迭代顺序无保证，顺序必须是契约）：
`after` 之后的第一条，`NotFound` 即到头。

## 5 · 身份与授权

```text
调用方 ──(boot 授予)──▶ 目录入口门闩（Collect 取回，vestor = 目录 task id）
调用方 ──UnsealHole + Accord──▶ 目录：回信 hole 的对端 token（写进 [49..57]）
目录   ──gate::accord──▶ 调用方权限表（子集 R|W）
服务 owner = 入口门闩的 vestor()
```

- **身份不来自消息体**：目录按请求取「`[49..57]` 那枚回信 pie 的 `vestor`」——
  内核在 `Accord` 时赋值，消息体伪造不了。没带有效回信 pie 即无身份（`caller = 0`）：
  `Register` 不看身份，`Unregister`/`Replace`/`Connect` 一律拒绝。
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
| `MailCall::Collect { index } -> (PieToken, Permission, TaskId)` | 报出我持有的第 index 份（含其 `vestor`） | 用户态此前**无法自省自己的权限表** |
| `MailCall::Release { token }` | 放下我自己的一份（Pole 同步 unmap） | 此前**没有任何自释路径**（`revoke` 只允许授与人收回） |

配对：`Unseal*` ↔ `Seal`（动资源）；`Accord` ↔ `Revoke`（他人）；`Collect` ↔
`Release`（自己）。

## 8 · 引导（根授予）

内核是根授予的源头，机制不变（父任务把权限交给子任务）：

```text
boot:
  1. 建目录 req hole + 入口门闩（vestor = 目录 task id）
  2. spawn shell、spawn 目录（目录只捕获 dreq；不再捕获 caller）
  3. 把入口门闩放进 shell / echo 权限表（索引 0 / 1）
  4. echo 自注册：Collect 取回 entry + 目录门闩 → Accord entry 副本给目录
     → UnsealHole 自造回信 hole + Accord 给目录 → Register（回信 token 写 [49..57]）
  5. 目录收下 entry 门闩（take_entry）并 bind("echo")
```

用户态 `Directory::open()` 用 `Collect` 取回目录门闩，顺带拿到它的 `vestor`
（= 目录 task id，`Accord` 回信 hole 的目标）——**不需要向用户态传任何整数**。
目录会话是进程级授权，不随命令关闭。

## 9 · 已决 / 被否

| 决策 | 定论 |
|---|---|
| `ServiceCall`（class 7） | **删除**——它是入口策略，不是原语 |
| 根授予 | 逐级委托（`Accord`）+ 自省（`Collect`）；不做公开 id |
| 接口形态 | 一份入口门闩（A3）；回信由调用方自带 |
| 一个名字几个 provider | 1 个 |
| 名字权限 | v1 不做管理接口；`Register` 资格 = 持有门闩 |
| Disconnect | 不进协议 = `Release` |
| 目录状态 | 无连接实例；Unregister 只管名字 |

## 10 · 已知边界

1. **回信 pie 的 token 可猜**：身份 = 回信 pie 的 `vestor`，而 pie token 是全局
   连续小整数（`gate::next_pie_token`）。多 client 下，任务 A 一旦猜中 B 已 `Accord`
   给目录的回信 token，就能以 B 的身份发请求（回复仍落进 B 的 hole）。真正的修法是
   **per-caller 请求通道**（目录按「从哪条 hole 收到」定身份，不信任任何 body 字段）
   或**不可猜的 pie token**；v1 单 client 下不构成问题。
2. **名字可抢注**：v1 没有名字权限；任何能造门闩的任务都能注册新名字。
3. **Unregister/Replace 已实现但不在常规演示里**：echo 自注册路径已由「注册后立刻
   自注销」临时验证（身份取 echo 自身，返回 Ok）；v1 shell 没有对应命令，故不常驻。
4. `Enumerate` 一次一个名字（64 字节装不下列表）。

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

`req` 路径：`Directory::open`（`Collect` 取目录门闩 + 目录 task id，自造回信 hole
并 `Accord` 给目录）→ `Connect("echo")`（目录 `gate::accord` 转授入口门闩 + 回 owner）
→ 调用方 `UnsealHole` + `Accord` 给 owner（自带回信通道）→ `Push`（前 8 字节回信
token）→ echo `+1` → `Push` 回信 → `Pull` → `disconnect`（`Revoke` + `Release`）。

## 13 · 文件清单

```
新增  crates/env/src/dispatch.rs            协议类型 + 编解码
新增  kernel/src/work/unit/gate/release.rs  自释原语
改写  kernel/src/service/dispatch.rs        Directory 核心 + serve 适配
改写  kernel/src/boot.rs                    根授予 + 目录/echo（echo 自注册，不 bind）
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
