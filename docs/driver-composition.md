# Driver Composition

## 合同

- 基线为 `b08b9ff4`；保留回收完成点记时、launch 通道复用、command 释放和 1024 代回收判据。
- 一个通用 Daemon 守护声明选定的 Driver；Control 唯一负责实例生命周期。
- Daemon 不执行硬件恢复，不复制 Task/Team 生命周期账，不直接 join/slay/oust。
- Assembly 管理装配、资源交付和通用退休依赖；System 不解释 DMA。
- Hub 管设备目录、认领和绑定身份；设备控制 Driver 管具体硬件协调进度。
- PCI 管总线机制；IOMMU 管 Domain、IOVA、翻译及持有；设备 Driver 管协议和停止确认。
- 保留 Traverse 的授权、实际驻留和范围检查；本轮包含通用精确派生及 AccordWindow 迁移。
- β 关键 Driver 自身死亡后安全终止，不承诺自动重启或继续运行；终止保护须先于引用清理。
- 用户成本约束优先：只用当前 Codex 主会话；独立审查使用获授权的 OpenCode DeepSeek V4.1 Flash。

## 完成门槛

| 项目 | 当前证据 |
| --- | --- |
| A1：通用退休依赖替换 System HubDma 闸门 | 已实现全局及静态闸门，宿主和真实Ruin/forced/accept通过 |
| A2：Control 唯一管理执行实例生命周期 | 已实现：Hub 执行阶段机与直接 join/slay/oust 移除，逐实例目标退休、交回队列和调用方迁移；原生新路径未验 |
| A3：设备协调迁入设备控制 Driver | 未实施 |
| A4：公共目录与来源元数据分离 | 已实现，相关宿主与真实产品景通过 |
| A5：PCI 不解析 VirtIO 设备协议 | 未实施 |
| A6：绑定保留首次失败 | 已实现，宿主回归通过 |
| A7：关键常驻域引用清理前安全终止 | 内核机制四/八核基础通过；Control及Driver装配未实施 |
| A8：Domain 与 IOVA 范围共同标识 | 未实施 |
| A9：IOMMU 分配 IOVA，绑定编号只表达身份 | 未实施 |
| A10：显式资源交付与身份核验 | 未实施 |
| A11：逐绑定异步推进 | 未实施 |
| A12：单端点限制收敛到实现 | 未实施 |
| A13：Derive 加 Accord 替换 AccordWindow | 全部调用方与旧 ABI 已迁移；原生窗口、真实失败和八核128代通过 |
| 单个 Daemon 真实装配与受限守护 | 未实施 |
| 双设备、双 Domain、相同数值 IOVA | 未实施 |
| 最终固定源码四/八核与 1024 代回归 | 未实施 |

## 第一检查点：首错与目录来源

`Entry` 只保存目录公共信息；`Cell` 和待发布的 `Enrollment` 保存
`origin: Option<DeviceId>`，复用既有身份，不引入硬件类别枚举或第二设备账。
普通登记没有 PCI 来源；发布、认领、失败重试和 vacate 均保留原关联。
本检查点不改变当前 requester 唯一性或真实 owner 授权。

`Binding.failure` 只记录第一次失败。后续失败不能覆盖它，完成记录仍可查询；
新绑定从无失败开始。返回值继续表达当前操作的结果。

验证记录（2026-10-21）：

- 修改前 retention 40 项通过；workspace dev check 通过，存在既有警告。
- 修改后 retention 42 项、capability-import 8 项、system-shape 20 项通过。
- 修改后 workspace dev check 通过；未为消除既有警告改写无关代码。
- 四核 `iommu-product` 真实产品景通过，wrapper 退出 0；现场为
  `target/driver-composition/pr1/iommu-product/`。
- 迁移夹具首次运行 41 通过、1 失败：第二个设备登记重建 Effects，丢失已激活 coalition。
  修正为同次登记共用 Effects 后 42 项通过；原授权及操作序号断言保留。
- 静态自查确认 pending 登记隐藏、来源转移、真实 owner 核验和未知回包重试规则保留。
  DeepSeek 已完成完整架构方案审查，主线程逐项核对；该差异尚无独立复跑结论。
  未声称完整重构验收完成。

实际命令：

```sh
cargo test --manifest-path programs/tests/retention/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/capability-import/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/system-shape/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo check --workspace --offline
CARGO_INCREMENTAL=0 QEMU_SMP=4 TRACE_OUT="$PWD/target/driver-composition/pr1/iommu-product" nu scripts/qtest.nu --package kernel --scene iommu-product -- --features semihosting
```

## 独立方案审查核对

