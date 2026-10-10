# Unit results 方案历史

> 已由 [当前统一稿](unit-domain-redesign.md) 取代。这里保留被撤回的 Task.results、多祖先结果视图及 Join(Team) 子树领取设计。

# Unit 域统一重设计：Team、Task 与 UnitCall

状态：设计评审稿，未改 ABI 或实现。以本稿为当前契约；[历史稿](unit-domain-redesign-history.md) 与其他局部提案仅供溯源。

最新收束：**Hear 并入 Join；Oust 仅放下 Team；域管理者及时领取结果，不新增独立的 Task 结果放弃调用。** Spawn 不给创建者附加一份必须释放的结果句柄。Join 保留非消费观察，并显式支持领取。保留原有词汇与分组字段，公共血缘枚举用 Scan。

TaskState/TeamState 各自唯一表达状态。Tag、completion、boarding、gate.closed、construction 等独立事实删除；结果索引引用同一 TaskState，不复制退出结果。

## 1. 历史依据与设计漂移

这里的“最初”按仓库可读历史核对，不把 PR 的 base 当成最初设计。

| 版本 | 原设计或变化 | 本次处理 |
| --- | --- | --- |
| d14deee1 | Team 是共享 Space 的任务容器；早期 Team 上同时存在 sire/heir，弱引用关系及全局反查 | 资源容器职责保留；早期引用结构不作为今天的授权模型 |
| f2274948 | heir 收束到 Task：强持有子 Team，兼作撑命、Spawn 授权与级联遍历；Team.sire 弱溯源 | 保留 Task 拥有子域这一裁决；区分溯源、所有权和稳定控制拓扑 |
| 7554b75c | Oust 与 adopt 对偶，只放下已收尾子域，不等待物理回收；历史明确指出 Join 的 Denied 无法证明死亡 | 保留收尾/回收边界，补结果记录寿命 |
| 4e2bb473 | 调用表是 Build(ELF) → Spawn(Held) → Hatch；Join 为收尾探测/等待；Fall 仅能力到达 | Held 和分阶段授权保留；不恢复内核 ELF loader |
| 92989517 | Build 改为空的 Constructing 域；映射构造能力；首次 Spawn 提交 staging | 保留用户态装载；给构造提交和关闭补状态与原子边界 |
| 1b5046b9 | Hatch 改为 Embark，首次放行与恢复合用；增加 Debark、Slay，Task 新增 stopped/parked | 保留动作词汇；首次放行和撤停止门分别定义 |
| master 4d664c99 | Team 构造字段与 Task 能力转授关系继续增长；heir/heirs 命名分别指子域和能力后代 | 保留 heir/heirs 原名，明确子域关系与能力转授关系的不同契约 |
| PR #5 35b3d73e | 加 DebarkTeam/EmbarkTeam/Status/Observe；Team 聚合首任务、故障与观察策略 | 撤销该聚合模型，按下述公共契约重做 |

需要纠正前稿：原 Join 注释明确只保证“调用开始时已完成”才返回 true，挂起后允许 false 再探测。本稿选择更强的等待后重查契约，这是显式 ABI 行为变更；不声称原实现已经具有该行为。

## 2. 对象边界与不变量

**Team 是资源域，Task 是执行与权柄主体。** 一个 Team 可以有多个 task，不存在内核默认主任务；第一个 Spawn 不赋予代表身份。Task 可拥有零到多个子 Team；同 Team 的线程并不共同拥有彼此的子 Team。

| 对象 | 拥有或负责 | 不负责 |
| --- | --- | --- |
| Team | Space、直接成员、构造提交、域级暂停门 | 命令名、主任务选择、故障严重性、测试期望 |
| Task | 执行上下文、能力表及转授关系、拥有的子 Team、单任务停止状态 | 将共享 Space 的其他成员当作自己的孩子 |
| TeamLife | 稳定域级父子拓扑、共享 TeamState 的引用、派生计数与完成队列视图 | 代替 heir 表授予控制权；强持有祖先 Space |
| TaskState | 执行状态与 Reaped 中唯一的退出事实；状态存储可越过执行对象寿命 | Rust 闭包返回值 T、诊断日志、物理回收证明 |
| Control/Job/测具 | 主任务与辅助任务策略、取消、期望退出与场景判定 | 修改内核退出事实 |

核心不变量：

