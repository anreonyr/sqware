# Unit 域统一设计：Team、Task、状态与调用

状态：已在隔离工作树实现，验证记录见 [迁移记录](unit-migration-report.md)。此前 [初期方案](unit-domain-redesign-history.md)、[results 方案](unit-domain-results-history.md) 均为历史，不再定义当前契约。

## 1. 收束决定

保留原有词汇与字段分组。Hear 并入 Join，Oust 只操作 Team，Scan 枚举直接 heir。不新增 Task.results、结果句柄、Oust(Task)、completion、Tag 存储、boarding、gate.closed 或 construction 状态镜像。

**退出事实属于 TaskState，结果保留与接收进度属于 Team 的成员记录。** Build 建立的 heir 关系授权相应域管理者；同 Team 的线程可以观察，但不共同拥有 Team。Spawn 创建者不自动新增一份独立结果订阅。

删除 Task.results 不要求取消多个祖先独立领取。采用契约：owned 根的管理者可以接收子树退出结果；各管理者独立领取，但退出事实只存一份。Join(Team) 查询该根子树中调用者尚未领取的记录。接收资格仅来自明确的 owned 根关系，不因同域观察权限自动产生。

## 2. 对象与字段

```text
Team {
    id,
    space,
    sire,       // 原创建者弱溯源
    state,      // 共享、同步保护的 TeamState
    tasks,      // 共享成员元数据与 held 保活容器
    life,       // 稳定父子元数据，不强持有祖先执行资源
}
Task {
    ident,
    life,       // 等待登记寿命，不证明退出
    state,      // 共享、同步保护的 TaskState
    gate,       // pies / heirs / version 及能力同步
    heir,       // 强持有 owned 子 Team
}
```

Team 的 tasks 与 life 如需共享访问成员集合，引用同一份成员存储，不复制列表。life 中的父链只引用元数据，不持有 Task/Team/Space 资源；子节点索引与活动节点寿命必须支持父执行对象先回收后的子树终止。Held 的强引用与 TaskIdent.team 形成暂时保活环，首次放行或终止必须拆除。

一条成员记录包含：

```text
TaskId
活 Task 的弱引用
共享 TaskState 的引用
尚未领取的管理者集合（按 owner TaskId 去重）
```

任务所属 Team 从容器归属得到，不再复制一份可改的归属。held 强引用属于同一 tasks 分组的保活索引，不放进持久退出状态。弱执行引用可失效，状态引用仍可保存退出事实；不能用弱引用失效代替 Reaped。

**不增加 completed、observed 或复制的 result 字段。** 待领取集合表示各管理者的接收进度，不是第二份完成事实。同一管理者因多个嵌套根获得资格时仍只记一次，领取任一匹配根都会消除其对该任务的待领取项。存在于成员表不等于仍活着，存活只查 TaskState。只有 Reaped 且待领取集合为空才能移除记录；成员总数不是活任务数。集合及所需链接在 Spawn 发布前预留，退出事实发布不分配。现有能力撤销 hooks 与等待续约仍可能准备临时工作，见迁移记录中的边界。

## 3. TaskState 与 TeamState

以下为有限载荷示意，不是直接可编译的递归 Rust enum。

```text
TaskState:
  Held
  Starved { next }
  Running { hart, ticks_left }
  Blocked { key, ticket, next, join: Option<JoinWait> }
  Parked
  Debarking { hart, ticks_left }
  Debarked { state: Held | Starved | Blocked }
  Doomed { hart: Option<HartId>, cause, reason }
  Reaped { cause, reason }

TeamState:
  Constructing { staged }
  Ready { default_entry }
  Debarking { state: Constructing | Ready }
  Debarked { state: Constructing | Ready }
  Doomed { staged }
  Ousted
```

