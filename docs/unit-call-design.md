# Unit 域生命周期接口设计稿

> 已被 [Unit 域统一重设计](unit-domain-redesign.md) 取代。以下保留为第一轮评审历史，不作为当前实施依据；其中 Join 等待语义及结果保留到 Team Oust 的方案已修正。

状态：待评审，尚未修改 ABI 或实现。依据 `master` 的 Unit 契约与 PR #5 提交 `35b3d73e13f695c935665efa6d79b12265af98e7`。本稿只处理 Unit 域；Construction 交付与 Rack 空间通知另行处理。

第二轮评审见 [UnitCall 设计优化评估](unit-call-design-optimization.md)。以下为第一稿备选；新建议优先评估 Join 返回 TaskExit，并先解决结果所有权，不再默认采用通用 Inspect。

## 问题与建议

PR 直接追加 `DebarkTeam`、`EmbarkTeam`、`Status`、`Observe`，同时引入 Team 暂停门、退出结果和 observed 故障过滤。调用数量不是唯一问题：目标范围、成功边界、结果寿命和策略归属都没有先形成公共契约。

建议撤下这四个新增入口，按三条职责重新组织：

1. `Embark/Debark` 仍表示放行与暂停，通过显式目标区分单 task 和 Team 子树。
2. `Join` 继续只等待 task 收尾，不偷偷加入状态查询、事件消费或故障处理模式。退出结果由一个明确设计的只读 `Inspect` 提供。
3. 非零退出如何影响应用、其他成员和整机场景，由 System、Job 和测具决定。Unit 不提供 `Observe`，读取结果也不改变退出性质。

推荐方案明确提出 **一个待评审的新调用 `Inspect`**，理由、权限、数据和寿命见下文。旧基线 12 个调用变为 13 个，替代 PR 当前的 16 个。若评审要求本轮绝不增加操作号，则保留旧 ABI，并暂缓依赖退出结果读取的功能；不能用未知结果冒充退出 0，也不能以另一个调用的私有参数暗藏查询功能。

## Unit 与其他层的边界

| 层 | 拥有的事实或策略 |
| --- | --- |
| Unit | Team/Task 的创建与血缘、Held 的首次放行、单任务停止、子树暂停、收尾事实、退出记录的寿命 |
| Room | 调度、当前任务 Reap、持有 Doom 权限时销毁域及子树 |
| Control | 实例主 task、实例 owner、认领、准备/退场 hooks、实例停止与结果汇总 |
| Job | 成员结果、前后台终端归属、暂停原因、是否取消其他成员、流排空 |
| 测具 | 哪些退出符合该场景预期、根服务是否成功、异常诊断是否导致用例失败 |

内核不知道“Shell 命令”“实例主程序”“后台读”“已被 Job 接管”。Team 首个 Spawn 的 task 不自动成为内核认定的主任务；Control 已经持有 `Built.task`，主任务语义归 Control。

## 完整操作表

class 保持 1，旧操作号显式固定，禁止依赖枚举顺序编号。

| slot | 调用 | 本轮变化 |
| --- | --- | --- |
| 0 | Spawn | 仍创建 Held task；增加内部退出记录预留，不改参数与首次构造契约 |
| 1 | SelfId | 不变 |
| 2 | Sire | 不变 |
| 3 | HeirCount | 不变 |
| 4 | Heir | 不变 |
| 5 | Build | 不变；没有 managed/observed 或 Shell 专用标志 |
| 6 | Embark | 参数从 TaskId 改为显式 ControlTarget |
| 7 | Fall | 不变，仍只观察交入能力 |
| 8 | Join | 不变，仍是 `TaskId + Wait -> bool` |
| 9 | Oust | 保持权限、收尾和不等物理回收的边界；同步释放对应退出记录 |
| 10 | Debark | 参数从 TaskId 改为显式 ControlTarget |
| 11 | Slay | 不变，仍针对单 task，并保留已有的后代级联语义 |
| 12..15 | 退役 | 拒绝 PR 中 DebarkTeam/EmbarkTeam/Status/Observe 的编码 |
| 16 | Inspect | 待评审的新接口；一次读取有界的生命周期事实 |

基线枚举包含 12 个变体，slot 为 0..11。PR 已使用的 12..15 视为退役编号，推荐把 `Inspect` 分配到 16，不复用已用于四个旧提案入口的值。实施时以黄金测试固定整张表。

不为追求外观对称而扩展 Slay 或 Join。销毁 Team 子树继续走受 Doom 能力约束的 Room::Doom，不能通过新增目标绕过这项权限。

## 放行与暂停：显式目标

```rust
enum ControlTarget {
    Task(TaskId),
    Tree(TeamId),
}

Embark { target: ControlTarget } -> UnitResult<()>
Debark { target: ControlTarget } -> UnitResult<()>
```

`Task` 只改变该 task 的停止状态；`Tree` 表示指定 Team 内全部 task 及其 task 拥有的后代 Team。不要把单 task 操作悄悄改为子树操作。

授权：

