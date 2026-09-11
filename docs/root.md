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
| AC5 | 子域崩溃也能被父域观察到 | ✅ `Join` 由内核在收尾路径唤醒；返回真 ⇔ 退出钩子已跑完 |
| AC6 | 子域启动参数为空、服务门闩自开 | ✅ `Spawn(.., &[], ..)`；`dir`/`echo` 各自 `UnsealHole` |
| AC7 | 目录身份不经报文/启动参数传递 | ✅ 客户端用 `Reserve(门闩).owner` 求得（见 §9） |
| AC8 | 跨血缘的"杀"由本域经内核的他杀原语执行 | ✅ `kill echo` → `kill echo -> ok`，目录侧 `discover echo -> not found`（§5.1） |

## 2 · 裁决账

| 裁决 | 定论 |
|---|---|
| D1 | **E5**：清单解释权上移——内核不含清单格式，只把 initrd 区只读映射给 root |
| D2 | 建域资格 = **能力**（`UnsealNole` 铸一枚，S 态门控）+ **S 态兜底**；U 态即便持有建域权也一律 `-1 Denied`。两道门不是冗余：能力答「谁有权」，S 态答「血缘树能不能伸进沙箱外」 |
| D3 | 建域与授权**分两笔**（`Build`/`Spawn` → `Accord` → `Hatch`） |
| D4 | 关机判据 = 计数（`PUSHED/REAPED`）+ `ROOTED` 守门（原 `BOOT_DONE`） |
| D5′ | 新增 `Join { task, millis }`（有界，内核驱动唤醒） |
| S1 | 未放行态 `Held` + `Hatch`（而非域级授权表 / 轮询） |
| S6 | `Join` 用**独立站点表**（不污染 `WaitKey` 的命名空间） |
| S7 | 负码新增 `-6 BadImage` |
| N1 | 诊断名挂**域**（`Team.name: Name`，线程保留角色名） |
| N2/N3/N4 | 非 `Held` → `-1`；`Join` 放开 `usize::MAX`；`MAX_ARGS = 64` |
| 命名 | `Build`（装域）/ `Spawn`（产线程）/ `Hatch`（放行）/ `Join`（等结束） |
| T2-1 | **内核盖章**：`HoleMeta/PoleMeta.owner`（资源开辟者）+ `PieCall::Reserve` |
| T2-2 | 报到通道 = **父域预建的报到孔**（一条，串行握手共用） |
| T2-3 | 握手**串行**（hatch 一个 → 收报到 → 配给 → 下一个） |
| T2-4 | 删 `Reply::Connected.owner`（同一事实由 `Reserve` 给出） |
| T2-5 | 启动通道**不回收**（`memo` 保活到关机） |
| T2 命名 | 子→父 `Quay`（报到）/ 父→子 `Pier`（配给）/ `dock`（父侧开孔）/ `moor`（子侧认孔） |
| K1 | **机制在核、政策在服务**：内核只给 `RoomCall::Doom`（判据只有血缘，传递）；root 是全体域的祖先 ⇒ 跨血缘的"该不该"落在本域——这就是 Linux `kill` 的形状（谁都能请求，够格的那个执行） |
| K2 | 服务是 root 的**第二个线程**（不是新域）：主线程照旧阻塞在 `wait_dead(shell)`，服务线程无界等请求孔；两边各自"等着"，没有节拍轮询 |
| K3 | 准入门 = **能力**，不是判据表：入口门闩只亲授给 root 引荐过的域（`Refer`），沙箱里的域连不上目录、也就拿不到副本——"独占不靠判据，靠没有第二个创建入口" |
| K4 | 服务线程要**显式收场**（协议 `Quit`）：它在本域、是兄弟线程，**不在血缘里** ⇒ 级联收不到它，而请求孔又是它自己开的（本域 seal 不动） |
| J1 | **死亡两相**：`suspend` 先把整棵血缘子树的受害者**全部停摆**（置 `Doomed`），再逐个 `die`（钩子 → `Reaped` → 入队）。两相是正确性要求——钩子会摘门闩、唤醒等待者，若还有受害者能被唤醒后运行，它会在注定要死的状态下看到已死资源 |

## 3 · ABI（`crates/env/src/fid.rs`，class 1）

