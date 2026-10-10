> 实施状态（2026-10-10）：已由主智能体接手并完成迁移与验证，阶段裁定和验证见 [迁移记录](unit-migration-report.md)。以下为最初分阶段计划，历史工具链缺失和“尚未编码”描述不再表示当前进度。

# Unit 域重设计实施计划

本文以 [unit-domain-redesign.md](unit-domain-redesign.md) 为唯一设计依据；PR #5 只用于核对现状，不构成新契约。PR 审查树 `/workspace/pr5-review` 的 HEAD 仍为 base `4d664c99`，其 PR 内容是未提交 diff。实施前应从真实 PR head 创建隔离分支/工作树，并把本计划及设计文档带入；不得在审查树上直接提交。主仓当前为 detached base。

## 实施前阻塞：阶段 1 必须定稿

以下项目会改变所有权、回收或并发语义，当前文档尚不足以直接编码。形成逐项裁定后才进入状态/生命周期实现；不得用新公共调用、新句柄或额外状态字段代替裁定。

1. **共享成员/TaskState 存储及同步边界。** 指定同一成员记录和唯一 `TaskState` 的实际所有者、锁/引用模型、所有读写入口及线性化点。当前 `Task.state` 是独占访问字段，`tag: AtomicU8` 供无锁判断；删除 tag 后，调度器、wait、gate、control 如何在不读撕裂 enum、不持锁跨调度的条件下共享状态必须明确。工作队列的 `next` 不能作为持久 `Reaped` 载荷；定义脱链/转换顺序与回收链接暂存位置。
2. **同 owner 多 root 的资格与 Oust。** 待领取集合按 owner 去重，但单有 owner ID 无法判断 Oust 一个根后该 owner 是否仍由另一个 owned 根覆盖。明确资格从稳定拓扑即时派生，或在成员记录中保存可精确撤销的根来源；覆盖嵌套/重叠根、重复领取任一匹配根、Oust 根提交与并发领取。不得把 owner 多份复制回结果事实。
3. **Team 物理回收后的祖先交付。** 后代 Team 的成员元数据必须在其 owner 先 Oust、Team/Space 执行资源回收后仍可让祖先 owner 领取；同时不能让退出记录强持有 `Task`、`Team` 或 `Space`。定稿稳定子树元数据的保留/引用归属、何时删节点和空间分离办法，覆盖多层后代先 Oust、祖先晚领取。
4. **Join 的观察/领取定位与授权。** 文档规定观察由当前 Team 或直接 heir 成员表定位，领取又可从 owned 根稳定子树定位；明确 Task 目标、Team 目标各自的定位算法、锁内复核、目标成员已 prune/Team 已 Oust 时的结果。确认 Task `Join(领取)` 的“相应 owned 根授权仍有效”如何确定；确认祖先 Task 的 Join 是否可接收其后代结果、仅可观察哪些后代，以及与“观察不产生领取权”一致的精确权限边界。ID 不能充当权限。
5. **等待票据、登记闭合与锁序。** 固定 Join 的状态快照、检查—登记—再检查协议，唤醒键及票据寿命，Reaped 发布与等待者唤醒顺序；POLL/AtMost 固定 deadline/Forever 对无关唤醒的行为。把设计稿候选顺序 `gate(TaskId 排序) → Unit 结构提交锁 → scheduler owning lock` 与 wait/messenger 现锁层级逐处对照，明确哪些锁绝不嵌套、如何处理状态复核后被改写，以及禁止在锁内分配、Space 操作、用户拷贝、析构的可执行边界。
6. **Rust `Join<T>` 异常收尾。** 内核 Join 只交付 `TaskExit`；用户态 `DONE/LEFT` 只仲裁 T 槽。定稿闭包 panic/fault/reap 未写 DONE、父方同时 Drop/Join、`try_spawn` 的 Spawn 成功而 Embark 失败、唤醒键晚到时 Completion Box 和 T 的唯一释放方。Drop 不增加 native 结果义务、不调用 Oust(Task)；内核 owner 持续领取也不能被误认为已释放用户态 T 槽。
7. **ABI 编码/迁移表。** 明确 Join 目标区分、领取位、Wait 编码、返回 `TaskExit{task,cause,reason}` 的寄存器布局和 `ret3`、Scan 游标/容量/用户缓冲区、错误映射、最大 ID 和所有新增 slot。Unit class 固定 1；旧 slots 的拒绝规则必须确定；内核、`crates/env` 生成编码、运行时 wrapper、程序镜像同一提交迁移，不发布中间 ABI。沿用既定 Denied/Busy/OoM/BadEntry；用户 loader 的 BadImage 与 `reason` 非零不可混作调用错误。
8. **有界容量与根管理者。** 规定待领取 owner 集、成员/拓扑元数据与域深度的限额、预留失败回滚、满额时 Spawn 拒绝语义及可观测错误；确认无用户 owner 顶层 Team 的“启动管理职责”实际落点，保证持续领取而无永久保留项。