- `Task` 沿用现有规则：同 Team，或目标 Team 位于当前 task 的直接 heir 表中。
- `Tree` 只允许当前 task 直接拥有的子 Team；TeamId 的存在或猜中编号不能授予控制权。
- 不允许通过 `Tree` 暂停当前 Team，避免在请求“全体离开调度槽”时仍等待调用者自己。当前 task 自暂停继续使用 `Task(SelfId)` 的原契约。
- 子树成员的权限来自对根 Team 的既有控制关系；不会因此获得其他兄弟 Team 的操作权。

首次放行与恢复分开：

- `Embark(Task)` 可将 Held task 首次放行。
- `Embark(Tree)` 只撤掉本根的暂停门，不放行 Held task，不撤掉单 task 的 stopped，也不撤掉后代自身的暂停门。
- 已暂停根的 `Debark(Tree)`、已恢复根的 `Embark(Tree)` 均幂等。单 task 的既有失败行为保持，不借本轮扩大变更。

暂停成功必须同时成立：根门已发布；所有已运行成员已经离开 Running 槽；确认使用的成员关系稳定。之后新建、放行、唤醒、窃取和装槽路径都受暂停门约束。

控制树关系的寿命不能只依赖父 task 的 Weak 指针：父 task 收尾与子树回收之间，继承门和结果归属仍必须存在，直到相应控制树节点被合法撤回。

`Busy` 表示已请求暂停但尚未确认，门继续生效，可重试；不是“没发生任何事”。`OoM` 必须发生在新门发布前，或者由可证明无分配的推进路径避免；不能分配失败后留下未说明的半暂停。`Denied` 不改变状态。

恢复成功表示根门已撤掉，符合条件的 parked 成员已经进入可调度路径，并不保证它们已执行指令。Blocked 成员保留原等待；等待完成只使其可运行，不抹掉独立停止状态。

并发实现要求：门检查与装槽必须有明确的线性化关系；建成员与恢复扫描也必须协调。仅在扫描前后读一次 revision 不足以替代该证明，特别要覆盖“恢复复核后、撤门前，出现并被停放的新成员”。不为了本轮另加全系统锁或全局任务扫描。

## Join：维持收尾契约

`Join(task, wait)` 继续返回目标是否已 Reaped 且退出 hooks 已结束。超时返回 false，Wait::POLL 不阻塞，Forever 无期限。成功不保证栈、trap 帧、Team 或 Space 已物理回收。

Join 不消费退出记录，不把非零 reason 变成 UnitFail，不更新诊断分类，也不返回“主任务结束即整个子树结束”。延迟回收与 Oust 的现有边界保留。

原始 Join 的 Denied 不能当作“确认死亡”：现有部分包装中的 `unwrap_or(true)` 是调用方选择的探活策略，不是本轮可以用于制造可靠结果的证据。

## 退出事实：只读 Inspect 提案

```rust
Inspect {
    team: TeamId,
    after: TaskId,
    buf: VirtAddr,
    capacity: usize,
} -> UnitResult<(usize, TaskId)> // written, next_after
```

Inspect 查询所指 Team 子树中保留的 task 记录。根 Team 是结果的可寻址寿命边界，查询不依赖主 task 仍在 roster 里。权限为当前 Team或当前 task 的直接子 Team，拒绝其他 Team；当前 Team 使用实际 TeamId，不引入第二种 0 哨兵含义。

缓冲区由固定头及最多 capacity 个 TaskInfo 组成，调用方提供的有效可写范围至少为 `32 + capacity * 64` 字节，先检查全部范围与算术溢出。每次最多 64 条，最大 4128 字节，返回的 written 为记录条数。按 TaskId 递增，严格在 after 之后。初次 after=0；非零 after 必须指向本根仍保留的记录。next_after=0 表示本页结束时没有更大记录，否则返回本页最后一条的 TaskId，用它继续。分页不是全树冻结快照；每条记录内部一致，创建和退出期间需要重新从 0 扫描以更新已读记录。

固定头为 4 个 u64 LE：根 TeamId、关系修订号、根自身暂停位、根有效暂停位。TaskInfo 为 8 个 u64 LE：TaskId、所属 TeamId、创建它的 TaskId、TaskState、单任务 stopped、有效子树暂停、ExitCause、Reason。不复制 Rust struct 内存，不泄露内核地址；所有保留值/非法状态要严格解码。

TaskState 的编码为 Held=0、Ready=1、Running=2、Blocked=3、Terminating=4、Reaped=5（内部 Starved 对应 Ready，Doomed 对应 Terminating）。ExitCause 为 None=0、Reap=1、Killed=2、Cascade=3、Fault=4；两个暂停位只能为 0/1，None 时 Reason 必须为 0。Reason 只有退出记录有效时可解释。退出中的 task 可以仍表现为 Running/Ready 等但带已记录 cause；调用方必须等 Reaped 与 Join 的确认，不把 reason 已发布当作 hooks 已结束。主动 Reap 的 reason（包括 panic 约定码）保留原值，不推断“已处理”。ExitCause 来自真正的退出路径，不从 reason 数值反推。