| idx | 原语 | 签名 |
|---|---|---|
| 0 | `Spawn` | `{ team: TeamId, entry: usize, args: VirtAddr, count: usize, stack: usize }` → `TaskId` |
| 5 | ~~`SpawnTask`~~ | **空号**（并入 `Spawn`，不复用） |
| 6 | `Build` | `{ elf: VirtAddr, len: usize, kind: ProgramKind, name: VirtAddr, name_len: usize, build: PieToken }` → `TeamId` |
| 7 | `Hatch` | `{ task: TaskId }` → `()` |
| 8 | `Join` | `{ task: TaskId, millis: usize }` → `bool` |

- `Spawn` **恒产 `Held`**；`team = TeamId(0)` = 当前域（沿用 `Mmap.at = 0` 的「自选」先例）。
- `Build` 是系统调用面上**唯一带能力入场**的原语：`build` 必须是调用方**自己表里**一枚活着
  的 `Nole`（token 不自证——内核只在调用方表里找它，故「拿别人的 token」不是绕过面），
  外加 S 态兜底。带它是为了让权威显式可审计：**「你说的是哪一枚」**。
- 启动参数写在**新任务栈顶** `[stack_top-8·count, stack_top)`，子方 `a0 = args VA`、`a1 = count`；
  `_start` 在首次调用前 `save_args`，用户侧 `env::task::args()` 取回。**子域一律空参数**
  （只有 root 收内核的清单视图）。
- `Join` 契约与 `MailCall::Wait` 同源：`true` = **调用开始时**目标已死**且收尾完成**
  （退出钩子已跑完——它名下的门闩与通道都已消失）；`false` = 未结束（可能挂起过）→
  调用模式 `loop { if Join{t,0} { break } Join{t,MAX} }`。栈/帧的回收是内核私事、
  对调用方不可观测，不入契约。

### 3.1 查证与枚举（class 7，`PieCall`）

> 两条轴拆开之后（数据轴 `MailCall` = class 5、权柄轴 `PieCall` = class 7），查证与枚举
> 都归**权柄轴**；`MailCall` 现在只剩 `Push` / `Pull` / `Wait` 三个数据面动作。
> 原来的 `Owned` 同时改名 **`Reserve`**：它答的是「这枚门闩的来历」，与「把数据推过去」
> 不是一件事。

| 原语 | 签名 | 语义 |
|---|---|---|
| `Reserve` | `{ token: PieToken }` → `(TaskId, TaskId)` | 我持有的这枚门闩：`vestor`（谁授的）+ `owner`（资源谁开的） |

- 错误：token 不在本任务表 → `-1 Denied`；资源已封印 → `-2 Dead`。
- 与 `Collect { index }`（**未改名**）分工：`Collect` 按索引**枚举**
  （`moor()` 靠它发现未见过的句柄），`Reserve` 按句柄**查事实**。
- 两个身份不可混用：`vestor` 是**门闩**的来历（`Accord` 转手即改写），`owner` 是
  **资源**的来历（任意副本共享同一事实）。

## 4 · 内核结构

- **血缘闭合**：`adopt` 并入 `TeamBuilder::spawn`——sire 非空 ⇒ 必入 `sire.heir`。
  一次性复活 `Spawn` 授权、`doom` 级联、`Sire/HeirCount/Heir`。
- **未放行态**：`TaskState::Held` + `Team.held: SpinLock<Option<Arc<Task>>>`（`Option` 把
  「至多一个引导线程」做成类型义务）；`Task::release` = `Held → Starved` 入队。
- **计数挂产生处**：`conductor::push()` 移到 `TaskBuilder::hold`——`Held` 被扑杀时
  `REAPED/PUSHED` 仍配平（否则 `done()` 恒假，永不停机）。
- **`Join` 站点**：`joins: HashMap<tid, JoinSite{pend, waiters}>` + `join_times` 超时旁路；
  `wake_joiners` 在 `clear_loop` 里逐条 reaped 任务调用——此刻钩子已跑完（死亡两相，
  见 `messenger::{suspend, die}`），**fault 死亡也能被 join 到**。
  `pend` 闭合「判死 → 入簿」窗口，且**锁内绝不查注册表**（那是 3→3，lockdep 会拒）。
- **清单视图**：boot 在 root 的用户段登记一段 VA（lowest first-fit，紧接镜像），把 initrd 区
  `borrow` 成只读；VA 与长度经启动参数交给 root。