2026-10-21，通过用户授权的 `opencode-go / deepseek-v4.1-flash` 对固定基线
`b08b9ff4` 进行只读审查。未运行测试，不能把该审查作为原生通过证据。

采纳的约束：

- Hub 的 `StateInstance` 查询仍依赖真实 owner；守护委托不能靠改 owner 实现。
- 静态 Ruin 的名称闸门与 System 的 HubDma 依赖须共同迁移，不能只改全局退场。
- IOMMU 单 Binding、全局范围冲突和全局队列 disable 必须一起处理，不能仅给范围加 DomainId。
- 地址分配权迁 IOMMU 时，Acquire 返回实际地址，不继续由 Hub 提供权威 IOVA。
- Recovery 的单设备与映射布局必须扩展，不能只把 Hub 的步骤搬过去。

未采纳的建议：

- 把 Execution 状态机迁 Daemon：违背 Control 是生命周期唯一权威的用户合同。
- 另建 base/len 授权账：内核 `pend/accord.rs::effective_window` 已使用 Grant 的窗口，
  并检查父范围、继承、溢出和只收窄。Derive 应复用它，不复制状态。
- 把内部 Declaration 当成装配入口：公开 `ControlConfig` 包含 after/ending/ready/construction，
  内部 Table::Declaration 的 name/announce/restart 是其运行投影。
  新策略须接入实际装配声明及授权机制，不能另建按名字分派的 Driver 特例表。

## 第二检查点：精确派生基础

唯一外部 worker：`opencode-go / deepseek-v4.1-flash`，会话
`session-f46cff24-c892-4d07-85c9-f3e40cefb51d`。只允许实现 ABI、内核共享授权发布及相关测试；
不提交、不推送、不启子 agent。主线程接管独立验证与集成。

- `Pie::Derive(src, subset, mark, GrantRange)` 在调用者表内派生，使用新 slot 13。
- slot 12 历史上曾用于 Forget，不能复用；slot 11 暂保留到全部调用方迁移完成。
- `GrantRange::Inherit / Pages` 使用三字编码，来源绝对页坐标不与 ViewSelection 混用。
- 复用内核有效窗口、权限、ONLY、父子撤销和同步机制，不建立另一份范围账。
- 本检查点不等于 A13 完成；Resource/供给/Driver/测具仍须迁移后删除旧 ABI。

后续生命周期迁移已确认可复用 `Unit::Join(Team, receive=false)` 的 `Complete`：
内核同时检查不再 constructing 且全树 all_reaped，能够确认整个执行域停止，
不需要用 Oust 充当唯一停止证明。Control 应使用该事实分离 CPU 停止与资源回收；
不能把 `join_task` 或代表任务的退出结果当作整个 Team 已停止。

验证记录（2026-10-21）：

- 外部 worker 实现基础 ABI/内核共享路径及测试，执行 pend 51 项、workspace dev/release
  check 和 kernel tests check。主线程在其停止后接管差异；未声称 worker 完成全部 A13。
- 主线程独立执行 pend 51 项、env 38 项，全部通过。
- 首次四核原生测试退出 101：测试把 `[2,3)` 误判为越出 `[1,3)`。
  改为真正越界的 `[3,4)`；保留原断言强度，未修改生产权限检查。
- 主线程补验本地派生后跨任务 Accord、精确一页映射及父撤销清理远端后代。
  修正后的四核原生 `local_derive_inherits_and_narrows` 通过。
- 首次失败与修正成功分别保存在 `target/driver-composition/derive/local/` 和
  `target/driver-composition/derive/local-fixed/`，没有覆盖失败现场。

```sh
cargo test --manifest-path kernel/tests/pend/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test -p env --target x86_64-unknown-linux-gnu --offline
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true QEMU_SMP=4 TRACE_OUT="$PWD/target/driver-composition/derive/local-fixed" nu scripts/qtest.nu --package kernel -- --features semihosting -- local_derive_inherits_and_narrows
```

## 生命周期迁移的实施约束（待实施）

- 装配声明列出必需退休提供方及入口，运行时从声明建立依赖；发现失败、超时、失联均为
  Pending，不能从参与集合移除。内部 Declaration 是装配配置的投影，不是第二配置权威。
- 通用退休请求携带实际目标实例身份；提供方自行解释该实例的资源责任。
  静态 Driver 退休和动态业务实例退休都走同一机制，不再按四个 Driver 名称分支。
- 退休 RPC 使用已有 `ipc::rpc::request::Sender::begin` 和在途请求的 `poll`，
  不在 System 事件循环同步等待 Hub；否则 Hub 等 Control 停止确认时可能互相阻塞。
