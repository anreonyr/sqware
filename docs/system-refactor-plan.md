# System、接口生成与 Mark 重构

本轮重构消除中央 protocol 库和手写的重复协议表示。
契约由接口提供方声明，通信代码由工具生成；System 按状态和流程归属组织，保留 schedule 和现有产品行为。
当前状态：Loader 兼容基线与接口生成试点已完成。原有 8 个固定基线继续通过，另有生成行为和宏声明检查。
其余接口迁移、传输分离与 System 目录重组尚未开始。文中的目录和 API 除 Loader 试点外表示目标结构。

## 当前问题

- core/serve 分类混合了状态、业务操作、协议接入和程序装配，不能表达模块职责。
- run/install 登记几十个组件中间状态，run/schedule 直接排序组件内部请求管线。
- Control 拥有任务表与实例，其他模块却直接修改这些字段。
- Loader 构建操作和实际请求循环分散在 loader/serve 与 run/loading。
- Hub bridge 混合公共客户端与 System 私有授权流程，形成反向依赖。
- protocol 集中契约、传输和客户端，同一操作被重复写进多套表示。
- Mark 数值已自动计算，但角色字符串、Grant 表、根登记和程序清单仍需手工同步。

## 目标与范围

这轮同时解决三个问题：System 的状态与流程归属、通信接口的单一声明、Mark 的自动生成与装配检查。
验收要求是旧 core/serve/run 分类和中央 protocol 退出，调用方不再手抄接口 Mark。
不顺带重写内核 ABI、替换资源模型、改变任务生命周期或引入新的组件框架。
先保留当前 programs 的多 binary 打包方式；软件目录的职责边界与 Cargo 包装分别处理。

提供方拥有契约，例如在当前源码布局内：

```text
programs/src/system/
├── api/              # 独立 system-api 库：接口声明、公共数据、生成的契约
├── client/           # 独立 system-client 库：生成调用 + 多步骤调用流程
├── app/
├── control/
├── identity/
├── operator/
├── loader/
├── publication/
├── launch/
└── account/
```

这里的 client 指公共使用库。原 identity/client 的安装账和启动重试不是这个公共库的内容。
Hub、Terminal 和驱动的 API 放在各自提供方目录。无需给没有公共接口的程序创建空 API 库。
将来拆独立 binary 包时，api/client 可随所属软件目录移动，接口 ID 与 Mark 不随路径改变。

依赖目标：

```text
wire            → env                 # 小型纯编码与接口描述基础
runtime         → env                 # 任务、资源、内存、schedule
ipc             → wire + runtime      # 通用传输，不收集软件客户端
system-api      → wire + env           # 普通数据、接口声明与生成契约
system-client   → system-api + ipc     # 公共调用与资源交互
System 实现     → system-api + system-client + ipc + runtime
image           → 各软件的纯 API       # 装配检查和公共入口描述
```

wire 已建立，包含 Message、OK、Mark 定义元数据与碰撞检查；其他 common 内容按职责迁移。
env 的内核 ABI 类型与已有 Span 不因目录整理而整体迁移。
API 之间可以保留真实的数据依赖，例如 Hub 引用 System 的公共身份类型。

## System 的状态与流程归属

| 当前内容 | 目标归属 |
| --- | --- |
| boot、life、run/bootstrap、run/execute | app 的引导、内部服务、程序阶段和执行循环 |
| control/core/unit、core/instance、serve/unit | Control 的任务登记、实例状态与管理入口 |
| control/serve/lifecycle、driver、schedule | control/lifecycle 的队列、cursor、补偿和局部计划 |
| run/frame | 整体退出策略归 app，受管任务推进归 Control，回复归对应接口接入 |
| control/serve/material | Control 的启动物料和授权操作 |
| run/instances | 超时与收割归 Control，入口发布归组件安装 |
| identity/core | identity/book 的主体、联盟、成员关系和绑定 |
| identity/client/install::Roster | System 身份管理，负责派生、绑定、注入和回收 |
| operator/core、operator/serve | 命名树、授权、会话、订阅和接口接入分别组织 |
| loader/core、loader/serve、run/loading | 解析、映射、缓存、构建和接口接入；跨能力交付归 launch |
| run/publication、run/names、run/resource | publication、publication/names、publication/runtime |
| run/connections、run/living | 发布连接维护和撤销观察，经 Control 只读入口观察任务 |
| run/launch、run/hooks | launch 的身份准备、命名空间准备和就绪交付 |
| run/account | account，产品配置与帐号会话机制分开 |
| service/hub/bridge | 公共调用归 Hub 客户端，System 接收授权归 launch/activation |
| system/common 中被其他程序复用的 Machine/face | programs 共享支持 |

