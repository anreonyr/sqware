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
| A1：通用退休依赖替换 System HubDma 闸门 | 未实施 |
| A2：Control 唯一管理执行实例生命周期 | 未实施 |
| A3：设备协调迁入设备控制 Driver | 未实施 |
| A4：公共目录与来源元数据分离 | 已实现，相关宿主与真实产品景通过 |
| A5：PCI 不解析 VirtIO 设备协议 | 未实施 |
| A6：绑定保留首次失败 | 已实现，宿主回归通过 |
| A7：关键常驻域引用清理前安全终止 | 未实施 |
| A8：Domain 与 IOVA 范围共同标识 | 未实施 |
| A9：IOMMU 分配 IOVA，绑定编号只表达身份 | 未实施 |
| A10：显式资源交付与身份核验 | 未实施 |
| A11：逐绑定异步推进 | 未实施 |
| A12：单端点限制收敛到实现 | 未实施 |
| A13：Derive 加 Accord 替换 AccordWindow | Derive、Resource 与供给已验证；Driver 与旧 ABI 待迁移 |
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