TaskState 是执行位置与退出事实的唯一存储；Reaped 在 hooks 完成后发布，载荷无执行资源、等待票或队列链接。Doomed 保存尚未离槽的 hart；非运行任务先从 held、调度或等待容器摘出，再发布 hart=None，因而不用复制已撤除的执行位置。收尾工作链接和待埋葬链接属于工作/回收容器，不成为持久结果事实。

Task 本地停止用 Debarking/Debarked；祖先 Team 暂停导致的可运行停放用 Parked。Debarked(Blocked) 真正唤醒后变 Debarked(Starved)，不入运行队列。恢复 Task 不撤祖先暂停；恢复 Team 不放行 Held，也不撤成员/后代自己的停止。

TeamState 独立表达构造和域控制阶段。首次 Spawn 在一个提交点消费 staged、确定 default_entry、转 Ready；暂停包装保留。Doomed/Ousted 拒绝创建，暂停不关闭能力接收。Task gate 的接收条件从 Doomed/Reaped 派生，gate 本身仍保护能力关系。

Reaped/Ousted 均不等于物理回收。共享状态可晚于执行资源释放；结果记录不得经 TaskIdent 等引用继续撑住 Space。Sire 仍查当前 Team 创建者的弱溯源，不改为某次 Spawn 调用者。

## 4. Join 的完整契约

```text
Join(目标, 等待期限, 是否领取)
    → Pending / Reaped { task, cause, reason }
目标 = Task(TaskId) | Team(TeamId)
```

| 目标 | 不领取 | 领取 |
| --- | --- | --- |
| Task | 观察指定任务，可重复 | 指定成员 Reaped 后返回并移除本管理者的待领取项 |
| Team | 观察该 owned 根子树中一条本管理者可领取的 Reaped 记录，不推进 | 返回并移除该任务对应的本管理者待领取项 |

Task 的不领取授权保留原同 Team/直接 owned child 规则。Task 领取要求调用者在该记录的待领取集合中，且相应 owned 根授权仍有效。Team 形式只允许该根的 owner。结果观察不授予执行控制；同 Team 不授予领取权。顶层无用户 owner 的 Team 由明确的启动管理职责领取，不设置永久结果保留例外。

观察记录从调用者当前 Team 或直接 heir 的成员表定位；领取还可从 owned 根的稳定子树元数据定位其待领取记录；不以 ID 作为权限，不新增强持有所有已退出任务的全局结果表。临时观察在同步点固定共享状态引用。

领取成功的线性化点是：确认 Reaped，构造完整返回值，并移除本管理者的待领取项。相同管理者并发领取同一任务仅一人成功，另一人返回 Denied；不同管理者分别领取，互不消费。Reaped 且无人待领取时再移除成员记录；Team 等待者继续寻找其他成员。Pending、超时、错误不移除。不向用户缓冲写结果，避免移除后写回失败。

已固定的观察不受移除影响。最后待领取项清除且记录移除后，新的观察不保证还能找到记录；查不到为 Denied，不能推断 reason=0。非消费观察可重复的保证以记录仍有效为边界，不引入永久墓碑。

Team 搜索可无分配遍历稳定子树元数据、按成员次序扫描本管理者待领取的 Reaped，**不承诺完成时间顺序**，无需完成序号或第二张队列。观察 Team 会重复看到同一可用项，直到领取。暂时无 Reaped 返回 Pending，不表示 Team 已结束；Ready 空域也可再次 Spawn。

POLL 立即检查；AtMost 使用一个固定 deadline；无关唤醒后重查；登记与检查闭合漏唤醒。Forever 不因一次无关唤醒返回 Pending。旧 Join 挂起后允许 false，本稿等待后重查是明确行为加强。等待自身非 POLL 拒绝。

## 5. 创建、收尾、领取与清理

```text
Spawn → 同步发布 Held 与成员记录
退出  → 同一 TaskState 转 Reaped，唤醒 Join
领取  → 返回 TaskExit，移除本管理者待领取项
最后待领取项清除且 Reaped → 移除成员记录
最后执行/成员/观察引用释放 → 回收状态存储
```