1. Spawn 发布前准备完全部可失败资源；发布后返回的 TaskId 对应 Held，不能先运行。
2. Space 的寿命由资源持有者决定；结果和拓扑元数据不强持有已退出任务、栈、帧或 Space。
3. Task 的能力接收关闭后不再新增能力、子域或由该 task 发起的任务；退出级联范围不会继续增长。
4. 暂停不会改变 Held 或等待条件；恢复一种门不撤掉另一种门。
5. 所有 Join 的已完成结果都在退出 hooks 完成后可见；诊断可以更早记录，但不能充当该证明。
6. ID 只定位，所有权或能力才授权。TaskId/TeamId 不复用；计数器耗尽应拒绝创建，不能绕回变成其他对象。

## 3. 血缘、所有权与寿命

Team 记录创建者 TaskId 和弱的创建者引用。Sire 继续查询“当前 Team 的创建者仍在世时的 TaskId”，顶层或父已亡为 0；同域后创建线程不会因此把 Spawn 调用者变成 Sire。Spawn 创建者不因此自动得到一份独立结果持有项；结果接收归域管理者，不混用 Team.sire。

Task 的 `heir` 强持有它 Build 出的 Team，是域级控制凭证。Team 的成员表用弱引用；Held 的强引用放在同一成员管理对象内。Task 退出必须先关闭创建入口，再终止其 heir 子树并撤销自身资源；不杀共享 Team 中的其他线程，除非该 Team 又位于退出者拥有的子域范围。

沿用 life 词汇，选择独立的 TeamLife 元数据节点：Team 强持有自己的节点；节点强持有父节点、弱索引子节点，不持有 Team/Task/Space 的强引用。父 Task 或父 Team 物理回收以后，后代的暂停继承、计数和结果传播链仍存在。父子双向不构成强引用环。节点保存创建者 ID 仅作事实，授权仍查活调用者的 heir 或既有 Doom 能力。

创建与移除父子关系、计数变化、关闭状态都在拓扑提交协议中完成。已 Ousted/Doomed 的祖先不接纳新的 Build/Spawn。退出路径不临时申请遍历 Vec：每个已发布对象携带预留的收尾工作链接，逐项提交、逐项释放；不能用“快照 OOM 返回空集合”跳过级联。

## 4. 两条状态轴

### Team 生命周期

```text
Constructing --首次 Spawn 提交--> Ready
Constructing/Ready --Debark--> Debarking --> Debarked
Debarking/Debarked --Embark--> 原 Constructing/Ready
任意未放下状态 --子树终止或 owner 退出--> Doomed
Constructing/Ready/Doomed --Oust，子树未收尾计数为 0--> Ousted
Ousted --最后资源引用释放--> 物理回收
```

Ready 且当前无成员不等于永久结束，owner 可再次 Spawn。Constructing 可以 Oust，但必须先撤销 staging。Doomed 是不可逆的拒绝创建状态；暂停不是 Doomed。Oust 的成功提交同时关闭创建并摘除 owner 的 heir 项，不存在“检查空 → 新成员进入 → 摘掉域”的窗口。Busy 不悄悄关闭原本 Ready 的域。

### Task 生命周期与调度状态

```text
Held --Embark(Task)--> 调度可接纳
调度可接纳 <--> Running <--> Blocked
任意未收尾状态 --主动退出/Slay/级联/fault--> Doomed
Doomed --hooks 完成--> Reaped
Reaped --延迟埋葬--> 物理回收
```

沿用 Starved/Running/Blocked/Held/Doomed/Reaped 词汇，但不要求维持原来的字段布局。单任务停止由 TaskState 的 Debarking/Debarked 表达，不再保存 stopped 或 boarding。祖先 TeamState 的暂停约束和 Held 首次放行条件独立；Debarking 不证明已经离槽。Parked 仅表示域级暂停造成的可运行任务停放位置；收尾事实只发布为 TaskState::Reaped { cause, reason }，不再另设 completion。

## 5. 字段按职责重建

恢复上一版按职责分组的字段设计；保留词汇不要求保留扁平存放位置。以下是逻辑结构，不是可直接编译的 Rust。分组名称沿用 state、tasks、gate、heir、life 等既有词根；不引入 origin/authority/children 等替换词汇。具体容器与锁级别仍需按提交边界验证。

