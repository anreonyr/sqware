# System、接口生成与用户态基础重构

本轮重构消除中央 protocol 库、总括的 runtime 库和手写的重复协议表示。
契约由接口提供方声明，通信代码由工具生成；System 按状态和流程归属组织，保留 schedule 和现有产品行为。
当前状态：Loader 兼容基线、接口生成试点、schedule 独立及 runtime 全部迁移已完成。execution 与 resource 已接入全部调用方，旧 runtime crate、工作区成员、依赖和源目录已删除。
纯 wire 字节 codec、通用 IPC、提供方 API/客户端归位及持久会话失败隔离已落地；protocol crate 与兼容层已删除。System 状态和目录重组仍未完成。文中的目录和 API 除已落地内容外表示目标结构。

## 当前问题

- core/serve 分类混合了状态、业务操作、协议接入和程序装配，不能表达模块职责。
- run/install 登记几十个组件中间状态，run/schedule 直接排序组件内部请求管线。
- Control 拥有任务表与实例，其他模块却直接修改这些字段。
- Loader 构建操作和实际请求循环分散在 loader/serve 与 run/loading。
- Hub bridge 混合公共客户端与 System 私有授权流程，形成反向依赖。
- protocol 集中契约、传输和客户端，同一操作被重复写进多套表示。
- Mark 数值已自动计算，但角色字符串、Grant 表、根登记和程序清单仍需手工同步。
- runtime 混合程序入口、堆、TLS、域内并发、资源生命周期、通信连接和计划执行；port 与 session 重复管理连接关系。

## 目标与范围

