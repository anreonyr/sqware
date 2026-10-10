# Unit 域设计历史稿

> 已由 [统一评审稿](unit-domain-redesign.md) 取代。以下含已撤回的 Collect、消费式 Join 和编号建议，禁止作为实现契约。

# Unit 域统一重设计：Team、Task 与 UnitCall

状态：设计评审稿，尚未修改 ABI 或实现。对象状态的单一事实原则继续成立；调用表与结果协议重新评审。撤回将 Collect → Join → Oust(Task) 定为推荐流程的结论，不能据此实施。第 6–13 节保留上一轮方案供定位问题，其入口、权限、预算、编号与迁移计划都不是当前定稿。

新的收尾替代方案见 [Unit 收尾接口新方案](unit-result-proposal.md)：采用 Hear（原提案 Collect）命名，Join/Hear 返回同型结果并领取，不增加 Oust(Task)；撤回 Heir/HeirCount 公共入口，以 Scan 批量枚举直接子 Team。Hear 领取完成结果，Scan 枚举子域，不混合这两个职责。旧 Join 的观察用途与未完成单项放弃需求仍需逐个核对。

命名约束：保留原有词汇。重设计调整职责、寿命、同步和载荷，保留已有字段/方法/调用的词汇，字段可以按职责分组并调整访问路径；前稿提出的 children/authority/Lineage、pause/resume 和快照改名已撤回。

## 当前评审判断：结果接口需要重做

上一轮为解决“回收后仍能读取结果”，引入了 results 拥有项；随后为了发现任意后代完成，又设计 Collect 移交拥有项、Join 读取、Oust(Task) 放弃。这条三步协议可以被形式化，但它尚未证明是符合 Unit 原职责的最简接口。它把内部记录寿命问题外露成用户必须管理的新对象，因此撤回其推荐地位。

| 部分 | 当前判断 |
| --- | --- |
| Build/Spawn/Embark/Debark/Slay | 沿用既有创建与执行词汇；Team/Task 范围、权限与完成边界仍需逐项验证 |
| TaskState/TeamState 单一事实 | 保留设计原则；具体变体、载荷、引用及锁序仍是待验证方案，不因移入 enum 就自动正确 |
| Join | 保留指定任务收尾等待的职责；返回退出事实是合理候选，非消费/消费及记录寿命尚不能绕过评审 |
| Oust(Team) | 保留原来放下子域的含义，不杀、不等物理回收 |
| Oust(Task) | 撤回默认扩展；放弃结果与原来放下子域是否应共用入口，需要重新论证 |
| Collect | 撤回默认新增；“下一成员完成”需求真实，但不能先创造结果拥有项，再以该模型证明必须增加调用 |
| HeirCount/Heir | 数量查询与血缘枚举成立；不把两次调用当成快照。ID 游标是可选契约改进，不作为结果协议前提 |

下一步应从调用者真正要完成的两件事出发：等待已知 Task 并取得退出事实；在不知道哪个辅助任务先退出时，可靠获知下一次完成。比较有类型的指定等待/任意完成等待与独立收集入口，并明确结果读取是否消费。不能将任意完成解释为整个 Team 完成，也不能让读结果授予任务控制权。

结果寿命应先由创建者、域所有者和收尾消费协议闭合，再决定是否确需独立释放接口。不能依赖无限保留、覆盖诊断环、任务失踪推断退出成功，或未授权的自愿消息。若最后仍需显式结果拥有项，必须说明它解决了哪项无法由域寿命或领取协议解决的需求，才能恢复该方案。

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
5. 所有 Join/Collect 的已完成结果都在退出 hooks 完成后可见；诊断可以更早记录，但不能充当该证明。
6. ID 只定位，所有权或能力才授权。TaskId/TeamId 不复用；计数器耗尽应拒绝创建，不能绕回变成其他对象。

## 3. 血缘、所有权与寿命