- Control 停止请求只在 `Join(Team, receive=false) == Complete` 后确认 CPU 已停止。
  退休计划先等此条件，再等待外部责任，最后撤授权、供给并 Oust；Task 退出不等于两者完成。
- Hub 的执行登记最终只保留绑定准入与认领水位，不保存 Team、启动资源或回收阶段。
  对实例的启动、停止与查询均由真实 owner 通过 Control 完成；不能迁出后在 Daemon 重建状态机。
- 动态程序需要的通用启动载体由 Assembly prepare 交付并持有到退休，
  覆盖未知回复与释放失败；不能发消息后立即 Revoke 撤销尚未消费的载体。
- 全局退场先关闭新建准入、停止动态执行域、结清其退休依赖，再按依赖顺序退休提供方。
  当前无绑定但已创建的执行实例也必须由 Control 结清，不能只统计绑定数。

### Control 停止与回收的实际边界

2026-10-21 在 `dd12da15` 上复查 `kernel/src/core/work/unit/join/mod.rs`：
非消费的 Team 查询仅在 `!constructing && all_reaped` 时返回 `JoinReply::Complete`。
`Reaped` 表示单条成员退出，不能用于整个 Team 的停止证明；TaskId 和 TeamId 均单调分配，
达到上限后拒绝，不复用。外部退休请求可定位真实 TaskId，在途回复仍须核已有 InstanceRef。

第六检查点的第一份外部草稿误用 `Reaped`，并在 Hub 仍会自行 Oust 时让 Control 先 Oust，
还把现有资源拆除子计划误称为外部责任已经结清。该草稿未应用、未执行测试，已退回返工。
这三项必须闭环迁移：完整 Team 停止、外部责任 Done、Control 最终回收。不能通过删除
owner 回执分支、吞掉 Oust Denied 或放宽最终回收检查拼出通过。

## 第三检查点：Resource 与供给精确派生

外部 worker 会话 `session-2a1ed33c-ba68-42f2-9ac6-12bdc7ae841f` 已正常完成，
由主线程复核与补充验证；没有其他 Codex worker。

- Capability 与 Loan 分文件。Capability::derive 接受本任务持有的来源 token，
  返回拥有的本地子能力，能够用于 Pole 来源，不要求先造一个 Hole 包装器。
- Capability::release、Loan::revoke 使用可重试的可变借用；失败保留原句柄，成功后重复调用无操作。
- 供给 PieEffects 将本地 Derivation 与跨任务 Accord 分离。RootAssignment 使用 GrantRange，
  交付总是继承精确父范围，检查及失败根的恢复责任保留。
- Resource 与 System 供给内已无 AccordWindow/grant_window；Driver、测具及旧 ABI 仍待迁移。
- 主线程独立通过 resource 21、supply 23、lease-window 3、operator-watch 5 项。
  operator-watch 首次独立调度遇自动审批超时，未执行；同命令重试后通过。
- 四核真实 iommu-product 与完整 accept 场景均通过，现场为
  `target/driver-composition/derive/supply-product/` 和 `supply-accept/`。
- worker 首次 supply 编译发现 fixture 缺失新 derivation 模块，补齐模块后 23 项通过；
  不把该编译失败计作成功。workspace dev check 保留原有 5 项 warning。

## 第四检查点：Driver 与旧 ABI 迁移

外部 worker 从 `45453c84` 实施，固定 `opencode-go / deepseek-v4.1-flash`，会话
`session-f303d8fb-7f1c-440f-80ec-4ef02a006cf5` 已停止，主线程接管剩余验证及返工。
之后只有主线程写工作区；外部审查/补丁生成应保持只读，避免并发覆盖未提交记录。

- PCI 从宽 aperture 本地派生精确 BAR，保存 ONLY 本地根；交付失败重试复用同一句柄。
- Recovery 与 Work 已检查收到的是请求的完整精确范围，普通 Accord 继承该范围；
  不向接收者扩大范围或权限。测具先 Derive 窄根，再 Accord。
- 旧 class 7 slot 11 ABI、调用方和兼容封装已删除；slot 11/12 测试保持拒绝。
  授权发布只有私有共享后端，Traverse 检查保持不变。
- 主线程审查发现 pending Placement 未纳入 disable/release；新增取消回归修前红（退出101）、
  修后通过。现在禁用前不能释放；释放失败保留同一根并阻止重绑，成功后才允许下一绑定。
- 主线程独立通过 PCI 6、pend 51、device-recovery 5、iommu-pages 14 项宿主测试。
- 首次三项窗口原生测试 2 通过、1 失败：新增收尾错误地用 Revoke 处理来源根，
  并误期望关闭一个来源会删除另一个来源的副本。改用 Release 并保留来源独立性断言后，3 项通过。
  失败和成功现场分别为 `target/driver-composition/derive/retired-window/` 与 `retired-window-fixed/`。