这轮解决 System 的状态与流程归属、runtime 的职责分离、通信接口的单一声明、Mark 的自动生成与装配检查。
验收要求是旧 core/serve/run 分类、中央 protocol 和总括 runtime 退出，调用方不再手抄接口 Mark。
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
wire                                  # 纯字节 codec 与 Message，不依赖环境类型
env             → wire                # ABI 与环境类型自己的 codec、Mark 登记
resource        → env                 # dock、port、bell、pile 与私有句柄基础
execution       → env                 # room、unit、memory、lock、boot
schedule                              # 纯计划执行；不依赖 env 或执行库
ipc             → wire + resource + env # 通用通信、RPC 与环境时间，不收集软件客户端
system-api      → wire + env           # 普通数据、接口声明与生成契约
system-client   → system-api + ipc + resource + env + wire + execution
System 实现     → system-api + system-client + ipc + resource + execution + schedule
image           → 各软件的纯 API       # 装配检查和公共入口描述
```

wire 承载通用 Field/Span、基本字段与变长数据的字节编码、游标和长度辅助、Message。
env 保留寄存器 Wire/FromPair、环境句柄自己的字段实现和 Mark 元数据及碰撞检查；角色常量仍由提供方声明。
旧 env 字节 codec 路径可重导出 wire 的同一实现，不保留第二份 trait 或编解码正文。
API 之间可以保留真实的数据依赖，例如 Hub 引用 System 的公共身份类型。

## env 与用户态基础的边界

env 定义内核与任务共享的环境契约及发起环境调用的薄入口：调用号、参数与返回布局、失败码、权限位、原始句柄和调用骨架。
ledger 的启动参数、资源目录、manifest 与 capsule 格式继续属于 env；实际程序装配和资源分配策略属于镜像工具及启动代码。
env 不管理等待重试、资源对象收尾、通信会话、服务发现或业务授权。Mark 的值类型属于 env，具体接口角色由提供方 API 声明。
通用 Field/Span 属于 wire；env 的兼容路径只重导出。生成器支持显式 codec 路径，使纯数据帧不必依赖 env。
字节 codec 与寄存器打包分别定义，不引入 env 与 wire 的循环依赖。

resource 保留 dock、port、bell、pile 的模块名和类型名：

| 模块 | 职责 |
| --- | --- |
| dock | 内存能力和映射；区分撤图、能力释放及外来资源失效 |
| port | 通信两端配对、对端识别和字节收发；明确入口与回信端所有权 |
| bell | 通知等待、响铃和确认；保留中断确认语义 |
| pile | 多路等待与状态订阅；返回就绪提示，由调用方复核状态 |

原始句柄构造集中在创建及导入边界，不提供任意 token 到可信对象的普通转换。
本地持有、借用和派生授予分别承担 release、撤图及 revoke；seal 不作为通用对象析构。
类型限制合法操作，但撤销、对端退出和内核拒绝仍是运行时结果。
外来内存不能仅凭本地 Rust 借用就提供长期有效的安全 slice；Loader 保留受控快照与撤销失败语义。
port 拥有两端关系；ipc 的类型化消息、会话交互和共享缓冲建立在资源对象上，避免复制连接状态。
请求回复、订阅与共享缓冲保留各自的背压和生命周期，不强制使用同一种 RPC。

execution 的公开子模块使用内核词汇，不另设 thread/sync/exit 顶层分类：

| 模块 | 职责与迁入内容 |
| --- | --- |
| room | 当前任务运行控制；park、wait/wake、reap 与用户态组合 |
| unit | 执行单元创建、放行和等待结束；task 的闭包结果回收及私有 TLS 支持 |
| memory | 程序内存分配与归还、allocator；能力映射仍归 dock |
| lock | 用户态同步原语 |
| boot | 程序入口、初始参数、初始化和 main 返回结果适配 |

同名 env 模块提供单次调用契约，execution 提供用户态组合和生命周期管理。
不复制内核 scheduler/conductor/messenger 的实现目录。Control 的任务登记和 launch 的跨软件启动流程仍属于 System。

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

schedule 独立后继续提供资源注入、依赖图、顺序编排和可恢复执行。
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

当前宏覆盖具名请求变体和回复结构，尚未生成传输 I/O 或多形回复；这些随资源边界确立后的接口与客户端迁移扩展。
本批相关宿主测试共 32 项：Loader 10、生成宏 5、Identity 7、Mark 3、System 边界 7。
programs 全目标编译和 QEMU accept、product、system-fault 均通过。

### 3. 独立 schedule

将 runtime/src/schedule 迁为独立 schedule 库，保持现有语义和宿主测试。
迁移全部用户态调用方、System 边界测具与测试路径；不改内核内部的同名 runtime 模块。
不在此批改节点排序、Cursor、Dispatch 或组件工作流。

完成条件：schedule 不依赖 env/resource/ipc/execution；调用方不再引用 runtime::schedule；原有调度测试与程序编译通过。

本批已落地：

- 独立 `crates/schedule` 采用 no_std 库入口，无 Cargo 依赖；原调度实现保持不变。
- programs 的所有调用方直接依赖 schedule；删除 runtime 的原模块和兼容路径，内核内部 runtime 不变。
- 29 项既有调度测试迁到 `crates/schedule/src/tests`，保持测试内容与私有状态检查；System 边界检查扫描新库。
- programs 全目标编译、29 项调度测试和 7 项 System 边界测试通过。
- schedule 独立宿主编译、无依赖检查及 QEMU accept、product、system-fault 均通过。

### 4. 迁程序执行支持

按 room/unit/memory/lock/boot 归位现有 task、heap、TLS、exit 和程序入口支持。
同步迁移 #[entry] 生成路径、programs 入口装配及测具；明确 TLS 由任务创建与退场流程成对管理。
保留启动、等待、退出、分配与回收行为，不把 System 任务管理状态迁入 execution。

完成条件：执行支持不引用 runtime 旧实现；验证程序启动退出、域内并发结果与内存回收。

本批已落地：

- `crates/execution` 公开 room、unit、memory、lock、boot，直接依赖 env、talc、spinning_top。
- heap、TLS、闭包执行与结果仲裁、启动参数和退出结果适配从原 runtime 迁入；TLS 留在 unit 私有模块。
- programs 只保留汇编启动和 panic 链接装配，通用返回结果适配调用 execution::boot；mold 入口提示与所有调用方路径同步更新。
- 页粒度由 env::PAGE_SIZE 定义，capsule 与执行、资源、装配代码共享，不产生 execution/resource 的交叉依赖。
- `crates/execution/tests` 的 2 项宿主测试检查堆分配、失败、回收、并发释放以及初始参数指针和计数。
- TLS、闭包执行与任务退场的实际运行由 accept、product、system-fault 场景验证。

### 5. 用 Loader 确立资源边界

建立 resource 的私有句柄基础和 dock/port/bell/pile，优先贯通 Loader 所需的内存、发送端、回信端和派生授予。
列清每种对象的创建者、持有者、借用者和收尾动作，再迁接口；不直接给全部旧句柄补统一 Drop。
客户端收拢授权、期限和失败清理；服务端把外来报文及内核来源转换为经过验证的内部请求。
保留映像快照、Build→Claim、等待交付及回收语义；线上资源编号和 Mark 不作为授权证明。

完成条件：Loader 主流程不散落裸句柄构造和清理；覆盖授权失败、构建失败、Claim 超时、对端退出和映像撤销。

本批已落地：

- `crates/resource` 保留 dock、port、bell、pile；正式模块使用私有 Hole，原始查询和未验证适配集中在 raw。
- Port 的公开构造不依赖万能 HolePie；open 导入并识别入口，borrow_raw 明确接收原始编号。
- port::Sender 导入时识别服务对端，只有发送操作；实际权限与资源存活仍由内核检查。
- Capability 只释放本地创建的能力，Loan 借用源并只撤销指定对端的派生能力；显式清理可返回错误，Drop 提供非阻塞兜底。
- port::Reply 拥有回信端、核对回复来源，并明确按 seal、release 收尾。Loader 使用 Sender/Reply/Loan，保持 Build→Claim 和统一预算。
- dock 保留裸地址 View 的已有约束，不增加跨撤销仍有效的安全 slice；Loader 的内核保护快照与服务端来源、角色验证继续保留。
- 所有驱动、服务、客户端和测具的原资源调用均已迁移，旧 runtime 完全退出；显式 raw 仍用于协议和硬件边界，不表示已获业务授权。
- 9 项资源宿主测试编译实际实现，用环境调用替身验证清理范围、顺序、错误和来源拒绝；真实内核行为由三个 QEMU 场景补齐。

本批最终验证：29 项调度、7 项 System 边界、10 项 Loader 兼容、2 项执行支持、9 项资源测试共 57 项均通过。
programs 全目标编译、resource 宿主编译、image 宿主装配编译及 QEMU accept、product、system-fault 均通过。
Cargo 元数据确认无 runtime 成员或依赖，execution/resource 互不依赖，schedule 无依赖。

### 6. 分离通用通信，迁公共接口

先完成纯 wire codec 分离及生成器接入，再迁通信和 RPC。
wire 不承担收发、对端识别、期限或业务角色；提供方 api 定义布局，ipc::rpc 使用同一 codec 处理单次请求回复。
RPC 明确管理回复配对、来源、回信端与统一期限，取消后隔离迟到回复；不自动重试有副作用的调用。
服务端可以入队并延迟回复，Build→Claim、资源角色与授权策略仍是提供方流程。

codec 基础已落地：

- wire 无 Cargo 依赖，承载 Field/Span、既有基本类型和 String 的编码、游标与数组辅助、Message。
- env 依赖 wire；TaskId/PieToken 的字段实现和寄存器编码仍归 env，旧字节路径仅重导出同一 trait 和函数。
- Mark 定义描述与碰撞检查迁到 env::marks，接口生成器和兼容入口直接引用它；固定角色字符串与裸值未变。
- Frame 默认使用 env 的兼容重导出，显式 `#[frame(codec = ::wire)]` 支持纯 wire 消费者，不要求引入 env。
- 长度求和与乘法溢出返回 None，避免 debug panic 或 release 回绕；普通有效帧布局保持不变。
- 独立 wire 测具没有 env 依赖，通过真实派生检查默认兼容路径与显式 codec 的字节一致、UTF-8、255/256 长度、游标溢出和数组计数边界。
- 5 项 wire、10 项 Loader、7 项 Identity、3 项 Mark、5 项生成宏测试共 30 项通过；programs 全目标、wire 与 image 宿主编译通过。
- 依赖元数据确认 wire 无 env 依赖、env 单向依赖 wire；QEMU accept、product、system-fault 均通过。