这一查询是必要事实面，不替应用裁决：Control 按 Built.task 找主任务退出；其他 task 的非零退出或 Fault 是否终止实例由 Control 策略决定；Job 只接收各实例最终结果并决定是否继续同作业的其他成员。内核不提供 `fault.or(main_reason)` 这种替调用方挑结果的汇总。

失败：无权或根已 Oust 为 Denied；capacity=0、超过 64、游标与根不符、缓冲区不足/不可写也拒绝。没有更多记录是成功的空页。OoM 不产生部分记录；不得因读结果失败而丢失结果。检查与写回的详细线性化点在实现前随任务退出锁设计一并核对。

### 结果寿命与预算

- Spawn 发布之前同时预留 task 及其所属控制树所需的记录槽；任一步失败不发布 task。
- 退出路径只填预留记录，禁止依赖用户态报告、临时堆分配或有界诊断环。
- task/栈物理回收不删除退出记录；结果至少保留到拥有它的根 Team 被 Oust。重复查询不消费。
- 中间父 task 退出或中间 Team 被放下，不让仍由祖先根拥有的记录失去归属。记录持有小型生命周期元数据，不长期持有 Task、Space、程序映像或资源能力。
- 预算计入创建路径，达到额度时拒绝下一次 Spawn；不能循环覆盖未读取结果。最大深度、每根记录数及对应失败码须在实施前定量确定，不能把“记录全部后代”实现成无上限增长。
- Oust 在现有构造操作门内重新核对收尾，再撤掉可查询归属。查询持有临时快照时可延后释放元数据，不允许重复回收或泄露。

如果结果寿命/预算无法满足，应回到设计讨论，不能复用 8 项 diagnose ledger 作可靠结果 API。

## 删除 Observe，分离事实与判定

删除 Unit::Observe 和 observed/observer 分支。读 Inspect、持有控制权、注册为应用实例，都不修改退出诊断。Task 结果按真实血缘/控制树保留，不依赖一个事后设置的布尔位。

当前 Observe 有两种职责：给后代结果找汇总根，以及把相同退出从整机失败账排除。前者由上面的结果寿命模型承担；后者是测具策略，不能以 ABI 解决。

实施时必须同步重做 QEMU 的判定路径：保留全部退出诊断，根服务/测具明确报告场景成功或失败，预期的命令非零退出由场景断言核对，非预期服务失败和内核 fault 仍导致失败。需要覆盖“命令退出 17 与服务故障同时发生”，防止把整机错误一起过滤。具体根角色、报告来源及断言位置必须在改 conductor 前列出；不能简单把所有非零退出忽略，也不能仅相信 Shell 输出的成功字符串。

## PR 迁移顺序

1. 评审目标、Inspect 的必要性与保留预算；固定操作号/字段编码/成功及失败契约。
2. 完成 Unit ABI 编解码和黄金测试；移除 12..15 的四个入口并拒绝其旧编码。
3. 把子树门与退出记录移入 Unit 的拥有者；暂停/恢复分派共用目标解析和授权。
4. Control 首次启动走 Embark(Task)，暂停/恢复走 Debark/Embark(Tree)，读取 Inspect 的原始记录后自行汇总结果。
5. 删除 Observe 调用及诊断过滤，改写真实场景的判定与断言；确认系统故障仍可见。
6. 更新 execution 包装、宿主替身、内核健康面、Shell 文档及整机脚本。内核与全部 ELF 一同重编译，不提供旧新布局混跑。

## 验收要求

ABI：存续操作号逐项固定，退役值拒绝；目标标签、缓冲区大小、分页、非法参数和返回状态有固定字节/寄存器期望。

权限：同 Team task、直接子 Team、兄弟/无关 Team、猜测 id、Oust 后查询分别检查；扩展 Tree 不绕过 Doom 权限。

暂停：Running/Ready/Blocked/Held、并发 Spawn、首次放行、唤醒、跨核窃取；根暂停与后代独立暂停、单 task 停止重叠；恢复窗口的新成员；分配失败与 Busy 重试。

结果：普通 0/17、panic/fault、Slay/cascade；主程序和辅助任务分开；父 task 先死亡、跨父回收、重复读取、分页中状态改变、预算耗尽、Oust 清理。主动 reason 即使等于某个保留码，也不能伪造另一种 ExitCause。

集成：主程序退出后清理辅助任务；一个成员 17、另一个 0 独立保留；fg/bg 与 Ctrl-C/Ctrl-Z；命令非零退出不覆盖 System 故障；所有 hooks 完成后才报告实例 Dead。静态模型不能替代真实多核与映射验证。

## 本轮需要评审的决定

1. 是否接受 Embark/Debark 的显式 Task/Tree 目标及 Tree 的直接父控制边界？
2. 是否接受一个通用只读 Inspect，而不是改变 Join 的返回/等待语义？
3. 是否接受退出记录由 Team 控制树持有、由 Oust 结束寿命，以及把整机场景判定移出 Observe？

尚未决策的实现参数：退出记录额度与最大深度、Tree 暂停/恢复的锁与发布协议、QEMU 根结果报告的具体接入点。这些是开始改实现前必须收敛的设计项，不是已经验证的保证。