- 旧 ABI 删除后，四核原生 local_derive 1 项通过；真实 iommu-product、start-before、
  start-after、Ruin 和 forced 均通过。Ruin 与 forced 保持辅助 Team 先于 Driver 回收，
  两景最终帧数均为 85→85。现场为 `derive/retired-local/` 与 `derive/final-iommu-*`。
- 八核 128 代首次运行在第 48 代被测试器默认 60 秒时限中断，未完成验收；已完成代的
  回收记录保留。失败现场为 `derive/final-iommu-soak/`。按公开 `--timeout 180` 参数
  重跑，所有代数、顺序、能力数量和最终帧数断言保持原样。
- 八核 128 代重跑通过：64 次正常退出、64 次 owner 死亡，156278 ms；最终帧数
  153→153，逐代 capability=15、launch=1、command=1。现场为 `derive/final-iommu-soak-180/`。
- 四核真实 Reference Release Busy 通过：失败时两枚 IOMMU 引用保留，重试同一绑定，
  最终帧数 87→87。八核真实硬件 CQ memory fault / Fence::Expired 通过：恢复原队列后
  同一 IOFENCE cookie 完成，最终帧数 153→153；不是用伪失败替代硬件超时。
- 当前源码四核整机 accept 通过，现场为 `derive/final-accept/`。
- 本检查点不等于完成完整重构；A1–A3、A5、A7–A12、Daemon 和最终固定源码
  双 Domain/1024 代仍须实施验收。没有据本检查点声称全绿。

## 第五检查点：通用退休闸门

外部作者仍为同一只读 Flash 会话 `session-4c4341ed-f94e-4dd7-a8ca-b80b0876bf9e`；
主线程是唯一 writer，未启 Codex 子代理。ControlConfig 的 retirement 声明定位 provider、
namespace 和 face。运行集合从实际 deployment.start 建立，不从发现成功的面或全 PROGRAMS 推断。
System 只认通用 Pending/Done；Hub 只向真实 sire 响应整机结清，并保留原有安全停止链。
静态 Ruin 按目标声明等依赖，正常和 forced 全局退场共用同一门。

- 使用现有 Sender::begin / Pending::poll 推进请求，不同步等待 Hub 回复。
  面发现仍是向独立 Operator 的有界 find（1秒），并非完全异步发现；失败保持 Pending。
- 首份外部补丁声称边界测试，但实际没有测试用例；主线程退回返工。
  独立编译又发现 interface 缺 metadata 模式、fixture include 路径及缺失方法，均修正后重跑。
- 主线程通过退休边界8项，control-api 8、hub-api 4、device-api 5、interface-marks 5、
  system-shape 20、image 17项。Ruin 追踪判定器14项通过，全部停止及最终回收断言保留。
- workspace dev check 通过，保留既有5项warning；image 首次使用 workspace包选择报未找到，
  改用其独立 manifest 后17项通过，该命令失败未计入通过。
- 真实四核Ruin通过，辅助Team先于Driver回收，帧数85→85；八核forced通过，帧数153→153；
  四核accept通过。现场均位于 `target/driver-composition/retirement/`。
- 新测试的适配与fixture已按功能拆出，每文件一个数据结构；整理后8项重新通过。
  外部只读复核未发现新增确定问题，但未独立运行原生，原生证据来自主线程。
- 这是全局及静态退休闸门；动态实例的具体退休身份、Control生命周期归一与Daemon仍待实施。

## 第六检查点：完整执行域停止前置

Control 的退休 hook 直接查询非消费的 Team `Complete`，不再以主线程退出推断整个
执行域停止。辅助成员仍活着时不会选中退休子计划；未知/失败查询均不能放行。
没有新增无调用的停止接口，没有改变当前 owner 回执和 Hub Oust 的顺序。

- 主线程通过 control-instance 25 项；新增用例驱动真实 hook，先主线程退出、辅助成员
  活着时不选退休，全部成员退出后才选择同一 InstanceRef。
- 外部初次修订把新方法放在未被fixture包含的Control实现中，独立编译失败；主线程改为
  在真实选择点查询，不复制生产方法到fixture。新身份相等断言的Debug编译问题也已修正。
- 八核真实 iommu-product 通过。八核128代首次运行随宿主进程中断，只保留52代记录，
  不计通过；核实旧进程不存在后在新目录重跑，128代通过，156465 ms，帧数153→153。
  原现场 `control/team-complete-soak/` 保留，成功现场为 `control/team-complete-soak-resumed/`。