通信与 Loader RPC 已落地：

- `crates/ipc` 拥有 hand、rack、session、rpc 和时间预算；仅依赖 env、wire、resource，不依赖执行库或软件实现。
- 原 communication 正文全部迁出；protocol 只保留指向同一实现的兼容重导出，所有直接调用方使用 ipc 路径。
- rpc::Contract 固定 Request、Response、回程 Mark 和显式回程字段访问；纯 API 仍只声明字节布局，Loader 的传输绑定归客户端所在模块。
- request::Sender/Receiver 和 reply::Sender/Receiver 分别提供 send/receive；发送请求返回一次性的应答接收权，接收请求返回调用者、请求与已验证的一次性应答发送权。call 只组合请求发送与应答接收。
- 每次 send 独占新的 Reply，由资源层持有并撤销远端授权；成功、错误、超时或放弃接收均关闭。接收权捕获发送时的 Deadline，等待回复不能重置预算；下一次调用不可能消费前一次回信端中的旧回复，不添加新的线上关联字段。
- Loader 在镜像授权前创建一次 Deadline，Build 与 Claim 共享预算；镜像 Loan、服务状态及 task/team 核对仍显式归客户端。
- request::Receiver 在入口校验回程的授予者、资源创建者、调用者和角色，再移交原有队列及 launch；reply::Sender 绑定应答类型，send 消费发送权并释放。接收失败保留已解码的未信任请求，供领域层清理镜像授权；未验证的回程编号不释放。
- Claim 仍按 owner/task 验证；Build 保留异步准备和交付，回复失败仍触发原有实例停止与回收。
- resource::ReplyError 将内核接收错误与错误来源分开；普通 Denied 不再被误报成 WrongSource。
- RPC 对偶收拢的 43 项宿主检查通过：IPC 15、资源 10、Loader 11、System 边界 7。覆盖独立收发、入口拒绝、放弃接收、延迟预算和双方失败清理；programs 全目标编译通过。
- QEMU accept、product、system-fault 均通过；依赖检查确认 ipc 不依赖 protocol、execution 或软件 API。
- 兼容重导出将在删除 protocol 的第 7 批移除；其他服务客户端暂保留原有交互流程，后续逐个迁纯 API 与领域调用。

