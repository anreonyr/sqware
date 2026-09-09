# root — 根服务域（生成所有任务 · 自然关机）

> 内核 boot 期**只**装一个 S 态域 `root`；此后 shell / echo / dir 全部由它产生，
> 权限也由它分发。root 退出 ⇒ `doom` 级联扑杀全部子域 ⇒ 全部任务回收 ⇒
> `conductor::done` 自然停机（srst）。**不需要任何外部 timeout。**

## 1 · 目标与验收

| # | 判据 | 现状 |
|---|---|---|
| AC1 | boot 只产生 root 域 | ✅ `boot::spawn_root` |
| AC2 | 无外部 timeout 自行复位 | ✅ `exit` 后 `task: all tasks exited, system halted` |
| AC3 | root 异常死亡 → 级联 → 复位 | ✅ `doom` 挂在每条 reaped 任务上 |
| AC4 | 现有 e2e 不回归 | ✅ `spawn`/`dir`/`req`/`hole`/`clock`/`exit`（release + debug） |
| AC5 | 子域崩溃也能被父域观察到 | ✅ `Join` 由内核在回收路径唤醒 |
| AC6 | 子域启动参数为空、服务门闩自开 | ✅ `Spawn(.., &[], ..)`；`dir`/`echo` 各自 `UnsealHole` |
| AC7 | 目录身份不经报文/启动参数传递 | ✅ 客户端用 `Owned(门闩).owner` 求得（见 §9） |

## 2 · 裁决账

| 裁决 | 定论 |
|---|---|
| D1 | **E5**：清单解释权上移——内核不含清单格式，只把 initrd 区只读映射给 root |
| D2 | 建域资格 = **S 态**（U 态 `Build` 一律 `-1 Denied`；Pie 不破例） |
| D3 | 建域与授权**分两笔**（`Build`/`Spawn` → `Accord` → `Hatch`） |
| D4 | 关机判据 = 计数（`PUSHED/REAPED`）+ `ROOTED` 守门（原 `BOOT_DONE`） |
| D5′ | 新增 `Join { task, millis }`（有界，内核驱动唤醒） |
| S1 | 未放行态 `Held` + `Hatch`（而非域级授权表 / 轮询） |
| S6 | `Join` 用**独立站点表**（不污染 `WaitKey` 的命名空间） |
| S7 | 负码新增 `-6 BadImage` |
| N1 | 诊断名挂**域**（`Team.name: Name`，线程保留角色名） |
| N2/N3/N4 | 非 `Held` → `-1`；`Join` 放开 `usize::MAX`；`MAX_ARGS = 64` |
| 命名 | `Build`（装域）/ `Spawn`（产线程）/ `Hatch`（放行）/ `Join`（等结束） |
| T2-1 | **内核盖章**：`HoleMeta/PoleMeta.owner`（资源开辟者）+ `MailCall::Owned` |
| T2-2 | 报到通道 = **父域预建的报到孔**（一条，串行握手共用） |
| T2-3 | 握手**串行**（hatch 一个 → 收报到 → 配给 → 下一个） |
| T2-4 | 删 `Reply::Connected.owner`（同一事实由 `Owned` 给出） |
| T2-5 | 启动通道**不回收**（`memo` 保活到关机） |
| T2 命名 | 子→父 `Quay`（报到）/ 父→子 `Pier`（配给）/ `dock`（父侧开孔）/ `moor`（子侧认孔） |

## 3 · ABI（`crates/env/src/fid.rs`，class 1）

| idx | 原语 | 签名 |
|---|---|---|
| 0 | `Spawn` | `{ team: TeamId, entry: usize, args: VirtAddr, count: usize, stack: usize }` → `TaskId` |
| 5 | ~~`SpawnTask`~~ | **空号**（并入 `Spawn`，不复用） |
| 6 | `Build` | `{ elf: VirtAddr, len: usize, kind: ProgramKind, name: VirtAddr, name_len: usize }` → `TeamId` |
| 7 | `Hatch` | `{ task: TaskId }` → `()` |
| 8 | `Join` | `{ task: TaskId, millis: usize }` → `bool` |