- 验证记录补丁首次因宿主重启后的工具路径失效而未写入，代码已提交为dc4b52ac；
  本补记补齐该提交的真实证据，不改写已推送提交。
- 本检查点未完成A2；启动载体、逐实例退休义务和Hub重复生命周期阶段机仍须闭环迁移。

## 第七检查点：通用执行域退出保护基础

TeamLife 保存唯一退出策略；可信引导任务获得独立 Guard 授权，和 Retain 分角色，
只有FETCH、不向外授出VEST。Guard/Unguard使用Unit新槽43/44，已查本地历史无占用。
策略安装要求实际Paused；普通Nole、保留权、缺token或非owner不构成退出策略权。
撤保护的外部责任条件由可信Control保证，内核不接收无效的done布尔证明。

- Armed或Tripped的每次死亡都走不返回的终止路径；在slay、reap、trap doom及Pend清理前拦截。
  未启用保护的任务保留原清理行为，不能通过全局testing标志绕过保护。
- 全局停止用单个原子owner发布，避免先发布alarm后发布owner的窗口。peer在撤地址空间租约、
  禁中断后仅回执一次，再进入不返回的HSMStop/循环；主hart等全部回执后永久停住。
  此回执证明CPU静默，不证明设备已经排空。
- 独立复核指出Paused可能来自尚未完成的Debark；主线程补查真实成员状态，on-hart或死亡
  成员均禁止安装/撤除。trap死亡声明与策略修改使用同一commit；父子方向都拒绝重叠保护。
  原生核心回归覆盖Paused但成员仍on-hart、正常撤除后真实清理及父先/子先两种冲突。
- 首份外部草稿在Tripped后允许其他路径继续清理，且testing分支返回cleanup；未应用并退回。
  修订又有基于未应用旧稿的Nole上下文和mail模块上下文，主线程按真实源码精确适配。
- workspace dev check、env初验38项通过；新编码补入后ABI/账本子集9项、Pend51项通过。Pend首次因fixture
  缺退出适配编译失败，补为明确只包含未保护任务的模型；不以它证明Guard安全。它仍有cfg警告。
- 真实内核核心权限/状态用例通过；补验撤除保护后，普通资源经真实hook被清理，也通过。
- 首次critical测试发生在同伴hart启动前，没有静默回执，不算通过；现场guard/critical-4保留。
  专用case复用真实启动流程后，四核、八核分别记录3、7个peer静默回执，目标没有进入引用清理。
  VM永久停止使原qtest按预期超时退出101；严格独立verify判定器接受证据，6项负对照通过。
  不把这两个runner红结果称为测试全绿。命令及边界见kernel/tests/critical/README.md。
- 普通四核iommu-product通过，现场guard/iommu-product，保留引用及正常回收判据未放宽。
- 父子冲突与成员静默修正后，原生core-overlap通过；最终八核critical-8-final仍取得7个
  peer回执且未进入目标引用清理，独立verify接受证据。诊断钩子位于Pend锁外，避免日志锁序影响。
- 尚须Control/Assembly持权装配PCI/IOMMU/Recovery、按退休依赖撤保护，并实测真实Driver
  在DMA责任未结清时意外死亡。当前未声称A7或完整重构完成。

## 第八检查点：Assembly 持有通用启动载体

Assembly 为每个执行实例交付并保存启动 Hole，完成 identity/namespace/supply 后才发布。
Hub 不再创建、填充、交付、收窄或释放启动载体；其 Identify/Embark/Running/Stop/Oust
仍待下一阶段迁移。实例 owner/sire 保持 Hub，没有通过改 owner 绕过实例面的授权检查。

内核在 Unit::Build 保存真实调用者的不可变 Creator，Unit 新槽45只提供身份查询，
不提供创建或生命周期权。Boot/内核直建为0，同Team线程共享真实Creator。
消费者核验注册的bootstrap Mark、Hole种类、alive、真实creator对应的owner/vestor、
精确FETCH、实际消息sender、帧provider以及完整版本/角色。System网关由可信creator签发。
该网关和Operator地址语义不同，不能混用。

载体保存可选根、交付receipt、已填充/收窄进度及首次尝试的固定期限。
临时失败走真实schedule Pending重入；到期先拒绝新effect，再由Control停止实例。
释放失败保留同根；WholeTeamComplete闸门仍在所有退休资源清理之前。
release(Revoke)经内核cull递归处理分支，不依赖先Oust来解除活heir。

测试判据迁移：

