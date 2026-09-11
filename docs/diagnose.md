# diagnose — 诊断与现场转储

> 路径约定：`文件:行` 相对 `kernel/src/`。panic 之后的**调度侧**动作见 [task.md](task.md) §7；
> 分配器侧的预算见 [memory.md](memory.md) §10。

## 1 · 语义定位

诊断是**常驻**的：`halt` / `report` / `render` / `scene` / `trace` / `frame` / `backtrace`
**没有 feature 门**，三档产物里都在（`runtime/diagnose/mod.rs:1-15`）。只有两处例外：

| 能力 | 门 |
|---|---|
| 结构化导出（`export.rs` + trace 的宿主镜像） | `semihosting` |
| canary 现场清查 | `audit`（`scene.rs:405-408`） |

它要回答的是三个不同的问题：**谁先发现的**（`claim` / `ALARMER`）、**现场是什么样**
（`scene` + `frame`）、**之前发生了什么**（`trace` 事件环）。

## 2 · 结构

| 文件 | 职责 |
|---|---|
| `diagnose/halt.rs` | 报警源登记（`ALARM:21-24`）+ 单次报告（`claim` CAS `:57-67`）+ `broadcast:71-106`（SBI IPI）+ 停核 `hush:27-49` + 归巢 `home:141-162` + 组稿 `info:166-220` + 停机自环 `halt_loop:223-230` |
| `diagnose/report.rs` | `Report{seal, paras}`、`paragraph`、`seal`（打戳 hart + ticks）、`clear` |
| `diagnose/render.rs` | 段落 → stanza 定宽栅格（列宽 ＝ 非空槽最宽、char 安全截断、Decor 全抑制） |
| `diagnose/scene.rs` | 现场采集：`capture_kernel:125-163`（归巢落盘的 sp/fp 或当前）、`capture_user:166-202`（running 任务的 trap 帧）；`dump` 组稿 |
| `diagnose/frame.rs` | 领域无关的投影引擎：`StackReader` 逐页 `walk_raw` + DRAM 值域 + R 位（**采样绝不触发缺页**，`:15-18,107-138`）；`walk` ＝ `chain`（fp 链）+ `scan`（无表时扫候选 `ra`，4 对齐、去重、`SPAN=4096`/`DEPTH=32`） |
| `diagnose/trace.rs` | per-hart 事件环（`BUFFER_SIZE=512`，从 spare 仓常驻，`:239-270`）；`note` 尽力而为不失败；`panic_dump` 每 hart 倒 `TRACE_DUMP/hart_count` 条 |
| `diagnose/backtrace.rs` | 定长 `Backtrace{frames:[Frame;DEPTH]}`（**回溯层零分配做成类型约束**）+ `classify`（Root/Kernel/User/Unknown） |
| `diagnose/export.rs` | `semihosting` 下的单文件 `sqware-diagnose.jsonl`：事件行 + 整档报告两条流；`HOST` 锁 `try_lock` 串行、10_000 ticks 超时静默放弃 |

## 3 · 两条路径（它们**不是**同一条）

**内核 panic**（完整报告链）：

```text
panic! → panic_handler（halt.rs:120）→ alarm:100 → claim:57（CAS 抢占报警源 + 写 ALARMER）
       → broadcast:71（IPI，其余核在 trap/调度入口/三处自旋里经 hush → hunker 卧倒）
胜出核 → home:141（落盘 SCENE=[sp,fp] 并切 ROOT 栈）→ info:166
       → ROOT canary 复读 :170 → 门户切 Spare :174 → 组稿 [panic] at file:line:col + 任务身份
       → trace::note(Halt(Panic)) → scene::dump → seal → render → export → halt_loop
```

嵌套 panic 走 `claim` 的假分支：只打一行 `info:` 然后 `halt_loop()`——**报告只报一次**。

**用户域 panic**（**不进**报告链）：`programs/src/entry.rs:45` 用字面量 `put`（禁 `format!`）
打印，然后 `exit()` → `RoomCall::Reap{reason}` → 内核写 `EXIT_REASON` 并返回空指针 →
`trap_handler` 走退场窄尾 → `messenger::quit` → `trace::note(RoomEvent::Exit{tid,reason})` →
`reap` → `bury`。

> 这条路径**不登记报警源、不组报告、不做现场转储**。内核不需要知道「那是 panic」：
> 一个域不可续，就是一个任务停止。现场转储只由内核 panic 或显式 `crash_scene!` 触发。

## 4 · 不变量