Spawn 在发布前预留状态、成员节点和可靠收尾工作链接；失败不发布，首次构造不提交。退出事实发布不分配、不覆盖旧结果；cause 来自实际路径，reason 原样保存，不从用户 reason 推断 Fault。首个已提交终止原因不被后续 reap 覆盖。

活成员记录不得移除。正常只有 Reaped 记录可被最后移除，因此已经移除的成员必然完成。owner 退出后不能把活记录当结果垃圾删除：先关闭创建并请求子域终止，活成员仍保留收尾元数据；清除已死 owner 的待领取项，Reaped 且其他待领取项也为空后自动 prune；其他管理者的项仍保留。迟到完成不恢复已清除的项。

各管理者存活时应持续领取，包括不关心返回值的完成任务。成员元数据及待领取集合均有额度，包含活记录、未领取 Reaped 记录及域深度；满额拒绝 Spawn，不能覆盖结果或失败退出。及时领取释放额度，调度延迟的暂存仍受预算约束。

Rust Join<T> 的 DONE/LEFT 只管理共享返回值 T。Drop 不调用 Oust(Task)，不产生额外 native 结果义务。DONE 后由父方释放；提前 Drop 的槽由用户态延期清理列表持有，后续 runtime 调用观察同域任务收尾后释放，覆盖未写 DONE 的异常退出。该列表不存 native 退出结果。无后续 runtime 调用时可能保留到 Team 物理回收；异常终止不承诺执行被中断闭包的 Rust 析构或释放其 TLS。内核退出结果仍由所属 Team 管理者统一接收。

## 6. Oust 与子树边界

Oust(Team) 在一个结构提交点检查直接所有权、全子树没有未收尾成员、没有进行中的创建提交，再关闭创建、标 Ousted、摘 heir。未领取但 Reaped 的记录不阻止 Oust；清除本管理者因该根建立的待领取资格，不清除其他管理者的项，最后待领取项为空才移除记录。同一管理者仍有其他有效 owned 根覆盖该任务时保留其项；Held 阻止成功。Busy 不关闭原本开放的域。

判断收尾时遍历稳定子树元数据：剩余成员只要都是 Reaped，就已收尾；已经移除的成员由前述不变量保证已收尾。不以“没有可领取结果”判断完成，也不维护第二枚完成位或另一个未收尾计数作为权威事实。

Oust 持有 operation 租约，先确认全子树 Reaped，锁外撤销 staging，再在 Unit 提交锁下发布 Ousted 并摘 heir；所有 Spawn 都使用同一租约。撤销失败保留所有权且不关闭创建，未释放的 staging 恢复。Oust 不等待物理回收，不删除已固定观察引用。

**祖先管理者可独立领取同一成员的退出结果。** 状态、成员记录和待领取集合由共享元数据保存，不强持有执行资源；后代 owner 先领取或 Oust，不删除祖先尚未领取的项。需要保留这一接收进度，不能仅靠 TaskState::Reaped 推断谁已领取。移除 Task.results 消除的是独立结果表，不是多管理者接收所必需的信息。

## 7. 调用表与命名

| 调用 | 作用 |
| --- | --- |
| Build | 创建 Constructing owned Team |
| Spawn | 创建 Held Task，首次构造必要时提交 |
| Embark | 首次放行 Task，或撤 Task/Team 本地停止 |
| Debark | 停止 Task 或 Team 子树 |
| Slay | 请求终止 Task 及其 heir，或 Team 子树 |
| Join | 等待、观察或领取完成结果 |
| Oust | 放下已收尾直接子 Team |
| Scan | 分页枚举调用者直接 heir TeamId |
| SelfId | 当前 TaskId |
| Sire | 当前 Team 创建者弱溯源 |
| Fall | 仅 Accord 到达通知 |