Team 记录创建者 TaskId 和弱的创建者引用。Sire 继续查询“当前 Team 的创建者仍在世时的 TaskId”，顶层或父已亡为 0；同域后创建线程不会因此把 Spawn 调用者变成 Sire。Task 的 Spawn 创建者另存于结果所有权记录，不混用 Team.sire。

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
    results: Results,               // Spawn 结果拥有项
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
| results | 所创建任务的 ID、TeamId 与共享 TaskState 引用 | 仅管理结果所有权，不复制 cause/reason；Join 非消费、Oust 放弃、owner 退出清理 |

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

results 与各根完成队列只保存 ID 和同一状态存储的引用；结果 owner 放弃、队列消费及临时读取都结束后，状态存储才释放。Task 的栈、帧及资源对象无需等待它。Join 返回的 TaskExit 是传输快照，不是另一份可独立修改的内核真值；Collect 只移交对同一状态的结果拥有项，不另返回或保存退出结果。原 Life 仅负责等待登记资源，不再以弱引用失效或另一枚存活位推断完成。

Parked 的强引用由停放容器持有，不能在自己的 TaskState 中存 Arc<Self>；state 只保存该位置需要的链接。Debarked 的载荷是有限的 Held/Starved/Blocked 分支，不是任意递归 TaskState，也不需要 Box 或新分配；不能包入 Running、Debarking、Doomed、Reaped 或另一层 Debarked。Debarked(Blocked) 收到真实唤醒时转为 Debarked(Starved)，更新并移除原等待票但不入队；Embark 后根据域门进入 Starved 或 Parked。域暂停时仍在等待的 Blocked 保留等待状态，真实唤醒后才进入 Parked。

运行 hart 若存于 Running，running_hart 从该权威状态读取，不再靠另一张可独立修改的反查表推断；hart 的当前槽仍需持有执行引用，但其关联只在同一提交点转移，不是另一个可写的 Running 真值。

运行/未收尾计数暂从正确性必需字段降为待证明的派生索引。第一版优先在域结构同步保护下无分配遍历权威状态来确认 Debark/Oust；运行位置检查包含 Running、Debarking 以及 Doomed 内尚存的 Running 位置；只有性能证据要求时才加计数，并规定唯一更新入口、溢出检查及与状态重算的检查。无锁镜像、冗余完成位和计数都不能仅为读取方便加入。

删除 tag 后必须重新检查原来无锁读 tag 的每个调用点、Task::exclusive 的可变访问与状态锁序；不能只把 AtomicU8 删掉然后无锁读带引用的 enum。这里是设计选择，尚未完成调度器实现验证。

### gate 只负责能力同步，不另存生命周期

原 `gate: SpinLock<bool>` 中的 bool 表示退出清理已开始。新设计由进入 TaskState::Doomed 唯一表达这个边界，Reaped 继续保持关闭；不再保存第二枚 closed/accepting。Held、Blocked、Parked、Debarking、Debarked 都不等于退出，仍可按原授权接收能力。

gate 中的 pies、heirs 是能力与转授关系，version 是能力关系复核所需的派生版本；它们不是互斥执行状态，不搬入 TaskState 的某个分支。gate 若保留，表示这一组数据的同步容器，而不是另一个状态门。

必须闭合“检查状态 → 发布能力”的竞态：不能在 gate 锁下读一次非 Doomed，释放状态同步后继续插入。候选提交锁序为按 TaskId 排序的 gate 锁 → Unit 域结构提交锁 → scheduler owning lock；能力提交在同一个域结构提交范围检查涉及任务的权威状态并修改关系。进入 Doomed 可以仅持域结构提交锁完成，随后释放它，再获取 gate 做退出清理；不得在持有域结构或调度锁时反向等待 gate。这样能力提交与退出状态转移只能一方先完成。

资源准备和可失败分配在上述提交范围外完成；锁序需要全量 lockdep 验证。此处保留 gate 锁域的必要性，不等于现有 SpinLock<bool> 可以原样保留，也不表示把全部能力工作都放进 TaskState 的锁。