- **资源开辟者**（T2）：`HoleMeta/PoleMeta` 增 `owner: usize`——`Unseal*` 时的任务 id，
  构造期定型、无 setter；`AnyPie::owner()` 走 `Weak::upgrade()` 读它（**不查 memo**，
  否则 `pies`(L3) → `memo`(L3) 是 3→3，lockdep 会拒）。

## 5 · 用户面

```text
programs/src/bin/supervisor/root/
  main.rs      解封建域权（NolePie::unseal）→ 开报到孔 → 逐子域串行握手
               → Join(shell) → exit
  manifest.rs  清单格式（只被 build.rs 与 root 知道）
crates/runtime/src/core/handshake.rs
  dock()       父侧：开报到孔（一次）
  moor()       子侧：认报到孔（vestor == sire 且 owner == sire）
  Quay / Pier  两条 8 字节报文（方向即类型）
```

- 清单格式与 `build.rs::INITRD_BINS` 对齐；`kind` 仍由**内核打包表**决定，root 原样转交
  ——**程序不自称特权级**这条不破：全仓唯一声明「装成哪种空间」的地方是内核的打包表。
- boot 只按 `ROOT_OFFSET/ROOT_LEN`（build.rs 导出）取 root 镜像，**不解析清单**。

### 5.1 他杀服务（`kill`）

内核那枚 `RoomCall::Doom` 只认血缘；跨血缘的"该不该"没有内核判据——**那本来就是政策**。
而 root 是**全体域的祖先**（boot 只装它）⇒ 它对任何域的杀都够格，于是它就是 Linux `kill`
里"够格的那一个"：**谁都能请求，够格的那个来执行**。

```text
客户端（shell / 任何拿得到入口副本的域）
   └─ Connect("doom") → 入口门闩副本 → Kill{target, ack} ─► root 的服务线程
                                                              │ ① 名字 → 目录：活实例 → 属主 task
                                                              │ ② room::doom(task)  ← 内核血缘判
                                                              │ ③ 探"指它的副本还在不在"
                                                              └─ 回执 Ok / Dead / Denied / Slow
```

- **线格式**（`crates/protocol/src/doom/`，两端共用一份定义）：`Kill = [op][target 32][ack 8]`
  （41 字节）+ 一字节回执；`Quit = [op]`（1 字节，无回执，只给主人停服用）。
- **四值回执不压成一档**：`Ok` = 已下令**且已等到它真的没了**（服务手里那枚指向目标的副本
  被摘掉——与"客户端死亡"用的同一条机制，`gate::doom` 的 BFS）；`Dead` = 没这个目标
  （ESRCH）；`Denied` = 内核/协议不许；`Slow` = 已下令、但它还在（ESRCH 之外的那一档，
  **不假装成功**）。
- **请求孔由服务线程自己开**：客户端认服务靠 `Reserve(入口门闩).owner`（门闩的**开辟者**），
  而门闩是 per-task 的 ⇒ **谁读它就得谁开它**，否则客户端的回信孔会配到另一个 task 的表里
  （实现期踩过这一脚：回执永远出不来，客户端等到 `-3 Busy`）。
- **收场**：主线程在 `wait_dead(shell)` 之后对服务说一句 `Quit`（走目录连上它）再
  `wait_dead(svc)`——原因见 `docs/task.md` §9.9（兄弟线程不在血缘里）。

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
6. **上行孔少了 `VEST`，子域授不出去**（B）：root 把上行孔按 `R|W` 授给子域，子域要把
   它再交给自己的控制线程时被拒——`Accord` 要求源门闩带 `VEST|BACK`。修法：`dock()` 改授
   `R|W|VEST`（**这条孔本来就是「子域往父域推」用的，多一个转授权不改变其用途**）。

## 7 · 已知边界

1. ~~root 长期持有目录请求门闩~~ —— **B 已消解**：目录能力改由 **dir 亲授**，root 只转达
   `Refer{who, name}`（`name` 即**预约**：名字空间由 root 播种），手里**零服务孔**
   （实测：root 的 6 枚门闩里 `owner == dir` 的只有 1 枚，即 dir 的控制孔；见 §10）。
   残留的是**控制面**：root 持有每个子域的控制孔副本，能往子域的控制通道推消息——
   那是父子关系的本义，不是越权。