Control 唯一拥有任务和实例状态；launch 可以持有等待交付的请求，但不能复制实例状态表。
身份管理器拥有身份安装账，publication 拥有发布账，各自通过明确操作协作。
run/resource 管理运行时命名空间发布，与启动物料分别归位。

依赖方向：

```text
app         → 各组件的安装、计划与管理入口
account     → launch、身份管理、名称注册
launch      → control、loader、身份管理、publication
publication → Control 只读观察、身份查询、Operator 管理入口
control     → loader、启动物料；通过注入 hook 接入跨能力流程
```

组件自己登记私有资源、构造局部计划，app 组合流程入口。继续使用现有 Registry。
app 不引用 Request、Decision、CurrentTip 或私有 Dispatch，也不排序组件管线的内部步骤。
Control 提供 hook 组合点，由 app 注入 launch 子计划；组件交出等待兴趣，由 app 汇总等待。
内部服务接收明确的生命周期信号；app 拥有程序层错误。
叶子模块默认私有，System 内跨能力入口优先限制到 crate::system。

## 保留的 schedule 约束

runtime::schedule 继续提供资源注入、依赖图、顺序编排和可恢复执行。
顺序流程用 Schedule::sequence，局部依赖图用 Schedule::new 和 before；同阶段无依赖节点按注册顺序执行。
节点名称用于标识，不通过名称排序决定执行行为。

Plan::advance 在 Pending 或失败处保留 Cursor，已完成步骤不重复执行。
每个 invocation 独立持有 Cursor；更换计划或开始下一轮时显式 reset。
prepare 缓存资源位置，执行时核对类型，登记顺序变化后重新绑定。

Dispatch::begin 设置预算，select 提交 invocation，take_result 同时消费 invocation 和结果。
skip 占用预算，stop 提前结束。子计划每次推进后占用预算并执行 finish。
Pending 子计划由业务重新入队；finish 自身 Pending 时恢复 finish，不重复选择或推进子计划。
finish 必须消费结果，包含最后一个预算已使用的情况。

## 接口生成的边界

使用普通 Rust 接口和数据类型作为声明，附加固定操作码、Grant 分组、线网布局与资源角色等必要元数据。
复用 mold 现有 syn/quote 和 Frame/WireCodes 能力，新增专门生成模块。
最终具体语法在第一条真实接口上确定，不先制作覆盖所有协议的语言。

| 工具生成 | 显式业务代码 |
| --- | --- |
| 请求与回复表示、编码和解码、大小及边界检查 | 业务状态与任务表 |
| 单次调用的编码、发送、接收、解码胶水 | Build 后 Claim 等多步骤调用流程 |
| 服务端解码、操作识别、交付请求和回复入口 | 身份安装、发布、授权判断和失败补偿 |
| Grant 映射、角色 Mark、描述和登记信息 | schedule、cursor、Pending 与执行预算 |

生成分派入口不同步执行整个任务创建流程。它把请求交给原有队列和计划，结果就绪后显式回复。
生成器不从 PieToken 类型猜资源所有权，也不通过 Mark 标签代替实际凭据验证。
现有报文的整数宽度、字段顺序、计数字段、固定缓冲和尾部规则必须明确表达，不能直接序列化 Rust 内存布局。

Loader 当前公共 build 方法含资源授予、Build/Claim 两次调用、统一时间预算、撤销和清理。
初次迁移生成这两个操作的通信胶水，公共 build 保持显式组合流程。
Build 与 Claim 是不同操作码，但共用一份 Grant；生成器必须分别表达操作和授权边界。