| 旧Hub启动载体判据 | 当前生产责任与验证 |
| --- | --- |
| create/populate/deliver/restrict失败不重复已完成动作 | bootstrap-carrier直接编译Assembly源码，逐边界失败、相同root重试及真实schedule Pending重入 |
| 部分启动中止不再发布后续步骤且最终释放一次 | every_partial_prepare_is_releasable_without_publishing_later_steps；Control failed_prepare不能claim/embark |
| 丢Embark回复不再发布载体、不重复启动 | 载体重复prepare不重发；retention保留Hub Embark丢回复同实例断言 |
| 失败release保留同句柄，重复完成不再释放 | release_failure_keeps_carrier_then_releases_once，断言所有尝试均用原root |
| 停止未确认不清理，失败Oust不构成确认 | retention保留Stop/Oust失败和同receipt判据；Control真实hook等待整个Team Complete |
| 多实例互不串扰、非法来源不被接纳 | 独立carrier/child检查，以及creator、kind、权限、sender、provider、角色、版本、短帧和重复候选负例 |

原测试迁移补丁因删除原启动/回滚判据被自动审批拒绝，没有应用。
先新增并实跑替代责任处的测试，再精确调整旧Execution夹具；没有放宽保留引用或最终回收检查。
外部只读Flash复核发现生产hook把临时错误直接转Stopping，主线程修正为固定期限Pending，
并新增真实schedule驱动用例；对释放的疑问经真实内核递归实现核验，无需调换顺序。

已完成验证：bootstrap-carrier12项、control-instance26项、retention42项、interface-marks5项；
system-shape20项、workspace dev check通过，仍有既有5项警告。真实内核UnitBuild/Spawn/Creator用例通过，
包含caller与sire不同、查询真实Creator且child未获得Build权限。

真实产品首次失败于错误System网关，修正后四核product、start-before、start-after通过。
原生Creator夹具首次在Constructing阶段调用hold失败，改走真实Spawn后通过；没有绕过ready检查。
上述失败现场保留在target/driver-composition/bootstrap/的creator-native、product。
修正Pending前八核128代通过，64正常/64 owner死亡，155071ms，帧153→153，
逐代capability15/launch1/command1；现场soak-128。本记录不把它当作Pending修正后的验证。
Flash复核确认Pending修正无新漏洞；其提出的未创建Hole即到期边界也纳入逐边界超时测试，
断言清除Carrier且不调用pie释放。Pending修正后八核soak-128-reviewed通过：64正常/64 owner死亡，
155137ms，帧153→153，逐代capability15/launch1/command1。没有声称完整A2、Daemon或最终1024代已完成。

## 第九检查点（待调用方迁移）：Control 收据交回

Instance以Receipt::Owner/Returned/Consumed保存唯一Team坐标。普通退休完成后仍保留owner
收据；新的ReleaseInstance(操作码11，复用Ruin授权)由真实owner交回最终核销责任，Control仅在
真实Oust成功后记Consumed和回复OK。Busy/Denied/其他失败保留同收据；回复消失后独立重试。
Returned未结清时不驱逐、不允许全局退场，等待间隔保持有界。Dead不能被stop复活。

退休帧增加固定target，0为全局；共享Participant的scope在创建时固定，核对相同target回包。
逐实例调用尚未装配，Hub暂对非零target保持Pending。此接口不能单独证明A2完成。

- 主线程实跑Control-instance32项、Control-api9项、Control-RPC5项、retirement-boundary10项，
  含同收据Busy/Denied失败、辅助成员仍活、真实schedule回收、无新owner调用的独立重试、
  容量不能驱逐未归还/未核销收据、跨实例/全局错scope回包不能放行。
  核销后若schedule尚待finish，依据真实Consumed继续退休分派，不再依赖已失效的Team查询；
  新用例保留原核销记时、不重复Oust。没有把Unknown/Denied用作该分派证明。
- workspace dev check通过，保留既有5项警告。四核accept和iommu-product通过，仅计一般回归。
  accept未执行loader.rs旧acceptance函数，因此撤回曾误计的新增Release原生证明并移除未执行断言；
  新Release尚需接入真实产品调用方，再取得原生证据。现场receipt/accept、receipt/product。
- 外部整包草稿因缺grant、Assembly接点及fixture迁移被自动审批拒绝，未直接应用。
  根线程逐文件重写，补齐sender隔离、模块及调用点、真实核销计时和测试后验证。
- Flash复核提出将Denied/Absent归一Consumed，未采纳：这违反真实核销证明要求。
  老调用方自行Oust后再交回属于不兼容的双重消费，必须把调用方改为唯一Release路径，不能伪造完成。
  它所列的活实例/回复消失/Denied测试缺口已有对应31项用例；Returned到finish的Room为内部
  不变量错误，实际reclaim已限定owner早退，不能用Pending掩盖未核销的完成结果。