RPC 的对偶按交互步骤划分：

| 步骤 | 发送端 | 接收端 |
| --- | --- | --- |
| 请求 | `request::Sender<C>::send(deadline, build)` → `reply::Receiver<C::Response>` | `request::Receiver<C>::receive(buffer, within)` → `Incoming<C>` |
| 应答 | `reply::Sender<R>::send(self, response)` → 完成并释放 | `reply::Receiver<R>::receive(self)` → 已解码应答并关闭 |

请求端持有可重复使用的入口；每次请求产生独立、不可复制的一次性应答权。
接收请求成功才产生应答发送权，业务层可保留该权利并延迟应答。
两端共享 Encode、Decode、Send、Receive 等错误阶段，但本地端点所有权与远端发送能力仍分别收尾。

整合原 port 与 hand/rack/session；两端关系归 resource::port，类型化通信和交互归 ipc，依赖 wire 与 resource。
调试调用的原始契约留 env，格式化与输出循环按实际调用方归位，不迁入纯 schedule。
system-client 接入新传输；逐个迁 Identity、Operator、Control 的接口声明和客户端。
Identity 已作为第二例迁移：

- `system-api::identity` 拥有领域词汇、请求与应答 codec、错误码、17 个 Grant、回程 Mark 和四个线上限额；仅依赖现有 env、wire、mold。protocol 的对应模块只重导出同一实现。
- Grant 的动作码、名称、Mount、Wire 匹配和 Mark 来自提供方局部的一张声明表；保持原 1..17 动作码和 `identity-*` 记号。全局 Mark 碰撞检查继续覆盖该表。
- authority 的 principal、coalition、membership、binding 容量及创建配额归 `identity/core/limits`；模型与应答逻辑直接依赖提供方 API，不把存储策略放在线上契约里。
- 公共客户端保留领域与发现接口，改用固定 Request/Reply 契约的 RPC。每次调用重验入口 owner/Grant，共享一次 Deadline；实际回复来源与返回身份数据的 authority 分别验证。
- 服务端入口统一导入已验证的应答发送权，队列与 Current 持有该权利；回复或放弃均自动释放。Grant 与动作的匹配、权限判决、状态更新和 revision 仍归 authority 模型。
- 完整请求头但损坏动作载荷仍解码为 `None`，合法回程收到 `Bad`；不可信回程直接拒绝。保留一页收件缓冲，rpc 接收允许调用方提供字节切片，由 codec 判断合法帧上限，避免超长帧阻塞后续请求。
- 本批 64 项宿主检查通过：Identity codec 7、authority 模型 11、Identity 客户端与契约 9、IPC 16、Mark 3、Loader 11、System 边界 7。客户端测具使用真实源码与 codec，模拟发现、资源查询和 IPC 边界；真实 RPC 生命周期另由 IPC 测具验证。
- programs 全目标编译与 QEMU accept、product、system-fault 均通过。公共客户端实现与 RPC 契约绑定已在后续批次移入 system-client；protocol 的兼容导出将在第 7 批删除。