- `Spawn` **恒产 `Held`**；`team = TeamId(0)` = 当前域（沿用 `Mmap.at = 0` 的「自选」先例）。
- 启动参数写在**新任务栈顶** `[stack_top-8·count, stack_top)`，子方 `a0 = args VA`、`a1 = count`；
  `_start` 在首次调用前 `save_args`，用户侧 `env::task::args()` 取回。**子域一律空参数**
  （只有 root 收内核的清单视图）。
- `Join` 契约与 `MailCall::Wait` 同源：`true` = **调用开始时**已回收（未挂起）；
  `false` = 未回收（可能挂起过）→ 调用模式 `loop { if Join{t,0} { break } Join{t,MAX} }`。

### 3.1 T2 新增（class 5，`MailCall` 末尾）

| idx | 原语 | 签名 | 语义 |
|---|---|---|---|
| 13 | `Owned` | `{ token: PieToken }` → `(TaskId, TaskId)` | 我持有的这枚门闩：`vestor`（谁授的）+ `owner`（资源谁开的） |

- 错误：token 不在本任务表 → `-1 Denied`；资源已封印 → `-2 Dead`。
- 与既有 `Collect { index }`（index 10，**未改名**）分工：`Collect` 按索引**枚举**
  （`moor()` 靠它发现未见过的句柄），`Owned` 按句柄**查事实**。
- 两个身份不可混用：`vestor` 是**门闩**的来历（`Accord` 转手即改写），`owner` 是
  **资源**的来历（任意副本共享同一事实）。

## 4 · 内核结构

- **血缘闭合**：`adopt` 并入 `TeamBuilder::spawn`——sire 非空 ⇒ 必入 `sire.heir`。
  一次性复活 `Spawn` 授权、`doom` 级联、`Sire/HeirCount/Heir`。
- **未放行态**：`TaskState::Held` + `Team.held: SpinLock<Option<Arc<Task>>>`（`Option` 把
  「至多一个引导线程」做成类型义务）；`Task::release` = `Held → Starved` 入队。
- **计数挂产生处**：`conductor::push()` 移到 `TaskBuilder::hold`——`Held` 被 kill 时
  `REAPED/PUSHED` 仍配平（否则 `done()` 恒假，永不停机）。
- **`Join` 站点**：`joins: HashMap<tid, JoinSite{pend, waiters}>` + `join_times` 超时旁路；
  `wake_joiners` 在 `clear_loop` 里逐条 reaped 任务调用——**fault 死亡也能被 join 到**。
  `pend` 闭合「判死 → 入簿」窗口，且**锁内绝不查注册表**（那是 3→3，lockdep 会拒）。
- **清单视图**：boot 在 root 的用户段登记一段 VA（lowest first-fit，紧接镜像），把 initrd 区
  `borrow` 成只读；VA 与长度经启动参数交给 root。
- **资源开辟者**（T2）：`HoleMeta/PoleMeta` 增 `owner: usize`——`Unseal*` 时的任务 id，
  构造期定型、无 setter；`AnyPie::owner()` 走 `Weak::upgrade()` 读它（**不查 memo**，
  否则 `pies`(L3) → `memo`(L3) 是 3→3，lockdep 会拒）。

## 5 · 用户面

```text
task/src/bin/supervisor/root/
  main.rs      开报到孔 → 逐子域串行握手 → Join(shell) → exit
  manifest.rs  清单格式（只被 build.rs 与 root 知道）
task/src/core/handshake.rs
  dock()       父侧：开报到孔（一次）
  moor()       子侧：认报到孔（vestor == sire 且 owner == sire）
  Quay / Pier  两条 8 字节报文（方向即类型）
```

- 清单格式与 `build.rs::INITRD_BINS` 对齐；`kind` 仍由**内核打包表**决定，root 原样转交
  （`docs/supervisor.md` §13 的「程序不自称特权级」不破）。
- boot 只按 `ROOT_OFFSET/ROOT_LEN`（build.rs 导出）取 root 镜像，**不解析清单**。

## 6 · 实现中发现并修掉的缺陷

1. **`TaskBuilder` 默认入口是 `IMAGE_BASE`**（而非域的 `e_entry`）：root 的引导线程从镜像
   首字节起跑，表现为「a0/a1 是启动参数、ra=0、跳到 0」。修法：`TaskBuilder::new` 取
   `team.default_entry()`（0 时退回 `IMAGE_BASE`）。——`Team.default_entry` 此前**只读不写**，
   由 `Build` 补上唯一写入点。