- 本检查点保留在本地，迁移Hub及普通owner调用方后再推送。当前Owner收据不驱逐会使未迁移
  调用方在256条历史后背压；这是明确的中间缺口，未声称1024代通过或A2完成。

## 第十检查点：A2 实例生命周期归一

Hub 不再保存 Team、Running/死亡阶段或执行状态机。核心只保留执行实例准入身份、认领水位、
启动期限、闭合标记和有界闭合历史；适配层保留有界 launch/return RPC 工作队列。启动报告不再
自报授权：Hub 先通过 Control 的 `StateInstance`（真实 owner 核验）认证，再经同一个实例面
`Embark`；丢回复时重试同一实例，Control 的重复 Embark 幂等。启动超时只关闭准入并排交回，
不伪造 Running/死亡事实。

退休语义：

- 来自真实 System sire 的非零 target 才推进该 task 绑定的 `StopCpu -> Invalidate`，重复
  target 不重放已完成步骤；没有绑定也可完成该实例资源责任，不存在对应 Hub 准入时不伪造
  Oust 完成。绑定 Fault/Normal、Sensor 失活判据保持不变。
- Hub 在资源关闭、未绑定实例的 target、启动超时和全局 settle 时排真实 owner 交回；
  只有确实经实例面确认 owner 的实例才排 Release，别的 owner 只得到 no-receipt 结果，不留下
  永远挂起的假 job。逐实例 target 回复不等待 Release 最终回复，避免 Control 等资源 Done、
  Hub 等 Oust 的循环。
- 全局 target 0 在所有绑定结清且自身已接交回工作完成后才 Done；全局退场先关闭新构造准入、
  停止动态执行域并等其结清，之后才按声明推进退休提供方，不会让全局 Hub 请求等一个尚未被
  Control 停止的执行域。
- `Participant` 从 `app` 迁到 `assembly`，全局 Settle 与逐实例退休共用同一实现。Assembly
  的实例退休计划第一步推进该实例声明的提供方责任，之后才撤 identity exports、namespace、
  construction claim、supply、bootstrap，最后 Control 核销。动态 `device-work` 三张声明
  已列出 Hub retire 依赖。
- Login 与 Shell 的最终核销改为 Control `ReleaseInstance`；Shell 用现有异步 Work 队列推进
  Release，不再同步阻塞 pumps，也不再直接 Oust。Hub 对执行实例不再调用 kernel join/slay/oust。

宿主验证（2026-10-22，基线 `4a9f7450` 之上）：

- retention 41、control-instance 32、control-api 9、control-rpc 6、retirement-boundary 10、
  system-shape 21、shell 14 全部通过。
- 另外 control-query 9、control-lifetime 8、bootstrap-carrier 12、hub-api 4、
  interface-marks 5、device-api 5、identity 19、startup-authority 6、shutdown-policy 8、
  loader-client 4、namespace-installation 4、operator-watch 5 通过。
- `cargo check --workspace --offline` 通过，保留既有 5 项 warning（League 可见性、未用
  device-work ENTRY、三枚 subset 常量）。
- retention 的旧 Hub Execution 用例已改为新责任：准入重放、水位、超时、target 推进、
  全局 settle 和闭合历史；不是删除旧断言。

未验证与已知缺口：

- 本工作区禁止运行 QEMU，因此没有新的原生 target 退休、Hub Release、WholeTeamComplete
  后资源顺序或 128/1024 代证据；四核/八核结论未在本检查点重跑。
- 新 Hub launch/return RPC 队列只有类型与源码边界检查，没有独立的宿主 RPC 驱动测试；
  其正确性仍需原生产品景证明。
- A3 设备协调、A5、A7-A12、单 Daemon、双 Domain 同数值 IOVA 与最终 1024 代仍未实施；
  本检查点只声称 A2 的宿主和代码路径闭环，不声称整个重构完成。

## 第十一检查点：根审查返工（A2 handoff 适配器）

根审查候选 `f669396e` 后，在同一 worktree 修正：

- `returned`：已认证/Root 退休的 Release 只在 OK 时 Released；Unknown、Denied、传输超时
  一律保留原 task 和同一交回责任，不调用 `ledger.released`，global idle 不被错误放行。
  只有未认证的纯 launch/非 owner 查询返回 Unknown/Denied 才可 NoReceipt。
- `launch`：认证真实 owner 后、发 Embark 前先建立有界准入；`Stopping`/`Dead`/无 Team
  不作为新的开放准入。丢 Embark 回复重试同一实例，不重复准入；失败或超时关闭该准入并
  可靠排交回。DMA 认领不再同步查 Control，只接受已经真实 owner 认证的准入。