Operator 的提供方 API 与会话调用已迁移：

- `system-api::operator` 拥有请求／应答、bootstrap 提示、事件记录、Permit、错误码、8 个 Grant、Mark 和 Path/PathBuf；原 protocol 的 frame、grant、marks、common path/name 只重导出同一实现。
- 保持线上动作码、Grant 位次与记号原值；动作码 Land=1/Part=2 与 Grant Part=1/Land=2 是不同轴，不在提取时互换。EntryId 的 8 字节小端实现归 API，旧 Id trait 仅在 protocol 做兼容适配。
- 持久会话的请求不携带回程字段，继续使用已建立的 Session。`ipc::session::Contract` 固定 Req/Union，`Session::call` 从 talk 发请求、从 link 接收应答，两步共享一次 Deadline，不自动重发。
- `hand::Receiver::recv_from` 在解码前验证内核发送者；会话应答、服务端请求和 Watch 事件均使用该入口。SourceFail 区分来源错误与原 RecvFail，未校验来源的既有 recv 接口保持原样。
- 模型与服务端直接依赖提供方 API。Grant 判定、Control 修改权限、Permit 判决、能力转授、bootstrap 接线和事件序号／路径过滤仍归各自原模块；没有把订阅改为一次性 RPC。
- 一次性 RPC 的独立回程隔离不适用于持久 Session；会话报文仍无调用关联号。后续批次通过共享状态与失败关闭处理超时后迟到应答，保留每个请求恰好回复一次的服务端契约。
- Operator 专属宿主检查覆盖八条请求与各类应答的固定字节、Permit authority、Path 规范化、bootstrap／事件形状和真实权限判决；IPC 会话测具检查 talk/link 路由、共享预算、来源验证顺序、错误分类及大缓冲收件。
- 本批 61 项宿主检查通过：Operator API/判决 10、IPC 会话 7、一次性 RPC 16、Identity codec 7、Mark 3、Loader 11、System 边界 7；programs 全目标编译和 QEMU accept、product、system-fault 均通过。