```rust
Team {
    id: TeamId,
    space: Arc<Space>,
    sire: TaskWeak,                  // 创建者溯源
    state: Arc</* 同步保护的 TeamState */>, // 阶段与该阶段载荷的唯一存储
    tasks: Tasks,                    // 成员弱引用与 held 强引用
    life: Arc<TeamLife>,             // 稳定祖先关系、共享 state 引用与完成视图
}

Task {
    ident: Arc<TaskIdent>,
    life: Arc<Life>,
    state: Arc</* 同步保护的 TaskState */>, // 可独立于执行对象存活
    gate: Gate,                     // 能力锁域：version、pies、heirs；不存 closed
    heir: Heir,                     // 拥有的子 Team
    results: Results,               // owned 域的结果接收索引
}
```

| 分组 | 内部事实 | 同步与寿命边界 |
| --- | --- | --- |
| Team.state | Constructing/Ready/Debarking/Debarked/Doomed/Ousted 及载荷 | 唯一域阶段；staged/default_entry 随分支归属，不另存 construction/ready/closed |
| tasks | tasks 弱引用、held 强引用 | 同一成员发布/首次放行/移除协议，避免两个列表分别提交 |
| TeamLife | 祖先关系、共享 TeamState 引用、完成视图 | 不复制域阶段；Ousted 状态不保留资源；计数仅是待证明的派生索引 |
| state | Held/Starved/Running/Blocked/Parked/Debarking/Debarked/Doomed/Reaped 及载荷 | 唯一状态与退出事实；不另存 tag、completion、boarding 或 stopped |
| gate | version、pies、heirs 及其同步保护 | 不存接收关闭布尔值；从 TaskState 派生接收条件，修改提交与状态检查原子协调 |
| heir | owned Team 项 | adopt/oust 对偶；创建、退出与级联时参加域结构提交协议 |
| results | owned 域成员的 ID、TeamId 与共享 TaskState 引用 | 只管理域结果接收，不复制 cause/reason；Join 观察或领取，Oust(Team)/owner 退出清理 |

- 恢复分组，撤回上一轮将 ready/operating/staged、version/pies/heirs 等全部铺回顶层的方案。旧字段词汇在所属分组内保留，不要求旧访问路径不变。
- 删除 Team 与 Task 上独立的 completion，以及 observed、observer、representative；退出事实归入 TaskState::Reaped。删除泛称 revision/changed 的乐观快照控制方案。
- 引入 TeamState 作为域阶段的唯一真值，删除 construction 分组与 ready/closed 等独立阶段位。Ready 变体携带 default_entry，Constructing 变体携带 staged；阶段查询不能重新保存一份缓存。
- 原 operating 的互斥职责归入域结构提交协议，不另存 operating 位或以 Constructing 表示“有人持锁”。阶段是事实，同步锁是修改事实的保护，不能互相替代。旧 Construction guard 随同步协议迁移，不新增同名状态容器。
- 分组不等于把每组都换成一把大锁。Constructing 载荷的资源准备、tasks 的结构提交、gate 的能力复核分别遵守既定锁序；Space 分配、析构和 Drop 不进入域结构提交锁。
- heir 仍指 Task 拥有的子 Team；gate.heirs 仍指能力转授后代，依类型与所属分组区分，不改为 children/grants。
- tasks 中的 held 保留必要强引用，与 TaskIdent.team 形成受管理的暂时保活环；首次放行、Slay 或 owner 退出必须拆除，Oust 不能绕过 Held 检查。
- sire 与 TeamLife 分别承担弱溯源和稳定祖先关系，不把 sire 升级为强资源引用。

### TaskState 与 TeamState 联合设计

两者同为各自对象的唯一状态，动作词汇对称，载荷按职责区分。下面是有限载荷示意，不是递归 enum，也不是直接可编译的 Rust。

```rust
enum TaskState {
    Held,
    Starved { next },
    Running { hart, ticks_left },
    Blocked { key, ticket, next },
    Parked { next },
    Debarking { hart, ticks_left },
    Debarked { state },           // 仅 Held/Starved/Blocked
    Doomed { state, cause, reason }, // 尚待移除的位置，Held/Starved/Running/Blocked/Parked 或已移除的 None
    Reaped { cause, reason },
}

enum TeamState {
    Constructing { staged },
    Ready { default_entry },
    Debarking { state },          // 仅 Constructing/Ready
    Debarked { state },           // 仅 Constructing/Ready
    Doomed { staged },
    Ousted,
}
```