共 11 个逻辑入口。执行门用 embark/debark 对偶，所有权关系用 adopt/oust；Slay 不伪造复活对偶。成员沿用 tasks、prune_tasks、publish 等词汇，状态查询从权威 enum 派生。扫描是否需要改变 prune_tasks 的调用者必须核对，不能旧路径自动清掉 owner 尚未领取的 Reaped 记录。

Scan 按递增 TeamId 返回严格大于 after 的一页直接子域；after=0 起始，容量建议 1..64，固定内核数组、锁外 u64 LE 用户写回。不是冻结快照，不维护 HeirCount。失败不改变 heir；ID 不复用，计数器耗尽拒绝创建。

Team 执行控制只允许直接 owned child；Slay(Team) 还需要 Doom FETCH。Room::Doom 原全局能力权限保留，通过共同机制实施，不被 Unit 更窄包装静默收权。

Debark 成功证明目标范围无 Running/Debarking/Doomed(Running) 占槽。Busy 可表示停止请求已保存，允许重试。Task Debarking 尚占槽时 Embark Busy；Team Embark 可取消本根暂停请求，不改变成员自己的停止。Slay 只证明终止请求可靠接受，不等 hooks；自 Slay 不返回。

## 8. 同步、迁移与验证

实施锁序：按 TaskId 排序 gate → Unit 结构提交锁 → scheduler owning lock。权威状态、成员记录、拓扑、admission 与移除在共同提交协议内修改。能力提交 gate→Unit 复核状态；进入 Doomed 后释放 Unit 再取得 gate 清理，不能反向等待。结构发布所需容量在提交锁外预留；Space 操作、用户拷贝和执行资源的析构不得跨提交锁。兼容的 messenger 唤醒信标及等待续约仍可能在 Unit 锁内准备元数据，不将整个退出/唤醒链描述为完全不分配。删除 AtomicU8 tag 后不得保留原无锁读 enum。

现有 control/unit/wait.rs、identity.rs、unit/observe.rs、unit/task.rs 保留非消费观察。Control 保存主 TaskId，持续领取 owned 根子树成员并决定主/辅助策略，删除 PR Status/Observe 聚合。旧 prune_tasks、调度撤除和 muster 清理必须与“Reaped 尚未领取”区分。Rust Join<T> 不新增 native 放弃接口。

ABI 已固定：Unit class 保持 1，Build/Spawn/Embark/Debark/Slay/Join/Oust/Scan/SelfId/Sire/Fall 使用 slots 32–42，旧 slots 0–31 拒绝。Task/Team 目标编码为两字 tag+id，Join 后续为 Wait+receive；返回 a0=cause（0 为 Pending，负值为错误）、a1=TaskId、a2=reason。Scan 使用 after TeamId、缓冲地址、容量（1–64），输出 LE u64 ID；ID 限在 1..isize::MAX，耗尽拒绝创建。所有程序镜像同步迁移。保留 Denied/Busy/OoM/BadEntry；BadImage 属用户 loader，reason 非零不是调用错误。

验证重点：领取前 Task 物理回收；观察/领取竞争；多次观察；领取授权与多管理者互不消费；耗尽与持续领取；owner 死亡后活成员可靠终止、Reaped 自动清理；Oust 对已领取成员正确判断；嵌套控制与子树接收范围、同管理者去重、后代先 Oust 不丢祖先项；Held 环拆除；Doomed(Running) 离槽；首次构造/Oust 竞争；deadline/漏唤醒；旧 ABI 拒绝与运行时 T 槽异常清理。

实现使用共享 Arc<SpinLock<TaskState>> 与 Arc<SpinLock<TeamState>>，gate 按 TaskId 排序后进入 Unit 提交锁，随后取得 scheduler/member 锁。Unit 锁不得跨 run/fetch 空闲等待或用户拷贝。成员额度和每节点子拓扑额度各 1024，深度 32。life 的父子强元数据链接由 Ousted+无待领取项+无子节点时 prune 拆除；它不持有执行资源。详见迁移记录。