Control 的提供方 API 与三种 RPC 已迁移：

- `system-api::control` 拥有生命周期请求与应答、State、错误码、五个 Grant、单表生成的八个 Mark、入口路径，以及 publication/account 类型与 codec。protocol 的对应数据模块只重导出同一实现。
- 普通生命周期、publication 和 account 分别绑定固定请求／应答契约，客户端使用独立回程端与共享 Deadline。名称引用入口复用 publication 契约，authority 与实际发送者仍分别验证。
- 普通请求保留完整头里的未知动作回程，服务端返回 Bad；account 解码保留头完整性标志，授权检查后再验证完整帧与账号名称，非法尾巴仍能返回 Bad。
- inbox 和生命周期 Request 持有一次性的 `reply::Sender<Said>`；排队失败返回原 Request，保留回复 Full/NotReady 的权利。完成仍在原 run/frame 回复阶段消费 Option，不提前回复，不改变回滚与实例操作顺序。实例重排队通过回复对象查询存活，不暴露裸回程 token。
- publication 的准入由已验证的回复权表示；不可信回程携带的来源能力仍通过原索引做清理，避免遗忘既有发布引用。客户端在 RPC send 成功后记录交付，接收失败不会撤销已经交付的能力；发送失败才撤销本次授予。
- publication 构造器把能力与许可作为一对参数，符合 System 三参数限制；原线上帧布局与固定状态码保持不变。公共客户端已在后续批次移入 system-client；业务策略继续留软件实现，随目录重组归位。
- 本批 53 项宿主检查通过：Control API 4、Control/publication 客户端与契约 4、IPC 17、Identity codec 7、Mark 3、Loader 11、System 边界 7。客户端测具模拟 IPC/资源边界并使用真实源码与 codec；异步生命周期队列未新增独立宿主夹具，由全目标编译、RPC 生命周期检查与真实系统场景共同验证。
- programs 全目标编译及 QEMU accept、product、system-fault 均通过。

公共客户端独立已落地：

- `crates/system-client` 的四个模块分别拥有 Loader、Identity、Operator、Control 客户端流程及 RPC/会话绑定；帧、状态码、Grant、Mark 和路径仍重用 system-api 的同一类型，无第二份 codec。
- 客户端不依赖 protocol 或程序实现，system-api 只依赖 env、wire、mold，不反向依赖客户端。客户端依赖 execution 仅用于 Operator 路径发现的退避等待；调试输出为私有实现，不成为公共使用接口。
- programs 的四域调用与服务端契约绑定全部改用 system-client 路径；Operator 模型直接使用纯 API 的 Selector。protocol 原客户端正文全部迁出，只保留对应兼容导出，供尚未迁移的软件接口使用。
- Identity/Control 宿主测具已直接编译新客户端源码；9 项 Identity、4 项 Control/publication、8 项 System 边界检查通过。新增边界检查约束纯 API 依赖集合及客户端与 protocol 的独立关系。
- 客户端提取保持字节格式、能力清理和预算；持久 Session 的失败隔离已在后续批次落实。
- programs 全目标编译、system-client RISC-V 检查与 QEMU accept、product、system-fault 均通过；依赖闭包检查确认 system-client 不含 protocol。
各域自行拥有错误与回复；保持原数值和布局，程序显式做域间转换。
Account 不再借 Loader 客户端类型表达自己的创建结果。

共享 injected authority 识别进入 System 客户端库，保留现有验证。
System 的身份安装、启动重试和授权策略留在实现。
拆 Hub bridge：公共调用归 Hub 客户端，System 接收授权归 launch。