## 6. 调用表：完整职责先于数量

保留 Build/Spawn/Embark/Debark/Slay/Join/Oust/SelfId/Sire/HeirCount/Heir/Fall，不恢复已被 Embark 替代的 Hatch。逻辑类型只表达载荷，不建立另一套动作词汇；Team 目标明确包含子树，不另造 Tree 目标名：

```rust
enum UnitTarget { Task(TaskId), Team(TeamId) }
enum OustTarget { Task(TaskId), Team(TeamId) }
struct TaskExit { cause: ExitCause, reason: Reason }
enum JoinReply { Pending, Reaped(TaskExit) }
// Collect 返回 TaskId；0 表示期限内没有成员完成
// 非零 TaskId 对应已移交给调用者 results 的 Reaped 状态引用
```

UnitTarget 的 Team 始终表示根 Team 及它的后代域，绝不表示“只操作该 Team 的直接成员”。OustTarget 的 Task 表示放下该 task 的**结果拥有项**，不销毁活 task。

| 调用 | 契约 |
| --- | --- |
| Build(kind) → TeamId | 能力允许时创建 Constructing 空域并挂到调用者 heir |
| Spawn(team, entry, args, count, stack) → TaskId | 当前 Team 或直接 owned child；首次提交构造；返回 Held，并建立调用者结果拥有项 |
| Embark(target) → () | Task 首次放行或撤本地停止状态；Team 仅撤根的 Debarking/Debarked 包装 |
| Debark(target) → () | Task 设本地停止状态；Team 转入 Debarking/Debarked；成功确认范围内无 Running/Debarking/Doomed(Running) 占槽 |
| Slay(target) → () | Task 终止及其 heir 级联；Team 关闭创建并请求整棵子树终止；成功只代表请求可靠接受 |
| Join(task, wait) → JoinReply | 非消费地等单任务收尾完成，返回真实 TaskExit |
| Oust(target) → () | Task 放弃自身结果拥有项，可在完成前放弃；Team 放下已收尾的 owned 子域 |
| Collect(team, wait) → TaskId | owned 根下一条完成任务的结果拥有项移交给调用者 results；0 为期限内没有完成成员；随后 Join 读取退出结果 |
| SelfId → TaskId | 当前执行主体，无上下文为 0 |
| Sire → TaskId | 维持 Team 创建者溯源语义 |
| HeirCount → usize | 从调用者 heir 容器派生当前数量，只供统计；不存另一枚 count，也不是枚举前置 |
| Heir(after: TeamId) → TeamId | 返回调用者直接 heir 中 ID 大于 after 的最小项；after=0 为开始，返回 0 为当前无更多 |
| Fall(wait) → bool | 保留仅 Accord 到达的通知；不是 Unit 任意状态变化 |

新表保留 master 的 12 个调用名称，包括 HeirCount 与 Heir；Heir 的参数明确改为稳定 ID 游标；只提出一个职责完整的 Collect，因此共 13 个入口。不能为凑回 12 个而删除既有词汇；本轮的 Heir 游标变更是明确评审的契约优化，不是隐式改写。Collect 已是 Pie 域使用的收集词汇，在 Unit 中表示完成事实收集；它仍是待评审的新 Unit 入口，不能因词汇已有就视作已获准实现。不得因缺失结果交付而再次添加临时 Status/Observe。Build/Spawn 仍分开，映射与授予能力发生在中间。


### 调用归属

这组调用属于 Unit 的生命周期与血缘轴，不另建通用查询域。

| 职责 | 调用 | 读取或改变的事实 |
| --- | --- | --- |
| 血缘查询 | Sire、HeirCount、Heir | 创建者与调用者直接拥有的子 Team；查询不移交所有权 |
| 创建与执行控制 | Build、Spawn、Embark、Debark、Slay | 域/任务创建与各自状态转移 |
| 收尾等待与读取 | Join | 指定 TaskState::Reaped 及退出结果；读取不消费 |
| 收尾结果收集 | Collect | 已完成任务的结果引用从根队列移交到调用者 results |
| 所有权放下 | Oust | 放下结果拥有项或已收尾子 Team，不等物理回收 |
| 能力到达通知 | Fall | 仅通知 Accord 到达，不作为成员完成通知 |