- 交回不再等资源关闭：绑定进入 Closing 即非阻塞请求 Release；逐实例 target 的
  资源 Done 只要求交回工作已可靠承接，不等待最终 ACK。`StopCpu` 仍只由真实 target
  越过，并保留 `hub: binding X execution domain stopped (tid=Y)` 一次性事件。
- 队列先预留再消费：launch 通道先 `try_reserve` 再 pull；`expire_admissions` 只在
  交回被可靠接纳后关闭准入；交回队列满时保留原责任并让 target/global 保持 Pending。
  队列容量在 Hub 启动时 fallible 预留。
- Shell `Work::Reclaim` 不再阻塞同 pipeline 的 pumps：Reclaim pending 时继续推进 IO；
  pending 使用 RPC 自身 deadline，不在每个 pump tick 替换请求；timeout 才重发；
  job 进入 terminal 后仍继续交回未结清成员。`StopCpu` 不再保留空转发函数。

新增真实生产适配器宿主测试：

- `programs/tests/hub-handoff`：直接编译生产 `serve/{launch,returned,execution}.rs`
  与真实 Hub core，注入可控 Control 响应。覆盖：Embark 前准入已存在、丢回复不重复准入、
  `Stopping` 不开放、trusted Release 的 Unknown/Denied/超时后成功仍保留责任、
  未见 launch 的早期构造 target 认证后交回、异 owner 无收据、绑定关闭前已发 Release、
  交回队列满时责任不丢、重复 target 去重、launch 队列有界、队列预留 fallible。
- `programs/tests/shell-adapter`：直接编译生产 `shell/adapt/controller.rs`，用受控延迟
  Release RPC 和持续 pump 的场景验证：Reclaim pending 不阻塞 pump、pending 不被每 tick
  替换、RPC timeout 后重发、terminal job 仍交回成员。

验证结果（2026-10-22，基于 `f669396e` 之后的返工）：

- hub-handoff 10 项、shell-adapter 2 项、retention 41 项通过。
- control-instance 32、control-api 9、control-rpc 6、retirement-boundary 10、
  system-shape 21、shell 14 通过；`cargo check --workspace --offline` 退出 0，
  保留既有 5 项 warning。
- 未运行 QEMU；无原生 target/Release/1024 代证据。Shell 适配器测试使用宿主 fake RPC
  与 fake Objects，不是真实内核执行证据。

## 第十二检查点：主线程补齐交接边界（尚待原生验收）

Flash 候选已封存在 `d6f83aeb`。主线程新增四条回归，修前各有真实失败：
认证成功但准入和返还队列都满时丢任务；未见启动通知的 target 发现超时后抹掉责任；
Shell 等待 Release 时遗漏管道事件；Control 已 Oust、回复尚不可见时 Shell 再 join 报错。
修后 hub-handoff 12 项、shell-adapter 6 项通过。

- Abort 留在原队列直到返还工作可靠承接，保留已经核验的 owner 事实；返还成功撤除同任务的在途启动。
- 每次发现与 Release RPC 独立有界，超时只重试；只有真实 owner 查询的 Unknown/Denied
  能排除未认证收据，已认证收据绝不据此完成。
- Shell 同时登记在途 Release、其他成员和管道事件；等待交回的成员不再重复 join。
  核实 core 的 fail 先进入 Cancelling，全部结果与 drain 完成才进入终态，未新增第二份终态事实。
- 退休向量在构造前 fallible 预留。通道连接移入已有 Prepare hook，在 Assembly 保存安装载体后执行，
  失败沿现有 Retire hook 回滚，不留下已构造而未登记的 Team。
- 核心内测试专用构造方法已移除；绑定测具改走实际登记、Claim 和 Close。
  Shell 私有字段夹具移到 tests 目录，生产模块不依赖测试。
- retirement-boundary 10、bootstrap-carrier 12、system-shape 21 通过，workspace check
  保持既有 5 项 warning。退休边界测试首次因审批服务超时未执行，单次重试成功。

首次四核 iommu-product 的内层 scene 通过、最终帧 85→85，但外层严格判定缺少旧有
`hub: execution instance launched` 事件，整体退出 1。失败现场在本隔离工作区
`target/driver-composition/handoff/product-root/`。启动成功回复处已恢复该一次性事件，
待固定源码重跑。未据内层通过声称原生验收通过。

固定候选的外部只读审查建议被逐项核查：静态 gate 自身推进 participant，并不依赖
全局 operations 为空；Retire 子计划之前已有非消费 WholeTeamComplete gate；
把 trusted Unknown/Denied 当完成违反用户合同；静态 attach_service 的底层 mint 已处理失败清理。
因此没有采纳删除退休门或伪造完成的建议。最终 1024 代、完整 Daemon 和其余架构条目仍未完成。