Terminal、Hub、驱动和测具的资源调用以及旧 runtime 删除已在前批完成。
本批继续迁移通用通信和提供方客户端；测具的原始环境调用不扩大正式资源 API。

完成条件：System 的纯 API 不依赖 resource/execution/schedule/ipc/程序实现，可直接运行宿主编码测试；用户态 runtime 完全退出。

### 7. 消除中央 protocol 与手抄 Mark

把 Hub、Terminal 和驱动接口声明迁到各自提供方，客户端随提供方维护。
逐项处理原 common：路径、名称和身份等内容按真实拥有者与复用关系归位，不整体改名搬进 wire。
image 及其 #[path] 编译的 program.rs 直接依赖纯 API，引用生成的入口描述。
启动与部署关系仍在程序清单，只有通道/入口标识改为共享描述。
装配处检查跨 API 的 Mark；动态名称的使用与静态定义检查分开。

protocol 删除已落地：

- Hub、Terminal、Router 的纯 API 与公共客户端分别归提供方目录，形成 hub-api/client、terminal-api/client、router-api/client；System 四域继续使用 system-api/client。API 不依赖客户端，所有包均不依赖 protocol。
- 旧接口、客户端与握手实现保留字节格式和资源生命周期；DTO 直接使用 wire::Message、环境类型使用 env，宏直接依赖 mold。诊断输出归 programs::debug，命名树服务前缀归提供方命名 API。
- 通道 Mark 和 Grant 从提供方单表声明生成。装配层的 unit/interfaces 只组合各 API 的 REGISTRY，env::marks::conflict_between 检查组内及跨提供方冲突，不手抄所有通道和 Grant 的 GROUPS 清单。
- 删除 crates/protocol 目录、工作区成员、依赖、正文和兼容导出。Cargo metadata 确认没有 protocol 包或依赖；程序代码没有旧导入。
- 原 Loader、Identity、Mark 宿主套件迁到 programs/tests/loader-api、identity-api、interface-marks；既有 Terminal/Login 测具直接使用纯 API，保留原测试体。
- 本批 48 项相关宿主检查通过：Loader 11、Identity 7、Mark 4、Hub API 3、Terminal/Router API 4、System 边界 8、Terminal 行为 9、Login 2。全程序编译及 QEMU accept、product、system-fault 均通过。

剩余装配工作：image 与 program.rs 的入口声明继续按提供方描述归位，避免字符串元数据与接口声明重复。

持久 Session 的失败隔离已落地：

- Session 私有化端点字段，Clone 共享 Ready/Busy/Closed 状态；Operator 借用面使用共享别名，不再复制三个裸字段。
- 原始导入验证本地回信孔仍存活且由本任务创建。调用前检查两端存活、回执队列为空；并发别名返回 Busy，不发第二个请求。
- 发送前错误恢复 Ready；请求已入队后，超时、错误来源、非法回复或展开中断都先标 Closed，再封印自有回信孔。即使 seal 失败，本地别名仍拒绝后续调用，不读取迟到回复。
- 已排队的重复／旧回复在新请求发送前触发关闭。所有检查、发送和接收共用一次 Deadline；不重发、不自动重建会话，不释放仍被别名借用的端点。
- 没有新增线上关联字段，服务端仍须对每个请求恰好回复一次；任意延迟的重复成功回执无法在该字节格式下与新应答区分。原始导入用于新的能力边界，普通别名必须 Clone 共享状态。
- 15 项宿主检查及 QEMU accept、product、system-fault 均通过，覆盖别名、迟到／已排队重复回执、并发拒绝、发送失败复用、seal 失败及展开清理。

### 8. 收拢 System 的状态、计划和目录