1. **报警源唯一，且仲裁先于换栈**——违反：多核争同一个 ROOT 栈顶、报告交错。
   `claim` 的 CAS（`halt.rs:57-67`）、`panic_handler:122-124`。
2. **停核 / 事件 / 导出三路不碰业务内存、不失败**——违反：报告自己卡死造成二次 panic。
   `hunker:34-38`、`note:179-186`、`with_host:35-43`。
3. **报告只报一次**（嵌套不重入转储）——`claim:57-64`、`panic_handler:125-133`。
4. **采样绝不触发缺页**——违反：回溯中途缺页侵入诊断本身。`frame.rs:107-138`。
5. **回溯层零分配**（定长数组）——违反：panic 现场再分配。`backtrace.rs:49-53`。
   注意**组稿层是有分配的**（`halt.rs:178-204`），所以要先切 spare 门户（`halt.rs:174`）。
6. **ROOT 栈 canary 复检**——违反：boot 之后误用 ROOT 不可见。`halt.rs:170-171`。
7. **退出原因单一出口**——违反：事件重复或丢失。`set_exit_reason` → `quit` 发
   `RoomEvent::Exit`。

## 5 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| 用户域退场 ≠ 内核 panic | 走 `Reap → quit → RoomEvent::Exit`，只记 trace | 内核不需要知道「panic」这个词（`entry.rs:32-36`、`trace.rs:71-78`） |
| 报文体有分配 | 允许，但先切 spare 门户 | 门户有免锁判别位，panic 路径不能走主堆（`halt.rs:174`） |
| 采样只读 `walk_raw` | 不触发缺页、不改页表 | 诊断不得成为故障源（`frame.rs:15-18`） |
| 结构化导出放 `semihosting` 门后 | 非默认 | 它要宿主文件系统；**门跑过的产物没有它** |
| canary 现场清查放 `audit` 门后 | 非默认 | 清查本身要遍历，产品档不该付 |
| trace 用定长环、每 hart 一份 | 不分配、不跨核同步 | 崩溃现场要能读（`trace.rs:239-270`） |

## 6 · 已知边界

1. **`Report` 的段落格式与锚点耦合**：`render` 的列宽按「非空槽最宽」算，任何一段超宽都会
   改变整段版式；诊断输出的稳定性因此依赖各 `paragraph` 的自觉。
2. **`export` 会静默放弃**：宿主锁 `try_lock` 失败或超过 10_000 ticks 就丢，**不打任何提示**
   （`export.rs`）；门里也从不带 `semihosting`，所以这条路径没有端到端验证。
3. ~~**`scene` 的段落名与注释不符**~~ —— **已修（本轮）**（改为「自成一个 `scene` 段，排在 csr 段
   之后」，并把 `csr_rows` 的「首行表头」改成「表头（第二行）」）。原记录：`scene.rs:425` 注「并入 csr 段最前」，实际另起 `scene` 段
   且在 csr 段之后；`csr_rows` 自称「首行表头」（`scene.rs:221`），实际首行是 task 行、表头在
   第二行（`:226-236`）。
4. ~~**`backtrace.rs:131` 的文档挂错对象**~~ —— **已修（本轮）**（改为一行历史注，指明 `Registers`
   已迁 `scene.rs`）。原记录：「现场寄存器」这句 doc 挂在 `kind_label` 上，而
   `Registers` 已迁到 `scene.rs:96`。
5. **溯源只剩十六进制**：`symbol()` 退化为裸 hex（`backtrace.rs:24-26`），`FrameResolver::
   executable` 恒 `false`（`:124-126`）使 `classify` 第三档不可达，`rustc-demangle` 零调用点。
6. **`kernel/Cargo.toml:8-14` 仍说「两档」**（现为三档），但「门跑过的产物没有结构化导出」
   这一条仍然成立（门的两处 build 都不传 `semihosting`）。

## 7 · 判据与验证

- **门只看两件事**：捕获里**没有** `[panic] at`，harden 档里**没有** `[depend]`
  （`scripts/examine.nu:447,451-456`）——诊断本身不产出判据，它是**失败时的证据**。
- **失败现场归档**：`diag` 只在判 FAIL 时写出，含 seed / icount / 命令表 / 日志路径 /
  退出码（`examine.nu:537-542`）；每条命令的逐步输出留在 `<OUT>/run<i>/console.log`。
- **交互回显是诊断量不是判据**：逐键回显只在失败时写进 diag（`:351-354,370`）。
- **结构化导出的消费者**：`scripts/runner.nu` 恒归档 `diagnose-<seed>-<ts>.jsonl`，且只在
  「退出码 0 且无 `[panic] at`」时才删控制台捕获。