2. **`joins` 锁内查注册表 = 3→3**（debug lockdep 拒）：改为 `pend` 信标闭合竞态。
3. **清单视图 VA 冲突**：最初用固定 VA，与已装载镜像 `AlreadyMapped`——改为在用户段登记后
   借用映射。
4. **单孔同时承载报到与配给 → 子域读回自己的消息**（T2）：子域在一条孔上先 push `Quay`
   再 pull `Pier`，若父域还没被调度，它会把自己刚写的 `Quay` 读走，父域永远等不到报到
   （实测死锁）。修法：报到孔（父域开）只走 `Quay`，配给孔（子域自建）只走 `Pier`
   ——与既有 IPC 的「请求孔 / 回信孔分两条」同形。
5. **`moor()` 认错泊位**（T2）：只用 `vestor == sire()` 认报到孔时，会把 root **转授**
   的目录门闩副本（vestor 也是 root）当成报到孔。修法：加 `owner == sire()`——报到孔是
   **父域自己开的门**，转授来的门闩 `owner` 是别人。

## 7 · 已知边界

1. **root 长期持有目录请求门闩**（T2 后仍是）：`Accord` 的 `subset ⊆ 自身权限`，root 分发
   后放下就再也分不出去了——故 root 手里始终有一份 `R|W|VEST` 副本，理论上可向目录
   注入请求。要彻底消掉得让**目录亲授**（引入式拓扑），本模块未取。
2. `initrd` 仍在（内核只用于取 root 镜像）；正式供给通道（文件服务 / 设备发现）就位后，
   本模块与 `build.rs` 打包端一起删除，`Build` 原语不受影响。
3. 无 kill 原语：root 退出即 `doom` 级联，故不需要。
4. `Spawn` 的 `args` 是**标量参数**；大数据仍走权限表（`fid.rs` 的裁决只放宽了标量一侧）。
5. **服务必须自开入口 hole**：客户端用 `Owned(entry).owner` 认服务；若由他人代开，
   `owner` 会指向代开者（`docs/dispatch.md` 已列为硬规则）。

## 8 · 验证

```bash
$ ( sleep 8; printf 'spawn\n'; sleep 3; printf 'dir\n'; sleep 2; printf 'req\n';\
    sleep 2; printf 'hole\n'; sleep 2; printf 'clock\n'; sleep 2; printf 'exit\n' ) \
    | QEMU_TIMEOUT=60 cargo run --release
spawnjoin -> 499500
discover echo -> found
req echo -> "ifmmp.tfswjdf…"
hole got "hi from shell…"
clock 14.97… sec
bye
root: session over, shutting down
task: all tasks exited, system halted        # ← 自然停机，无外部 timeout
```

debug 档同路径跑通（`dir` + `req` + `exit` → 同样自行复位，无 lockdep 违规），
`cargo fmt --check` 干净。

## 9 · T2：启动期握手（谁开孔、谁认孔）

```text
root                                     子域
  dock()      开报到孔（mtu=8，一条）
  Build+Spawn(Held)                        —— 启动参数为空
  报到孔副本 Accord(child, R|W)
  Hatch(child) ─────────────────────────▶  起跑
                                           moor()      认报到孔
                                                        （vestor==sire 且 owner==sire）
                                           UnsealHole  自建孔（服务孔 / 配给通道）
                                           Accord(sire) 交给父域
  Quay::pull ◀──────────────────────────  Quay{ 孔在父侧的句柄 }
  校验 Owned(句柄).vestor == child
  Pier::push(载荷) ──────────────────────▶ Pier{ 目录门闩在本侧的句柄 }
  下一个子域…                              （客户端：dir_id = Owned(门闩).owner）

dir 的「孔」就是它的请求门闩（root 留作分发源，故它 `Accord` 时带 VEST）；
echo 的「孔」是它的入口门闩；shell 只造一条纯配给通道。
```

- **身份全程不经报文/启动参数**：目录 id 由 `Owned(门闩).owner` 从资源事实推出。
- **认泊位要两个条件**：`vestor == sire`（父域授的）**且** `owner == sire`（父域开的）。
- 串行握手 ⇒ 报到孔单槽无争用；每条子域自建的孔只由父域 push、由子域 pull。