2. `initrd` 仍在（内核只用于取 root 镜像）；正式供给通道（文件服务 / 设备发现）就位后，
   本模块与 `build.rs` 打包端一起删除，`Build` 原语不受影响。
3. 无 kill 原语：root 退出即 `doom` 级联，故不需要。
4. `Spawn` 的 `args` 是**标量参数**；大数据仍走权限表（`fid.rs` 的裁决只放宽了标量一侧）。
5. **服务必须自开入口 hole**：客户端用 `Reserve(entry).owner` 认服务；若由他人代开，
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

## 9 · 启动期握手（B：谁开孔、谁认孔）

```text
root                                       子域
  dock(child)   开上行孔（mtu=9，每子域一条）+ Accord(child, R|W|VEST)
  Build+Spawn(Held)                          —— 启动参数为空
  Hatch(child) ───────────────────────────▶  起跑
                                             moor()      认上行孔
                                                         （vestor==sire 且 owner==sire）
                                             UnsealHole  自建控制孔（与自己的服务孔分离）
                                             Accord(sire) 交给父域
  Quay::pull ◀────────────────────────────  Quay{ 控制孔在父侧的句柄 }
  校验 Reserve(句柄).vestor == child
  ── 客户端要目录能力时 ──
  Refer{who, name} ──▶ dir 控制孔           dir 控制线程：reserve(name, who)
                                            → H.accord(who, R|W)
  Referred{token} ◀── dir 上行孔
  Pier{token} ──▶ 子域控制孔 ─────────────▶  Pier::pull → dir_id = Reserve(token).owner

dir 的请求门闩 `H` **只有 dir 自己开、自己持**（root 不碰）；
echo 的入口门闩同理；两者都另开一条控制孔给 root。
```

- **身份全程不经报文/启动参数**：目录 id 由 `Reserve(门闩).owner` 从资源事实推出；
  B 之后客户端拿到的副本 `vestor == dir`，来源还能再自证一层。
- **认上行孔要两个条件**：`vestor == sire`（父域授的）**且** `owner == sire`（父域开的）。
- **四条报文**（`[0] tag` + payload）：`Quay` / `Pier` / `Referred` 各 9 字节；`Refer` 两种
  线形——只引荐 9 字节，带预约 41 字节（`tag` + `who` + 32 字节名字）。
- **每条孔单一发送者**：上行孔只有子域推、下行孔只有父域推、dir 控制孔只有 root 推。
- 串行握手 ⇒ 上行孔单槽无争用。

## 10 · B：root 零服务孔（引入式拓扑）

**问题**：T2 之后 root 仍持目录请求门闩的一份 `R|W|VEST` 副本——它能往目录的请求队列里
塞消息、抢走客户端的待处理请求（`Accord` 的 `subset ⊆ 自身权限` 决定了「要分发就必须持有」）。

**做法**：目录能力由 **dir 亲授**。

```text
root 手里：N 条自建上行孔 + N 条子域控制孔副本           ← 零服务孔
dir  手里：H（请求门闩，只自己持）+ C（控制孔，给 root）
dir 控制线程：pull(C) → H.accord(who, R|W) → push(上行孔, Referred)
```

| 判据 | 结果 |
|---|---|
| AC-B1 root 无服务孔 | ✅ 实测 root 持 6 枚：3 条自建上行孔 + 3 条子域控制孔副本；`owner == dir` 的仅 1 枚（控制孔，非 `H`） |
| AC-B2 客户端副本来源可自证 | ✅ `Reserve(t).vestor == dir` |
| AC-B4 e2e 不回退 | ✅ release + debug，自然停机 |
| AC-B5 dir 空闲 0% CPU | ✅ 主线程 park 在 `H`、控制线程 park 在 `C` |

**为什么 dir 变成两个线程**：它要同时听两条输入通道（客户端请求 `H`、父域引入 `C`），而
`Wait` 一次只能等一条孔——单线程 park 在 `H` 上就接不到引入请求。控制面（授不发）与数据面
（注册表）分开，主线程的循环一字未改。

**两线程怎么交接门闩**：门闩是 **per-task** 的（同域不同线程也各持一份），主线程 `Accord`
出去拿到的是对方表里的 token，只能经**同域共享内存**交接——`Spawn` 恒产 `Held`，于是
「先 `Accord` 三枚 → 写静态 → `Hatch`」天然是一个同步点，控制线程读到的必然是写好的值。