Collect 是收尾生命周期操作，不能用于遍历活成员、返回任意调度状态或授予控制后代任务的权限；Heir 是直接 owned Team 的血缘枚举，不能拿它替代子树结果流。底层都可以引用同一 TaskState/TeamState，但入口职责不混合。

## 7. 授权矩阵

| 操作 | 授权 |
| --- | --- |
| Build | 保留 Build FETCH 能力及 ProgramKind 不提权检查 |
| Spawn | 当前 Team，或调用者 heir 中的直接 Team；调用者与目标祖先都未关闭创建 |
| Embark/Debark(Task)、Slay(Task) | 保留同 Team 或目标属于调用者直接 child Team 的权限 |
| Embark/Debark(Team) | 仅调用者直接 owned child；不允许暂停包含调用者自身的根 |
| Slay(Team) | 直接 owned child **且** Doom FETCH 能力；明确比 Task 目标更严格 |
| Join | 同 Team/直接 child Team 的尚有效记录，或调用者 results 已拥有的记录；Collect 授予的仅是结果读取/放弃权，不授予任务控制权 |
| Oust(Task) | 仅调用者 results 的拥有项；可由 Spawn 或 Collect 建立，不因同 Team 权限而允许替别人放下结果 |
| Oust(Team)、Collect | 仅调用者 heir 中对应项 |
| HeirCount/Heir | 只枚举调用者自己的 heir 表 |

Join 的授权可以通过小型记录中的 Team 身份完成，不靠活 Task 的 muster。muster 与 ID 索引都不授予权限。Join(SelfId, 非 POLL) 拒绝；POLL 可返回 Pending。越过控制根的 sibling/ancestor 不因节点链可见而获得操作权。

Room::Doom 原先可凭 Doom 能力终止任意目标 task 所属 Team。迁移期保留其原权限并调用共同的域终止机制；不调用限制更窄的 Unit::Slay(Team) 包装造成静默收权。后续是否将这项全局能力接口归入 Unit 是独立迁移决定。本稿新增的 Team 目标不会绕过 Doom。当前 Room 接口用活 TaskId 定位域的缺陷，不作为新 Unit 接口的定位方式。

## 8. 停止与恢复：指定成功边界

Task 的 Embark：Held 或 Debarked(Held) 原子取出保活引用并首次放行；Debarked(Starved) 按域门进入 Starved/Parked；Debarked(Blocked) 恢复原等待，不伪造唤醒。Debarking 仍占运行槽时返回 Busy，离槽后才允许恢复；其他已放行且未本地停止状态幂等成功，终止/已完成拒绝。Team 的 Embark 撤本根 Debarking/Debarked 包装，恢复其 Constructing/Ready 载荷；可以显式取消尚在确认中的域暂停请求。不放行 Held，不改变成员 Debarking/Debarked，也不恢复后代自身的 Debarking/Debarked。

Debark(Task) 将 Running 转为 Debarking 并推动离槽；其他可停止状态直接转为 Debarked。Debark(Team) 将根状态包入 Debarking 并推动成员离槽；确认整个子树无占运行槽成员后转为 Debarked。确认无占运行槽的 Running/Debarking/Doomed(Running) 后成功，尚未离槽返回 Busy。Busy 明示停止请求已经保存在对象的 Debarking 中，调用者可重试确认，不在错误分支偷偷回滚。重复 Debark 幂等。自暂停 Task 在切换前准备成功返回帧，恢复后从原调用继续；Team 不允许自包含暂停。

Team 恢复不承诺全部任务已经运行，只承诺根的暂停包装已撤销、已有 Parked 成员都纳入可继续调度的协议。创建、唤醒与恢复竞态必须使成员恰好处于 Held、等待、Parked、队列或运行槽中的一个位置。