## 阶段与依赖

### 1. 代码基线及设计闭合

**依赖：** 无。先由真实 PR head 建隔离实现分支；PR 审查树只作只读对照。核验 base→PR diff，并记录真实 head SHA。按上列阻塞项形成决策记录，必要时先修订唯一设计文档再开始代码迁移。

**基线位置：** `kernel/src/work/unit/{task,team,life,weak}.rs`、`kernel/src/work/room/{scheduler,messenger}`、`kernel/src/work/unit/gate/`；`crates/env/src/abi/call/unit.rs` 与 `crates/execution/src/unit.rs`、`crates/execution/src/unit/task.rs`；`programs/src/system/control/{unit,instance,lifecycle}`。重点标出现有 `Task.state + AtomicU8 tag` 双写、`Team.tasks/held/heir`、`Team.completion/observed`、退出 prune 与 Husks 链、旧 `Join` / Heir ABI、Control 的 Status/Observe 聚合。

**验收：** 决策表逐项解决上列 8 个阻塞；基线文件/入口/调用方清单能追踪 Spawn→退出→领取→清理及每类 Join 权限；真实实现基线 SHA 可复现。未定稿项不得带着假设写入后续阶段。

### 2. 生命周期、引用与唯一状态存储

**依赖：** 阶段 1 完成。先建立成员元数据、共享状态、稳定拓扑元数据和 held 保活索引的具体类型/锁边界，再迁移所有读写者；本阶段仅在实现分支内部过渡，不发布新 ABI。

**职责文件：** `kernel/src/work/unit/task.rs` 建 `TaskState` 的唯一权威态与合法转换（含设计载荷 Held/Starved/Running/Blocked/Parked/Debarking/Debarked/Doomed/Reaped）；`team.rs` 建唯一 `TeamState`、共享 `tasks` 成员存储、held 索引、拓扑节点；`life.rs`、`weak.rs` 分离等待寿命、活动弱引用与稳定元数据身份；`scheduler/core/*`、`messenger/wait/*`、`messenger/{reap,doom}.rs`、`gate/*` 全面改走同步状态，不留 `AtomicU8 tag` 的镜像读。退出工作链接置于单独回收容器，`Reaped{cause,reason}` 不携等待票/队列链接。拆除 held 环必须有确定提交点。

**验收：** 任意状态只有一个读写权威；没有无锁 enum 读取或状态/tag 不一致窗口；Running hart 仅在真正离槽后清除；Reaped 发布时 hook 已完成，载荷无执行资源/等待票；弱执行引用失效不等价 Reaped；记录和拓扑元数据不强持有 Task/Team/Space；Held 首次放行、终止及失败回滚不留环。过渡提交内部编译可通过但 ABI 不对外启用。

### 3. 成员、owner 接收与创建退出闭环

**依赖：** 阶段 2 的状态与容器边界稳定。先做发布预留，再接通收尾和领取。