Mark 从稳定的接口 ID、角色 key 或授权面 key 生成。旧接口保留旧字符串映射，新接口使用统一规则。
动作码显式固定，Grant 的内部数组下标与线上码分开；不得让表重排改变线上码。
别名只引用原定义；定义登记一次，多个实例使用同一 READY 通道合法。
每个 API 自动汇总自己的描述，最终 image 汇总实际装配的 API 检查跨软件碰撞。

## 分批迁移

### 1. 固定兼容基线

读取并固定现有 Mark 裸值、操作码、错误码、发现路径和关键报文样本。
优先覆盖 Loader Build/Claim、Identity 变长数据与 Grant、Operator 会话与事件。
基线是独立的旧格式期望，不通过调用新生成器计算同一份期望。
保存已有 accept、product、system-fault 行为及 System 调度顺序要求。

完成条件：后续实现能证明新旧的通信结果一致，不能通过同时修改期望掩盖变化。

Loader 基线位于 `crates/protocol/src/system/loader/tests`，直接编译现有帧、Grant、Mark 和错误码定义，不复制实现。
测试中的固定字节样本和 Mark 裸值独立于被测编码器；替换生成器时应保留这些期望。

| 固定项目 | 当前值 |
| --- | --- |
| Build / Claim 操作码 | 1 / 2；共用 Build Grant，授权码 1 |
| 请求长度 | Build 为 42 + 8 × 参数数，最多 554 字节；Claim 为 17 字节 |
| 回复布局 | status:u8、team:u64 LE、task:u64 LE，共 17 字节 |
| 回复状态 | 成功 0；Unknown 1、BadImage 2、Full 3、NotReady 4、Bad 5、Denied 6；未知状态映射 Bad |
| 回信 Mark | loader-back：0xc1bc7bad2c8f22ca |
| 镜像 Mark | loader-image：0x280b1d49733b5ae2 |
| 授权入口 Mark | loader-entry-build：0xa1c2ab202871949a |
| 限制与发现路径 | 参数最多 64，镜像最多 16 MiB，Claim 期限 3000 ms；svc/sys/loader |

测试还覆盖请求与回复的截断、尾部多余字节、非法请求动作、参数数 0/64/65，以及失败回复的零 task/team。
这些是纯通信兼容检查，不执行资源授予、构建、Claim 交付或 QEMU 场景。

### 2. 建立最小生成基础，贯通 Loader

提取最少的纯编码和接口描述基础；原 protocol 临时重导出这些类型。
在提供方目录建立纯 system-api，新增接口生成模块，以 Loader 作为第一例。
采用固定旧编码与 Mark，不在试点中升级版本或更换传输。
替换 Loader 手写请求/回复编解码、Grant/Mark 和单次调用胶水。
暂用原传输实现接入真实 Loader 请求队列，保持 launch 和 Control 实例登记流程。

完成条件：生成的 Loader 客户端和实际服务端贯通；兼容测试和真实构建/Claim 流程通过。
该接口只有一份权威声明；旧路径只重导出新定义，不保留另一份手写协议实现。

本批已落地：

- `programs/src/system/api` 是纯 system-api 库，依赖 env、wire 和编译期 mold，不依赖 runtime、protocol 或程序实现。
- `api/src/lib.rs` 中的 Loader 内联模块用 `#[mold::interface(id = "sqware.system.loader.v1")]` 声明。
  通道声明稳定 key 和旧名称，Grant 声明固定授权码，具名请求变体声明操作码、Grant 和兼容帧名。
- 宏生成 Ask/Claim 帧、操作常量、Wire 编解码分派、回复 Message、Grant 查询、Mark 与 REGISTRY。
  字段编码复用 Frame/Span，错误编码复用 WireCodes。原有 frame/grant/marks 路径只引用新定义。
- Loader 客户端调用生成的 Wire::store，服务端队列调用生成的 Wire::take。
  资源授予、Build→Claim、时间预算、清理和 schedule 仍由现有显式流程推进。
- Loader 独立拥有 Fail；与 Control 交界处显式转换。Mark 根检查读取生成的完整 REGISTRY。
- 提取公共 Message 后，Identity 元组请求改为具名 Request 包装；宿主测试检查包装与旧请求字节一致。

当前宏覆盖具名请求变体和回复结构，尚未生成传输 I/O 或多形回复；这些随下一批接口与客户端迁移扩展。
本批相关宿主测试共 32 项：Loader 10、生成宏 5、Identity 7、Mark 3、System 边界 7。
programs 全目标编译和 QEMU accept、product、system-fault 均通过。