| 方面 | TaskState | TeamState |
| --- | --- | --- |
| 初始阶段 | Held：尚未首次放行 | Constructing：尚未首次构造提交 |
| 活动阶段 | Starved/Running/Blocked/Parked：具体执行位置 | Ready：已构造，不代表成员一定在运行 |
| 停止请求 | Debarking：仍占运行槽，需离槽 | Debarking：本根已禁止新入槽，需确认整个子树无运行成员 |
| 已停止 | Debarked：保留首次放行或等待条件 | Debarked：保留构造载荷，暂停约束覆盖子树 |
| 终止中 | Doomed：接收已关闭，但可能尚有执行位置待移除 | Doomed：拒绝新创建，成员逐项终止，撤销尚存 staged |
| 收尾/放下 | Reaped：hooks 完成，保存本 task 的唯一退出结果 | Ousted：子树已收尾且 owner 已放下，不聚合成员退出结果 |

**停止不丢载荷。** Task 的 Debarked 保留 Held/Starved/Blocked；Team 的 Debarking/Debarked 保留 Constructing/Ready。首次 Spawn 可将暂停包装内的 Constructing 转为 Ready，但保持原暂停约束；返回 task 仍为 Held。包装是有限的内层分支，不允许 Debarked 包 Debarked，也不引入 Box 或退出时分配。

**停止分两种范围。** Task 的本地停止表示为 Debarking/Debarked。Team 的本地停止也表示为 Debarking/Debarked，并沿稳定祖先链影响成员 admission；不再保存 TeamLife.paused。Task 因祖先暂停而停放使用 Parked，不因此变成本地 Debarked。恢复 Team 不改成员的 Debarked，恢复 Task 也不改祖先 TeamState。

**终止不丢执行位置。** 不能把仍在运行槽的 Task 直接改成只有 cause/reason 的 Doomed，否则调度器找不到该 nudged hart。Doomed 携带尚待移除的位置；由 Debarking 进入时保留 Running 位置，由 Debarked 进入时保留其 Held/Starved/Blocked 位置。该位置与原调度容器在同一提交点移除，Doomed 的位置载荷转为 None；此后才能执行完整收尾并转 Reaped。None 只表示已无调度容器，不代替 hooks 完成事实。不另存 doomed 名册作为第二份终止请求真值；必要工作队列只引用这个状态。

**构造只提交一次。** 首次 Spawn 在同一提交点消费 staged、确定 default_entry，并将 Constructing 转为 Ready；若有暂停包装则只转移其内部载荷。失败保持原状态和所有权，不另写 ready=true。Team 进入 Doomed 后不再使用 default_entry；只保留尚待撤销的 staged，来自 Ready 时为空。释放 Space/预留资源仍在提交锁外进行。

**末态不等于物理回收。** TaskState::Reaped 无队列链接、等待票或 Task/Team/Space 强引用；TeamState::Ousted 无资源载荷。状态存储可以被结果拥有项、队列视图或后代节点保留；Task/Team 及执行资源可先回收。Team 不需要一个聚合 Reaped 来冒充“主任务已退出”，Task 不用 Ousted 表示其结果 owner 已放弃。TaskState::Reaped 是执行主体完成事实，TeamState::Ousted 是 owner 关系已撤去的事实，不能为形式对称而互换。

**查询只派生。** 本地暂停从该对象的 Debarking/Debarked 派生；有效域暂停沿祖先状态查询。Doomed/Ousted 同时禁止创建或 admission，但不伪装成已暂停；Reaped 的执行对象不再 admission。状态锁序和原子提交协议仍须验证，不能仅通过增加变体解决并发。

### 同一事实只存一处

这是字段设计的默认约束。分组不能掩盖重复真值；同一把锁下更新两个字段，也不能作为保留两份事实的充分理由。