**职责文件：** `kernel/src/work/unit/team.rs` 的 Spawn/Build 首次提交与成员发布，`team.rs` 共享成员记录的 owner 集和容量、`task.rs` 的任务状态发布，`kernel/src/work/room/messenger/reap.rs` 的首个终止原因、hooks 后 Reaped 发布及唤醒，`doom.rs` 的活 owner 清理，`scheduler/core/*` 的执行引用撤除与 prune。把当前 `Team::prune_tasks`（退出即移除弱成员）改为按 Reaped/owner 待领取规则 prune。Join 核心放内核 Unit 服务层/独立模块，由后续 envcall 转发；不得让返回记录强持有执行对象。实现观察不消费、领取只删当前 owner 项、领取成功构造完整 TaskExit 后线性化，最后 owner 领取才 prune Reaped 成员。

**验收：** Spawn 先预留所有节点/收尾链接再原子发布，失败无半任务；退出不分配、cause/reason 原样写唯一 Reaped；收尾完成后，Task 执行资源可先于领取回收；状态及拓扑元数据不强持有 Space，Space 按其实际资源引用释放；并发相同 owner 只一领取成功，不同 owner 分别成功；多 root 同 owner 去重、Oust 清理资格符合阶段 1 裁定；owner 死亡清项但不丢活成员收尾或其他 owner 结果；达到额度拒绝 Spawn 而不覆盖结果。

### 4. 调度门、Team 状态与能力同步

**依赖：** 阶段 2 的共享状态和阶段 3 的成员提交协议。状态转换先于控制 ABI 包装。

**职责文件：** `task.rs`/`team.rs`、`kernel/src/work/room/scheduler/core/*`、`kernel/src/work/room/messenger/doom.rs`、`kernel/src/work/unit/gate/*`、`kernel/src/runtime/switcher/envcall/unit.rs` 内部操作入口。实现 Team Constructing→Ready 的首次 Spawn 提交；暂停/停止包装状态、Held 不被恢复放行；Doomed 保留运行位置；Build/Spawn admission 与首次构造/Oust 竞争同一提交协议。能力接收 gate 与 Doomed/Reaped 派生状态按锁序同步；Oust 仅 Team，需直接所有权，完整子树已收尾且无创建提交才一次性标 Ousted/摘 heir；Oust 不新增 Doom 要求，Doom FETCH 检查属于 Slay(Team)。Scan 基于直接 heir 的稳定 ID 分页替代 Heir/HeirCount，不再走会丢待领取记录的旧 prune 路径。

**验收：** gate、状态、拓扑、admission 无互相倒置锁；队列中无不可运行 Task，Doomed(Running) 不提前离槽；Debark 成功条件及 Busy 可重试语义可验证；构造/Spawn/Oust 竞争只有一个提交顺序；Oust 允许尚未领取但已 Reaped 的成员，不允许 Held/未收尾成员，且只清当前根资格；Scan 排序严格大于 after、容量和用户写回失败不改 heir。

### 5. ABI 与包装原子迁移

**依赖：** 阶段 1 ABI 决策已定，阶段 3/4 内核行为可用。本阶段与阶段 6 留在同一隔离实现分支，完成 ABI 和调用点编译迁移；阶段 6 策略与验证完成后，才形成可发布的完整迁移批次。

**职责文件：** `crates/env/src/abi/call/unit.rs`（11 逻辑入口和编码/解码）、`crates/env/src/{wire/handle.rs,abi/wait.rs}`（目标/标量类型，如需）、`kernel/src/runtime/switcher/envcall/unit.rs`（权限、等待与用户拷贝）、`crates/execution/src/unit/*`（Safe wrapper / TaskExit）、生成调用表相关 `crates/env/src/abi/call/*`；所有 `programs/src/**`、`programs/tests/**` 的调用点和镜像。删除 HeirCount/Heir、Status/Observe 独立入口；Join 承载 Task/Team 目标、Wait 和领取位；Scan 替换继承枚举。保留原词汇及字段分组，不私自沿用旧 slot 编码。