### 3. 分离通用传输，迁公共接口

将 hand/rack/session 迁 ipc，依赖纯 wire 和 runtime；调试输出归 runtime。
system-client 接入新传输；逐个迁 Identity、Operator、Control 的接口声明和客户端。
Identity 作为较复杂的第二例，验证有界变长数据、分页、多个授权入口和权威验证。
各域自行拥有错误与回复；保持原数值和布局，程序显式做域间转换。
Account 不再借 Loader 客户端类型表达自己的创建结果。

共享 injected authority 识别进入 System 客户端库，保留现有验证。
System 的身份安装、启动重试和授权策略留在实现。
拆 Hub bridge：公共调用归 Hub 客户端，System 接收授权归 launch。

完成条件：System 的纯 API 不依赖 runtime/ipc/程序实现，可直接运行宿主编码测试。

### 4. 消除中央 protocol 与手抄 Mark

把 Hub、Terminal 和驱动接口声明迁到各自提供方，客户端随提供方维护。
逐项处理原 common：路径、名称和身份等内容按真实拥有者与复用关系归位，不整体改名搬进 wire。
image 及其 #[path] 编译的 program.rs 直接依赖纯 API，引用生成的入口描述。
启动与部署关系仍在程序清单，只有通道/入口标识改为共享描述。
装配处检查跨 API 的 Mark；动态名称的使用与静态定义检查分开。

完成条件：删除 protocol crate、工作区成员、Cargo 依赖和所有旧导入；兼容重导出也删除。
公共接口无需另写 marks.rs、Grant 表或根 GROUPS 清单。

### 5. 收拢 System 的状态、计划和目录

Control 收起任务表、实例和 Loader 字段，提供必要的操作、观察和 hook 组合入口。
launch 拥有准备与交付请求，不复制任务状态；app 注入必要子计划，打断循环依赖。
组件自己登记私有资源、构造局部计划；app 只组合稳定流程段，继续使用当前 Registry。
拆 run/frame 的程序退出策略、任务推进和请求回复，分别归 app、Control 和对应 IPC。
publication 收拢发布、names 与 runtime namespace；启动物料仍归 Control。
account 保留当前行为，把帐号/镜像选择与运行机制分开。
按上述归属迁移目录，消除 core/serve/run；公共软件客户端与私有管理通道不再同名混淆。

完成条件：顶层安装和计划不引用私有 Request/Decision/Dispatch；组件内部修改不要求同步修改 app。

### 6. 收缩公开面并完成验收

叶子模块私有，组件根只暴露必要入口。共享 Machine/face 支持不再由 System 私有目录提供。
测具通过窄的 fixture 接口观察状态、提交操作和注入故障，不直接改组件字段。
删除全部旧路径、迁移兼容层和临时重复定义。

最终检查：

- 纯 API 和生成器的宿主测试，包括旧格式兼容、非法长度/动作与 Grant 映射。
- image 宿主构建，验证清单能消费纯 API 并检查装配 Mark。
- programs 全目标编译，System 调度与模块边界测试。
- QEMU accept、product、system-fault 场景。
- 依赖检查：纯 API 不依赖执行库，ipc 不依赖软件实现，普通调用方不进入 System 私有目录。

各批次验证通过后提交，再继续下一批。临时重导出必须有明确删除阶段。
第 2 批是第一份可独立审阅的实现：生成 Loader 接口并通过真实调用，而非先全树移动目录。

## 当前可用的验证命令

以下命令对应当前仓库；接口与测试迁移时同步更新路径。

```sh
cargo check -p programs --all-targets --offline
cargo check -p system-api --target x86_64-unknown-linux-gnu --offline
cargo test -p mold --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/runtime/src/schedule/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/protocol/src/common/marks/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/protocol/src/system/identity/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/protocol/src/system/loader/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/system-shape/Cargo.toml --target x86_64-unknown-linux-gnu --offline
nu scripts/qtest.nu --package kernel --scene accept
nu scripts/qtest.nu --package kernel --scene product
nu scripts/qtest.nu --package kernel --scene system-fault
```