| 原先分散的内容 | 收敛决策 |
| --- | --- |
| state 与 AtomicU8 tag | 删除 tag 存储；需要状态类别时，在状态同步保护下从权威状态派生。TaskTag 如保留，仅作查询返回值，不能写回成为第二份状态 |
| Starved 与 boarding.parked | 为停放使用 TaskState 内的 Parked 分支，沿用已有词根。Starved 明确表示可运行队列位置，不再同时表示“可能已停放” |
| stopped 与实际暂停状态 | 删除 stopped 和 boarding。Running 收到停止请求转为 Debarking，确认离槽后转为 Debarked；非运行目标直接转为 Debarked，并保留 Held 或等待载荷 |
| TaskState::Held 与 held 容器 | Held 是首次未放行的唯一状态；held 仅是强引用持有容器，不允许凭“是否在 held 中”另作状态判断。从 Held 转移与引用移交同一提交，不单独改变其中一项 |
| TaskState::Reaped 与 completion 完成位 | 删除 completion；hooks 完成后仅转入 Reaped { cause, reason }。状态存储由结果拥有项及队列引用保留，Task 执行对象仍可回收 |
| gate 中的接收关闭位与 TaskState | 删除 closed 布尔事实；Doomed/Reaped 拒绝新增能力、子域及发起 Spawn，其他状态不因暂停而关闭接收 |
| construction/ready/closed 与 Team 阶段 | 收束为唯一 TeamState；从变体推导是否允许构造、Spawn 或 Oust，不另存阶段布尔值 |
| paused_local 与 paused_effective | 删除本地 paused 布尔值；从 TeamState::Debarking/Debarked 派生，本地/继承查询均不存镜像 |
| Team 完成聚合与各 Task 退出结果 | 只存每个 TaskState::Reaped 的完成事实；队列视图引用同一状态，Control/Job 的汇总不能再反写内核结果 |

TaskState 的存储通过共享引用独立存活，不能把它留在会随 Task 回收的内存内。正常退出或强制终止先进入 Doomed { state, cause, reason }，hooks 完成后将同一存储转为 Reaped { cause, reason }，不是复制到第二个对象。最终状态不可逆且不再携带队列链接、等待票、Task/Team/Space 强引用。待埋葬链接归回收容器，不能放入持久 Reaped 结果中。

results 与各根完成队列只保存 ID 和同一状态存储的引用；域接收索引、执行与临时读取引用都释放后，状态存储才释放。Task 的栈、帧及资源对象无需等待它。Join 返回的 TaskExit 是传输快照，不是另一份可独立修改的内核真值；Join 领取时返回完整退出结果并移除域接收索引，不移交结果句柄。原 Life 仅负责等待登记资源，不再以弱引用失效或另一枚存活位推断完成。

Parked 的强引用由停放容器持有，不能在自己的 TaskState 中存 Arc<Self>；state 只保存该位置需要的链接。Debarked 的载荷是有限的 Held/Starved/Blocked 分支，不是任意递归 TaskState，也不需要 Box 或新分配；不能包入 Running、Debarking、Doomed、Reaped 或另一层 Debarked。Debarked(Blocked) 收到真实唤醒时转为 Debarked(Starved)，更新并移除原等待票但不入队；Embark 后根据域门进入 Starved 或 Parked。域暂停时仍在等待的 Blocked 保留等待状态，真实唤醒后才进入 Parked。

运行 hart 若存于 Running，running_hart 从该权威状态读取，不再靠另一张可独立修改的反查表推断；hart 的当前槽仍需持有执行引用，但其关联只在同一提交点转移，不是另一个可写的 Running 真值。

运行/未收尾计数暂从正确性必需字段降为待证明的派生索引。第一版优先在域结构同步保护下无分配遍历权威状态来确认 Debark/Oust；运行位置检查包含 Running、Debarking 以及 Doomed 内尚存的 Running 位置；只有性能证据要求时才加计数，并规定唯一更新入口、溢出检查及与状态重算的检查。无锁镜像、冗余完成位和计数都不能仅为读取方便加入。

删除 tag 后必须重新检查原来无锁读 tag 的每个调用点、Task::exclusive 的可变访问与状态锁序；不能只把 AtomicU8 删掉然后无锁读带引用的 enum。这里是设计选择，尚未完成调度器实现验证。

### gate 只负责能力同步，不另存生命周期

原 `gate: SpinLock<bool>` 中的 bool 表示退出清理已开始。新设计由进入 TaskState::Doomed 唯一表达这个边界，Reaped 继续保持关闭；不再保存第二枚 closed/accepting。Held、Blocked、Parked、Debarking、Debarked 都不等于退出，仍可按原授权接收能力。

gate 中的 pies、heirs 是能力与转授关系，version 是能力关系复核所需的派生版本；它们不是互斥执行状态，不搬入 TaskState 的某个分支。gate 若保留，表示这一组数据的同步容器，而不是另一个状态门。

必须闭合“检查状态 → 发布能力”的竞态：不能在 gate 锁下读一次非 Doomed，释放状态同步后继续插入。候选提交锁序为按 TaskId 排序的 gate 锁 → Unit 域结构提交锁 → scheduler owning lock；能力提交在同一个域结构提交范围检查涉及任务的权威状态并修改关系。进入 Doomed 可以仅持域结构提交锁完成，随后释放它，再获取 gate 做退出清理；不得在持有域结构或调度锁时反向等待 gate。这样能力提交与退出状态转移只能一方先完成。