**验收：** class=1；11 个入口数量与设计表逐一一致；新 slots、ret3、拒旧编码、最大 ID、失败码测试覆盖；所有内核/环境/runtime/程序镜像同批可编译（包含下一阶段行为迁移所需的签名适配），搜不到旧 ABI 调用；新旧 wire 不会被静默误解。Scan 的锁内快照、固定内核页/数组和锁外 LE 写回符合定稿。

### 6. Control、Job、运行时策略与验证

**依赖：** 阶段 5 ABI 原子迁移。先迁系统 owner，再处理通用 Rust Join<T> 清理，最后跑分层验证。

**职责文件：** `programs/src/system/control/unit/{observe,wait,reap,task,mod}.rs`、`programs/src/system/control/instance/{mod,hook,schedule}.rs`、`programs/src/system/control/lifecycle/{ruin,dispatch,queue}.rs`：Control 保存主 TaskId、用 Team Join 持续领取本 owner 子树结果并决定主/辅助策略，删除 Status/Observe 聚合，适配阶段 2–4 已迁移的成员清理、调度撤除和 muster 语义；不在 Control 侧另行实现内核 prune。`crates/execution/src/unit/task.rs`：DONE/LEFT 只清共享 T 槽，fault/未写 DONE 与 Spawn/Embark 失败严格按阶段 1 所定仲裁规则释放一次。更新 `kernel/tests/gate/{lib,tests}.rs`、`kernel/tests/embedded.rs`、`kernel/src/health/{task,syscall}.rs`、适用的 `programs/tests/`，增加 owner/多 root/嵌套/Oust/Scan/ABI/运行时异常用例。

**关键验证及验收：**

- 内核状态与提交：Held 环拆除；Debarked(Blocked) 唤醒后仍不运行；Team 暂停不关闭能力接收；Doomed(Running) 占槽直到离槽；首次 Build/Spawn 与 Oust 原子竞争；gate 收能与 Slay/退出交错。
- 接收及生命周期：物理回收前后领取同一 TaskExit；观察可重复且不消费；同 owner 并发领取唯一成功；多 owner 各领一次；后代先被 Oust、祖先仍可领；同 owner 多根 Oust 一根不误删另一根资格；owner 死亡时活成员仍可靠收尾；额度满/回收后可继续创建。
- Join 等待：POLL 即时；AtMost 单 deadline；Forever 对无关唤醒重查；登记闭合不漏唤醒；领取与最后 prune 竞争；等待票过期/目标完成先于登记；权限/输入验证失败不消费；Join 用寄存器返回，Scan 用户写回失败不改 heir。
- ABI 与上层：旧编码明确拒绝；Scan 分页边界和无效写缓冲；Control 持续领结果；DONE/LEFT 两种时序、fault 前未 DONE、父方领取或 Drop 与子方完成/异常的竞态、Embark 失败及晚唤醒下 T/Box 恰好释放。
- 构建/测试按仓库口径：当前环境 PATH 无 `cargo`、`rustc`、`qemu-system-riscv64`，故此计划阶段不能声明通过。恢复工具链后先做受影响 crate/目标的编译和宿主测试、kernel 健康面，再用 `nu scripts/qtest.nu --package kernel --scene <scene>` 跑每个相关整机场景，整机场景用 release；不带 `--scene` 的默认 qtest 有一个预期失败的 scene 哨，不以默认全绿验收。lockdep 依仓库 release 的 `debug_assertions` 约定验证；不跑 `cargo fmt --all`，仓库明确它不作判据。记录命令、工具链版本、完整场景和失败日志；未能执行的检查标记为未验证。

阶段 6 结束条件：上列失败/竞态场景全部有可重复覆盖，适用测试和整机场景完成并记录；所有 root/后代待领取、容量和回收不变量通过；最终代码只在真实 PR head 的隔离实现分支形成，不在 PR 审查树改写或提交。