Slay 不等待 hooks。设置 Doomed 与可靠终止请求是一次提交；接受后重复请求幂等，不恢复创建。预留的收尾工作节点保证接受之后不会因 OoM 丢请求。暂停门不能阻止终止收尾或延迟埋葬。

Slay(SelfId) 不返回用户代码，进入退出路径；表中的 () 只适用于调用者仍存活的目标操作。正常主动退出仍走 Room::Reap，不把 Slay 自身的强制终止原因改写为主动退出。

## 9. 结果所有权与可靠子树完成事实

选择 **Join 非消费 + Oust(Task) 显式放弃 + Collect 将根视图移交为调用者结果拥有项**，不把 Join 变成单 owner 的领取操作。

Spawn 发布前预留独立的 TaskState 存储，以及每个仍被活 owner 持有的祖先控制根的一个完成队列链接。Spawn 调用者拥有一个结果项；目标根及其祖先各有独立视图。链接引用同一 TaskState 存储，不复制退出结果；Reaped 分支绝不强持有 Task/Team/Space。若任一必要预留失败，Spawn 全部失败且不发布任务、不提交首次构造。

每个 root 视图由创建该 root 的 Task 独占收集，所有 task 完成都交付原始 cause/reason，包括 0、非零、强制终止和级联；没有“已观察所以不算故障”开关。 root 中间被 Oust，也不能删除祖先已预留的视图。这使主任务活着时的辅助任务异常仍可交付。

退出只转移预留的状态存储，不临时分配结果对象。真实 cause 来自退出路径，不从用户 reason 推断；主动 Reap 使用 reason 的原值，不能借保留码伪造 fault/cascade。hooks 完成后，在同步保护下将 TaskState 转为 Reaped { cause, reason }，并将预留链接加入各根完成队列、唤醒 Join/Collect。一个 task 只完成一次；quit→reap 的两个调用点不能覆盖 cause 或重复交付。

生命周期规则：

- Join 可重复、可多等待者；读取不消费。收尾之前 Pending，之后 Reaped(exit)。
- Oust(Task) 删除 owner 的结果项；活任务继续执行，退出后的 owner 查询记录不再有效。放弃前已固定记录的 Join 可继续完成；放弃后发起的新 Join 不重建该项。
- 即使 Spawn 结果 owner 已放弃，祖先 Collect 的预留视图仍交付。Collect 在同一提交点从本根队列移除视图，并将同一状态引用挂入调用者 results；其他根和 Spawn owner 的结果不变。它不是读取 TaskId 后丢掉最后引用。
- 同 Team/直接 child 的非 owner Join 只能读取尚未放弃的结果项；队列视图本身不能偷偷复活已放弃的 Join 句柄。显式 Collect 可以建立新的结果拥有项，这是一次有授权的所有权移交，不是借 ID 猜测重新查询。
- owner 退出清理其结果项和它独占的 root 队列；对子域先关闭创建和触发级联。迟到完成不重新挂入已放弃的 root 队列，但其他祖先仍收到事件。
- Oust(Team) 成功后关闭本根查询和队列，放弃调用者在该子树内的结果项；其他已固定的读取延后释放元数据，不保留资源域。
- 重复 Oust 为 Denied；ID 不复用；没有对象/无权不能推断 reason=0。

**预算**：限制域深度、每 task 的 owned 结果项、每 root 的“已预留但未消费”视图数及全局保留状态记录数；额度满使 Build/Spawn 返回 OoM，退出不可失败，也不丢队首覆盖。额度值集中在 Unit 配置中，实施前用现有 churn/场景容量确定，不能用无限 Vec。Collect 只将队列额度转为结果拥有项额度；Oust 才释放最终持有额度；长期 Team 可持续创建与回收。元数据开销为 O(任务数 × 控制树深度)，实现检查需量化其可接受性，不能宣称少字段就少内存。