资源准备和可失败分配在上述提交范围外完成；锁序需要全量 lockdep 验证。此处保留 gate 锁域的必要性，不等于现有 SpinLock<bool> 可以原样保留，也不表示把全部能力工作都放进 TaskState 的锁。

## 6. 统一调用表

```text
Join(目标, 等待期限, 是否领取) → Pending / Reaped(TaskExit)
TaskExit = { task, cause, reason }
目标 = Task(TaskId) / Team(TeamId)
```

“是否领取”是语义参数，具体编码随完整 ABI 表评审，不使用含糊的默认值。Task 目标选指定任务；Team 目标选调用者在该 owned 根子树中的下一份完成结果。观察不移除，领取成功移除；Team 非消费观察重复返回当前队首，不偷偷推进游标。

| 调用 | 目的 |
| --- | --- |
| Build | 创建 Constructing 空 Team，挂入调用者 heir |
| Spawn | 在当前 Team 或直接 owned Team 创建 Held Task；必要时首次提交构造 |
| Embark | Task 首次放行/恢复本地停止；Team 撤本根暂停 |
| Debark | 停止 Task 或 owned Team 子树，成功确认离槽 |
| Slay | 请求终止 Task 及其 owned 子域，或整个目标 Team 子树 |
| Join | 等待并观察或领取任务退出事实 |
| Oust | 放下已收尾的直接 owned Team；不支持 Task 目标 |
| Scan | 分页枚举调用者的直接 heir TeamId |
| SelfId | 当前 TaskId |
| Sire | 当前 Team 创建者弱溯源，保留原义 |
| Fall | 仅 Accord 到达通知 |

共 11 个入口。不增加 Hear、Collect、Status、Observe、DebarkTeam、EmbarkTeam、Heir、HeirCount。Pie::Collect 不变；内部 heir/heirs 保留。Build 与 Spawn 之间仍可用户态装载、映射、授予能力。

## 7. 授权与寿命

Build、Spawn 与 Task 执行控制保留原授权：Build 需要 Build FETCH；Spawn 允许当前 Team 或直接 owned child；Embark/Debark/Slay(Task) 允许同 Team 或直接 child Team。Team 暂停/恢复只允许直接 owned child，不允许暂停包含自身的根；Slay(Team) 还要求 Doom FETCH。

Join(Task, 不领取) 保留同 Team/直接 child 的观察权限；域管理者还可观察其域接收索引中的任务。Join(Task, 领取) 只能移除调用者域接收索引中对应项，同 Team 的控制权限不能领取别人的结果。Join(Team) 只访问调用者直接 owned 根的结果流，覆盖该根及后代域，不代表聚合退出。Scan/Oust 仅操作调用者 heir。

ID 只定位，不授权。索引持弱引用；活执行对象、未领取的域结果和已登记观察持状态引用。领取后，已固定引用的观察继续有效，但不承诺以后永久可按 ID 查询；记录已释放或无权为 Denied，不能推断成功退出。Join 自身非 POLL 拒绝，POLL 可观察 Pending。

Room::Doom 原有凭 Doom 能力终止任意目标所属域的权限保留，调用共同终止机制，不被更窄的 Unit Team 包装静默收权。

## 8. 暂停、恢复与关闭

Task Embark 首次从 Held 放行；Debarked(Held) 同样首次放行。Debarked(Starved) 根据祖先门进入 Starved/Parked；Debarked(Blocked) 恢复真实等待。Debarking 尚占槽返回 Busy，离槽后才能恢复；原已运行而未停止目标保持原 Busy 行为。终止或已完成拒绝，不增加 Embarked 标志。

Team Embark 仅撤本根 Debarking/Debarked 包装，可取消未确认的域暂停；不放行 Held，不改变成员本地 Debarked，不撤后代根自己的暂停。与 Task 的首次放行载荷不同，不能为形式对称删掉这些区别。

Debark(Task) 将 Running 转 Debarking 并推动离槽；其他可停止状态转有限载荷 Debarked。Debark(Team) 设根 Debarking 并阻止子树新 admission，确认无占槽成员后转 Debarked。占槽检查包含 Running、Debarking、Doomed(Running)。Busy 明示停止请求已经生效，允许重试确认；重复 Debark 幂等。自暂停 Task 在切换前准备成功返回帧，恢复后从原调用继续。