Control 收起任务表、实例和 Loader 字段，提供必要的操作、观察和 hook 组合入口。
launch 拥有准备与交付请求，不复制任务状态；app 注入必要子计划，打断循环依赖。
组件自己登记私有资源、构造局部计划；app 只组合稳定流程段，继续使用当前 Registry。
拆 run/frame 的程序退出策略、任务推进和请求回复，分别归 app、Control 和对应 IPC。
publication 收拢发布、names 与 runtime namespace；启动物料仍归 Control。
account 保留当前行为，把帐号/镜像选择与运行机制分开。
按上述归属迁移目录，消除 core/serve/run；公共软件客户端与私有管理通道不再同名混淆。

完成条件：顶层安装和计划不引用私有 Request/Decision/Dispatch；组件内部修改不要求同步修改 app。

### 9. 收缩公开面并完成验收

叶子模块私有，组件根只暴露必要入口。共享 Machine/face 支持不再由 System 私有目录提供。
测具通过窄的 fixture 接口观察状态、提交操作和注入故障，不直接改组件字段。
删除全部旧路径、迁移兼容层和临时重复定义。

最终检查：

- 纯 API 和生成器的宿主测试，包括旧格式兼容、非法长度/动作与 Grant 映射。
- image 宿主构建，验证清单能消费纯 API 并检查装配 Mark。
- programs 全目标编译，System 调度与模块边界测试。
- QEMU accept、product、system-fault 场景。
- 依赖检查：纯 API 不依赖执行库，ipc 不依赖软件实现，普通调用方不进入 System 私有目录。
- schedule 不依赖环境或执行库；runtime 与 protocol 均无工作区成员、用户态依赖或旧导入残留。

各批次验证通过后提交，再继续下一批。临时重导出必须有明确删除阶段。
第 1 至 5 批已落地，runtime 迁移完成；下一份实现分离通用通信并迁移其余提供方接口。
第 6 批与第 7 批接口归位、protocol 删除已完成；接下来收拢 System 状态与目录，并处理前述装配元数据和持久 Session 生命周期。

## 迁移后的命名口径

- execution::room::park 接收 Duration，保持向上取整；env::room::park 保留单次调用的毫秒参数，不另留 sleep 别名。
- 域内任务创建和结果回收位于 unit::task，以 spawn/try_spawn 创建并放行，Join::join 回收结果；unit::spawn 仍创建 Held 任务。
- memory 的私有分配后端归 allocator，保留 Heap 等实际堆结构名称。
- 原始编号导入统一 from_raw，和 unseal 创建区分；raw::Hole 替代旧 HolePie，不表示已验证的能力。
- 私有 capability 模块中的 Capability 管理本地释放，Loan 管理派生撤销；提供方授权分类继续叫 Grant。
- 保留 dock、port、bell、pile 和线上角色、操作码、Mark、路径及符号，不用兼容别名维持两套词汇。

## 当前可用的验证命令

以下命令对应当前仓库；接口与测试迁移时同步更新路径。

```sh
cargo check -p programs --all-targets --offline
cargo check -p system-api --target x86_64-unknown-linux-gnu --offline
cargo check -p schedule --target x86_64-unknown-linux-gnu --offline
cargo check -p resource --target x86_64-unknown-linux-gnu --offline
cargo check -p wire --target x86_64-unknown-linux-gnu --offline
cargo check -p ipc --offline
cargo check --manifest-path crates/image/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test -p mold --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/schedule/src/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/execution/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/resource/tests/capability/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/wire/tests/host/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/ipc/tests/host/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/interface-marks/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/identity-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/loader-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/hub-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/device-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/terminal/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/login/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/system-shape/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/identity-rpc/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/operator-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-rpc/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/ipc/tests/session-exchange/Cargo.toml --target x86_64-unknown-linux-gnu --offline
sh programs/src/system/identity/core/test-host.sh
nu scripts/qtest.nu --package kernel --scene accept
nu scripts/qtest.nu --package kernel --scene product
nu scripts/qtest.nu --package kernel --scene system-fault
```