boot 创建且没有用户 owner 的对象，由显式内核启动 owner 管理结果项；健康用例结束或启动职责移交时放下。不能为这些对象增加永久保留的全局结果例外。结果索引即使因队列引用仍存在，也必须记录 owner 项已被放弃，阻止新的 Join 借未收集的队列引用重新获得已失效的结果。Collect 明确建立的 results 项是新的有效读取凭证。

Collect 每次只返回一个 TaskId，不再返回五字 CompletionEvent。非零返回时对应记录已为 Reaped，且状态引用已挂入调用者 results；单消费者的并发请求各取不同视图。调用者随后 Join(POLL) 读取 cause/reason，再 Oust(Task) 放下。与同一 owner 的并发 Oust 仍需调用方协调；Task 物理回收本身不能让该 Join 失效。

移交节点与结果容器插入所需资源在 Spawn 发布前预留，或者复用队列节点；Collect 的成功提交不分配，不允许“队列已弹出但 results 插入 OOM”。若调用者已拥有同 TaskId 的结果项，保留其原项并释放本根的重复引用，不能用不同结果覆盖它。节点预留预算同时覆盖队列持有和移交后的 results 持有；队列排空不等于结果预算已归还。

代价是获取退出原因需要 Collect + Join 两次调用，好处是结果读取只有 Join 一个契约、复用现有单字与三字返回支持，并消除收集后状态寿命不明确的问题。Collect 只改变结果引用归属，不改变 Reaped 事实或故障政策。

没有完成成员时按 Wait 等待，0 仅表示当前或期限内无下一项，不证明整个子树结束。完整结束仍用 Oust(Team) 的收尾判据，不能将任意成员完成伪装成整个 Team 已完成。

### HeirCount 与 Heir 的优化边界

保留原词汇与 HeirCount 数量查询，但数量从 heir 容器长度派生，不维护第二份可写计数。枚举直接从 after=0 开始反复调用 Heir，不先调用 HeirCount 分配或限定循环次数。

TeamId 不复用且创建时递增，使用“严格大于 after 的最小 ID”可避免 Oust 导致索引左移而漏过现存下一项。每次调用仅观察当前集合，不承诺冻结快照：已经移除的项不报告，结束后新建的项需要新一轮枚举。若未来要求固定快照，另行定义寿命与预算，不能暗加全表版本或 cursor 对象。

本轮保留单项寄存器返回，暂不引入用户缓冲与批量写回；确有枚举吞吐需求后再评估批量 Heir。优化接口不要求立刻换数据结构，最小较大 ID 可以先由当前容器扫描得到。

## 10. 等待与放下的线性化

Join/Collect 的新契约：POLL 立即探测；AtMost 以一次固定 deadline 等待；唤醒后重新检查事实/队列，剩余期限继续等待；超时与完成同时发生时在同一同步点决定返回。Forever 不因一次无关唤醒就返回未完成；Join 的未完成编码为 Pending，Collect 的无结果编码为 TaskId(0)。登记等待与检查必须闭合，避免“查到未完成 → 完成发信 → 才登记”漏唤醒。

Oust(Team) 在拓扑提交点同时验证 owner、无未收尾后代、无进行中的构造发布，随后标 Ousted 并摘 child；Held 计入未收尾。Constructing 的 staging 撤销采用预先固定资源再提交的协议，不能拿拓扑锁嵌套 Space/能力锁。撤销准备失败则不摘所有权；提交后资源释放在锁外完成。

Join 成功和 Oust 成功仍不保证栈、帧或 Space 已归还。Oust 不等共享 TaskState 被所有临时读者释放，也不等系统诊断完成。退出 hooks 是否会异步级联必须明确：本稿的 Oust 确认整棵子树收尾事实，父 Task Reaped 不冒充后代全部 Reaped。判据优先检查每个 TaskState 是否 Reaped；计数只是可选派生索引。

## 11. 同步方案及方法对偶