Team 恢复只保证暂停约束已撤且 Parked 成员纳入调度协议，不保证所有成员已经运行。暂停不阻碍终止与埋葬。Slay 接受后不可因 OoM 丢请求；重复终止幂等，首个已提交 cause/reason 不被后续路径覆盖。Slay(SelfId) 不返回用户代码，正常退出仍走 Room::Reap。

Oust(Team) 在一个提交点验证所有权、无未收尾后代、无正在发布的构造操作，再关闭创建、标 Ousted、摘 heir 并丢弃本 owner 的子树结果项。Held 属于未收尾。Busy 不关闭原本开放的域。Constructing 的 staging 预先固定并验证撤销资源；失败不提交，成功后锁外释放。Oust 不等物理回收，也不删除其他 owner 的结果项。

## 9. 域结果接收，而非单 Task 结果句柄

TaskState::Reaped { cause, reason } 是唯一退出事实。域管理者在 results 中接收 owned 根及其后代的完成结果；普通 Spawn 创建者不会因此再得到一份必须主动释放的 native 结果项。通过 Join(Task) 等待与观察不等于拥有待回收的结果句柄。

同一个 owner 对同一 Task 最多一个接收项；指定 ID 查询与各 owned 根队列只是索引。不同域管理者有各自视图，引用同一 TaskState，互不消费；只有实际负责域结果的管理者拥有视图，不给所有观察者自动复制订阅。

Spawn 发布前预留所需状态和域接收链接；失败则不发布、首次构造不提交。hooks 完成后转 Reaped 并使各接收项可领取，退出路径不分配。cause 来源于实际退出路径，reason 保留原值；非零 Reap 不伪装 Fault。

Join 观察可重复、多等待者，既不移除接收项也不改变状态。Join 领取成功在同一提交点返回完整 TaskExit 并移除本 owner 对应接收项及所有队列索引；Pending、超时、错误不消费。并发领取指定项仅一人成功，其余为 Denied；Team 等待者可继续竞争其他项。返回使用寄存器，不向用户缓冲写回，避免摘项后写回失败。

**管理者及时领取，记录只为等待接收而保留。** 调度延迟仍会产生暂存，因此有域深度、接收额度和全局记录预算；满额原子拒绝创建，不覆盖旧结果、不让退出失败。领取释放本 owner 额度，最后资源/接收/观察引用消失后回收共享状态。长期存活的域须持续接收，不能只观察不领取却无限积累。

owner 退出先关闭新增入口并触发 owned 子域终止，再清理其接收索引；迟到完成不重建索引。Oust(Team) 成功后清理该 owner 对子树的剩余项，不影响其他管理者和已固定的观察。未收尾成员只能在 Reaped 后从活动索引移除，确保 Oust 的收尾判断不依赖已消费结果。

```text
Join(Task 101, POLL, 不领取)      → Pending
Join(Team 10, Forever, 领取)     → Reaped(102, Reap, 17)
Join(Task 101, Forever, 不领取)  → Reaped(101, Reap, 0)
Join(Task 101, POLL, 不领取)     → Reaped(101, Reap, 0)
Join(Task 101, POLL, 领取)       → Reaped(101, Reap, 0)
Oust(Team 10)                   → () // 整棵子树收尾后
```

示例调用者是 Team 10 的域管理者。若不知道哪一个先完成，持续 Join(Team, 领取) 即可；不需先读 TaskId、再查结果、再释放句柄。队列暂空或主任务退出都不能证明子树已结束。

Rust Join<T>::Drop 仅处理共享返回值 T 的 DONE/LEFT 协议，不调用 Oust(Task)。native 退出结果仍由域管理者领取。异常退出前未写 DONE 时的 T 槽清理需运行时单独闭合，不能把内核结果领取当成共享内存释放。

## 10. Scan 与等待协议

Scan(after, buf, capacity) 返回按 TeamId 递增、严格大于 after 的直接子域一页。after=0 开始，下一页使用上页最后 ID；即使该项随后 Oust 也可继续。written=0 表示该次读取没有更多，不保证跨页冻结快照，不维护独立 total/count。ID 不复用且单调，汇总页数不是变化期间某一瞬间的精确数量。

建议每页 1..64 项，固定内核数组，u64 LE 写 TeamId；校验容量、地址范围与长度溢出。在结构锁下取一页，锁外写用户缓冲；失败不改变 heir，调用者丢弃错误调用的缓冲。确切失败编码和地址验证复用现有内存 ABI，阶段 4 再核定，不新增快照句柄。

