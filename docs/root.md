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
  `_start` 在首次调用前 `save_args`，用户侧 `env::task::args()` 取回。
- `Join` 契约与 `MailCall::Wait` 同源：`true` = **调用开始时**已回收（未挂起）；
  `false` = 未回收（可能挂起过）→ 调用模式 `loop { if Join{t,0} { break } Join{t,MAX} }`。

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

## 5 · 用户面

```text
task/src/bin/supervisor/root/
  main.rs      建目录 req hole → 建 dir/echo/shell → Accord → Hatch → Join(shell) → exit
  manifest.rs  清单解析（格式只被 build.rs 与 root 知道）
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

## 7 · 已知边界 / 有意偏离

1. **授权拓扑只做了一半**：裁决倾向 T2（子域自建 hole + `Sire` 上报），实现取的是
   **root 自建 hole + root 分发**（更少的握手）。**目录 task id 经启动参数告知**
   （取代旧 boot 期的 `vestor` 内标——那是内核侧特权，用户态 `Accord` 只会写授与人）。
   彻底 T2 需要一次启动期握手协议，留作后续。
2. `initrd` 仍在（内核只用于取 root 镜像）；正式供给通道（文件服务 / 设备发现）就位后，
   本模块与 `build.rs` 打包端一起删除，`Build` 原语不受影响。
3. 无 kill 原语：root 退出即 `doom` 级联，故不需要。
4. `Spawn` 的 `args` 是**标量参数**；大数据仍走权限表（`fid.rs` 的裁决只放宽了标量一侧）。

## 8 · 验证

```bash
$ ( sleep 5; printf 'spawn\n'; sleep 3; printf 'dir\n'; sleep 2; printf 'req\n';\
    sleep 2; printf 'hole\n'; sleep 2; printf 'exit\n' ) | QEMU_TIMEOUT=45 cargo run --release
spawnjoin -> 499500
discover echo -> found
req echo -> "ifmmp.tfswjdf…"
hole got "hi from shell…"
bye
root: session over, shutting down
task: all tasks exited, system halted        # ← 自然停机，无外部 timeout
```

debug 档同路径跑通（`dir` + `exit` → 同样自行复位），`cargo fmt --check` 干净。