选择一把专门的 Unit 域结构提交锁作为第一版正确性模型，保护唯一 TeamState/TaskState、TeamLife 拓扑、成员结构、创建提交、祖先运行/未收尾计数与队列视图归属。它不保护能力转授、Space 映射、用户缓冲拷贝或诊断，不恢复旧的全局能力 GRAPH 锁。

调度 admission/Running 离槽也必须参加该协议：门检查和权威调度位置变化在同一个提交范围。Debark 进入停止状态后，新的 admission 被阻止；同步保护下确认没有 Running/Debarking 或 Doomed(Running) 占用运行槽才成功。祖先计数如经性能验证保留，仅作派生索引，不成为第二份可独立写的状态。不能用 snapshot + revision 重查替代该保证。

目标锁图为按 TaskId 排序的 gate 锁 → Unit 域结构提交锁 → scheduler owning lock；不需要 gate 的状态转移可从域结构锁开始。调度路径不能反向获取 gate；Space 与资源准备在提交锁外，能力关系的最终提交按上述锁序复核状态；Drop 与析构放在提交锁外。具体 lockdep Level、新旧调用点调整和锁内禁止分配清单是实施前置检查；目前尚未证明现有调度器可直接满足此顺序。拓扑锁可后续按根分片，但不能先用分片跳过跨根创建/计数的原子性。

| 职责 | 方法组 |
| --- | --- |
| 资源域创建/放下 | Unit `build` / `oust(Team)`；Task `adopt` / `oust`，沿用既有对子 |
| 成员关系 | 沿用 `tasks`、`prune_tasks`；发布/移除纳入共同协议，不为包装重新造词；已有 TaskBuilder/TeamBuilder 的 `spawn` 保留，以所属类型区分对象 |
| 构造预留 | 沿用 `staged` / `cancel_staging`；首次提交沿用 `publish`，不新增 stage/unstage 词组 |
| 本地运行门 | 使用 `embark` / `debark` 对偶；`paused_local` / `paused_effective` 沿用 paused 词根并区分范围，不另用 pause/resume |
| 外部执行控制 | `embark` / `debark`，Task/Team 共用词汇、分别定义首次放行 |
| 退出事实 | 沿用状态转移 `transform` 与查询 `state`；不新增 complete/completion 方法或完成位 |
| 结果持有 | 对应 `spawn` / `oust(Task)`；内部容器沿用插入/移除操作，不强加 retain_result/release_result 方法词组 |
| 收尾队列 | 沿用 `push` / `pop`；外部 Collect 消费，不新增 enqueue_completion/dequeue_completion 词组 |
| 快照 | 沿用 `tasks_snapshot` / `heirs`；统一返回 Result，撤回 snapshot_tasks/snapshot_children 改名，不留 checked/静默空两套语义 |

Slay 是不可逆终止，不凑“反向复活”；Held 首次放行也不提供反向重新 Held。查询与修改必须作用同一对象和范围，不能 paused 查祖先而 set_paused 只改本地却不在名中区分。

## 12. ABI 固定与迁移

由于 Embark/Debark/Slay/Oust 参数和 Join 返回都变化，本稿不在原编号上悄悄重解释。保留 Unit 的 class=1：仓库明确以 class 表示职责域，不能把它改成版本号，也不能占用明确留空的 class=3。新契约使用 class=1 的新 slot 段，原有 0..15 整体退役，16..31 暂不分配。这样旧镜像调用被拒绝，不会将原 TaskId 恰好当作 target 标签。部署必须重建 kernel/env/execution/loader/System/Shell/测具。

新代际显式 slots 建议：Build=32、Spawn=33、Embark=34、Debark=35、Slay=36、Join=37、Oust=38、Collect=39、SelfId=40、Sire=41、HeirCount=42、Heir=43、Fall=44。使用已有 mold 的 #[slot(N)]，顺序不再生成编号。PR class=1 slots 12..15 不保留兼容别名。即使返回未变化的 SelfId/Sire/Fall 也迁移，避免旧镜像部分可用、部分静默失败。