Join 的 POLL 立即检查；AtMost 使用单一固定 deadline；无关唤醒后重查并以剩余时间继续等待；Forever 不因一次无关唤醒返回 Pending。检查与登记在同一同步协议内闭合漏唤醒；超时/完成竞态在同一提交点裁决。Join(Team) 空队列既不表示域永久结束，也不表示所有成员成功。

旧 Join 只保证进入时已完成才能返回 true，挂起后可能 false；本稿等待后重查是显式加强契约，不声称原实现已有此保证。

## 11. 同步与方法对偶

第一版采用专门 Unit 结构提交锁保护权威状态、TeamLife 拓扑、成员发布、结果项及索引。调度 admission、队列位置及离槽也参加该提交协议。它不覆盖 Space 分配、用户拷贝、诊断或资源析构，不恢复旧全局能力 GRAPH。

目标锁序：按 TaskId 排序的 gate → Unit 结构提交锁 → scheduler owning lock。无能力修改的状态转移从 Unit 开始。进入 Doomed 后释放 Unit，再取得 gate 清理；持 Unit/调度锁不能反向等 gate。能力提交在 gate→Unit 下复核状态并发布；前置资源预留在锁外。删除 tag 后所有 enum 访问必须进入同步，不能保留原无锁读法。

状态与 held/队列持有引用、结果项与其索引在同一提交点转移。计数只可作可重算派生索引，首版优先无分配遍历权威状态。退出工作链接在发布前预留；遍历有深度上限，不以 OOM 空快照跳过级联。staging 的预检、版本复核和提交后锁外释放必须闭合，提交后的合法资源释放不允许再以可恢复错误留下半个 Ousted。

| 职责 | 方法命名 |
| --- | --- |
| 子域拥有关系 | 沿用 adopt / oust |
| 本地执行门 | embark / debark，Task 与 Team 共用词汇 |
| 能力关系 | 沿用 pies / heirs 及原转授词汇；gate 不存 closed |
| 状态修改与查询 | 沿用 transform / state；派生查询不存 tag 或 completion |
| 结果队列 | push / pop；Join 领取，Oust(Team) 清理剩余域结果，不造 complete/observe 旁路 |
| 构造 | staged / cancel_staging；首次提交 publish，不伪造可逆操作 |
| 血缘枚举 | 公共 Scan；内部 heir/heirs，不改成 children |

Slay 不可逆、Spawn 与首次 publish 不可撤回成未发布状态；不为表面命名对称造复活操作。局部/祖先暂停查询必须明确范围，可沿用 paused_local/paused_effective 名称，但只从状态派生。具体 lockdep 等级与所有旧锁边尚未通过实现验证；这是阶段 4 前的必要核对。

## 12. 调用者迁移与后续验证

PR #5 的 control/unit/wait.rs 保留重复观察；identity.rs、unit/observe.rs、unit/task.rs 的存活探测用非消费 Join。Denied 可使策略拒用目标，不产生虚构退出原因。instance/mod.rs、instance/hook.rs 移除 Status 聚合，Control 保存主 TaskId，领取每份成员退出事实并自己判断策略。lifecycle/ruin.rs 保留观察→Slay→确认收尾→Oust(Team) 的边界。reap.rs 的 HeirCount 日志改 Scan 或删除精确数量假设。Rust Join<T> 不增加 native 结果释放义务。

Unit class 保持 1；具体 slots、目标/领取参数编码、ret3 布局、ID 上限及错误编码尚未固定。旧编码必须明确拒绝，kernel 与程序镜像同步迁移，不能静默重解释。保留 Denied/Busy/OoM/BadEntry 等词汇；BadImage 属用户 loader，reason 非零不是调用错误。

当前设计需要评审的具体选择是统一 Join 的显式领取参数与域结果授权；不是已经验证的实现。后续先核定 ABI、lockdep、预算与运行时 T 槽清理，再改内核/包装/Control/Job/测具。

验证重点：观察与领取竞争、领取后记录寿命、多管理者视图、owner 退出与迟到完成、长期域持续接收、额度满时原子拒绝创建、退出无分配、固定 deadline 和漏唤醒；以及 Held 保活环、嵌套暂停、Doomed(Running) 离槽、构造/Oust 竞争、旧 ABI 拒绝、主任务成功时辅助 fault 不丢失。

本轮仅更新设计文档，未运行 Rust/QEMU 实现测试。