建议线格式：target 占 tag+id 两字，Task=0、Team=1；未知 tag、零非法 ID、保留值都拒绝。Spawn 暂保留 team=0 表当前域的既有规则；Heir 的 after=0 是枚举起点；两者不扩散到其他目标。

Join 返回 a0=0 Pending，a1/a2=0；已完成 a0=1 Reap、2 Slay、3 Cascade、4 Fault，a1=reason、a2=0。Collect 成功返回 a0=TaskId，0 表示等待期限内没有完成成员；错误仍使用负码。Heir 返回 a0=TeamId，无更多为 0；HeirCount 返回容器派生数量。以上复用现有 ret/ret3，不再要求新增五字返回支持。所有 ID 必须限制在非负返回编码范围内，耗尽时拒绝创建。

保留 UnitFail 的负码 Denied/Busy/OoM/BadEntry；BadImage 仅用户态 loader 使用，内核 Build 不解析镜像。结果中的非零 reason 不转成 UnitFail。Busy 的持久副作用仅有文档明确的暂停门设置；构造预留、Oust 失败不产生半个公开对象。

实施顺序：

1. 固定本稿对象模型、权限、预算与 ABI 代际；检查新 slot 段和返回寄存器实现边界。
2. 建立独立 TaskState 存储与 life 元数据、创建预留与无分配收尾，先证明造→收→放闭环。
3. 迁移成员提交及调度 admission 协议，再实现 Task/Team 控制；lockdep 检查全部锁边。
4. 新 ABI 编解码与显式编号；旧 slot 拒绝；所有包装与镜像同步迁移。
5. Control 保存自己的主 task，Join 主结果并 Collect 辅助 TaskId 后 Join 读取结果；Job 决定失败联动；退出后 Oust task 结果和 Team。
6. Rust execution::Join<T> 保留共享内存返回 T 的协议；增加内核结果拥有项的 Drop/领取释放，异常退出不能永久等 DONE。不能把内核 Reason 当作 T。
7. 删除 PR 的 Status/Observe 与 observed 诊断过滤，重新接好测具预期结果和服务故障判定。

## 13. 必须验证的场景

| 类别 | 成功与失败边界 |
| --- | --- |
| 构造 | 每阶段 OOM、首次 Spawn 与 Oust/owner 退出竞争、映射失败、Held 不抢跑 |
| 权限 | 同域、直接 child、兄弟、祖先、猜 ID；Team Slay 缺 Doom；同域不得放别人的结果 |
| 暂停 | Running 跨 hart 离槽、暂停中新 Spawn/Build/唤醒、嵌套门、Blocked 恢复、自暂停、终止绕过暂停 |
| 引用 | 父 Task 先埋、父 Team 先释放、祖先元数据仍有效；Held 保活环必定拆除，其余不新增永久强环 |
| 结果 | 0/17、主动 reason 等于保留码、fault/Slay/cascade、重复 Join、放弃前后 Join、退出 hooks 之前不可见 |
| 交付 | 主 task 活着辅助 task fault、Collect→Join→Oust、Task 先物理回收、移交无分配、已有 results 去重、多根独立视图、owner 死亡与迟到完成 |
| 预算 | 深度/结果/视图额度满时原子拒绝创建；退出和清理不分配；长寿命线程 churn 持续释放记录 |
| 等待 | 完成与登记/超时竞态、多个等待者、无关唤醒、同一 deadline、不将 Denied 当退出 0 |
| ABI | 旧 slot 拒绝、slots 黄金值、标签与单字/三字返回边界、Heir 游标、所有程序同代际 |
| 策略 | 预期命令退出 17 与服务 fault 同时发生，服务仍导致场景失败；主退出不掩盖辅助 fault |

以上保留的是被重新打开评审的上一轮方案。结果所有权、Collect/Oust(Task) 必要性与 ABI 编号均未定稿；以文首“当前评审判断”为准。未修改或验证 Rust/QEMU 实现。
