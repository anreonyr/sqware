# System 与用户态基础重构

本轮实现已落地。中央 protocol、总括 runtime 及 System 的 core/serve/run 分类已退出；保留内核 ABI、线上报文、资源模型、任务生命周期和 programs 的多 binary 打包方式。

## 当前目录与状态所有权

```text
programs/src/
├── support/                 # 共享 face、Machine、启动时间常量
├── system/
│   ├── api/                 # 独立 system-api：提供方契约
│   ├── client/              # 独立 system-client：公共语义客户端
│   ├── app/                 # 引导、配置、程序阶段、组合计划、总等待器
│   ├── control/
│   │   ├── unit/            # 任务登记、镜像缓存、启动物料、就绪与收割
│   │   ├── instance/        # 实例命令、认领、超时、Prepare/Retire 扩展
│   │   ├── lifecycle/       # Mint/Embark/Debark/Ruin、队列、游标、补偿
│   │   ├── service/         # Control 请求接入、解码、回复
│   │   └── identity.rs      # 任务身份安装账、继承、绑定与回收
│   ├── identity/
│   │   ├── book/            # 主体、联盟、成员关系、身份快照
│   │   ├── service/         # 权威接口接入和服务计划
│   │   └── revision.rs      # 身份修订信号
│   ├── operator/
│   │   ├── tree/            # 命名树、条目与授权判据
│   │   ├── service/         # 请求、会话、订阅、事件交付
│   │   └── management.rs    # System 私有命名管理通道
│   ├── loader/              # 映像、缓存、映射、构建、任务与私有服务
│   ├── publication/         # 发布账、名称注册、运行时命名空间、撤销
│   ├── launch/              # 身份准备、授权接入、命名空间准备、就绪交付
│   └── account/             # 帐号会话机制；配置由 app 注入
└── service/hub/、user/terminal/、driver/router/
    ├── api/                 # 软件自己的接口声明和生成元数据
    └── client/              # 软件自己的公共使用库

```

System 只对独立程序开放 app::run；实现组件限制在 crate 内。普通驱动、服务和用户程序使用公共 API/客户端及 support，不导入 System 私有目录。压测与 accept 的独立二进制为薄入口，具体测具留在 harness。

Control 唯一持有任务表、实例表、待放行任务和 Loader 缓存，字段仅 Control 子树可见。创建依次执行容量检查、构建、登记；外部通过命令与只读观察访问状态。受管任务的启动进度归 Control；app 保留程序阶段、空闲、整体退出策略与程序层 Fault。

IdentityBook 只拥有权威身份模型。Control 的 Roster 拥有安装账，普通调用方的受信 authority 发现归 system-client。Operator management 是私有管理入口，不承担公共客户端职责。

publication 拥有发布、名称、连接与运行时命名空间的账。launch 只持有等待交付的请求，通过 Control 查询实例结果，不复制实例状态。account 根据注入的帐号与镜像配置处理可信 Login 请求。

Hub 的公共 activation 调用归 hub-client，接收、验证与授权安装归 launch。Control 接受 activation 和实例 Prepare/Retire 子计划，由 app 注入 launch 流程；Control 不依赖 launch、publication 或 account 的实现。

## 安装、计划和等待边界

组件自行登记私有资源并构造局部计划。app/install 只调用组件安装入口，再登记 app 自己的阶段、配置和等待资源；app/schedule 只组合稳定流程段，不引用 Request、Decision、CurrentTip、Inbox 或私有 Dispatch，不排序组件内部请求管线。

顺序保留为：

1. 引导内部服务、安装身份与公共入口、开始静态任务。
2. 发布维护、健康检查、Control 接收、Account 接收、Loader 构建。
3. Control 命令推进、发布维护、实例回收、回复、实例扩展与交付。
4. 启动进度与运行阶段、退出策略、等待兴趣汇总与等待。
5. 关闭 Loader 入口和缓存、通知内部服务退出、等待结束。

Mint/Embark 失败保留原失败，必要时转入 Ruin、刷新补偿期限并重置游标；携带任务的 Ruin 失败终止程序流程。实例准备失败转入 Stopping，清理 Pending 或失败时继续等待／重试，清理完成才报告 Dead。Ruin 请求也在回收完成后才回复。

总 Pile 与等待订阅归 app::wait::Waiting，Control 仅拥有自己的请求入口。组件交出入口和等待预算，app 汇总兴趣。关闭 Loader 时通过 Waiting 的窄 detach 入口撤掉订阅。

## 基础库分层

| 库 | 职责 | 依赖边界 |
| --- | --- | --- |
| wire | Field、Span、Message、字节游标和基本字段编码 | 无依赖、no_std |
| env | 环境调用号、寄存器布局、原始句柄、失败码、权限、启动格式、Mark 值与碰撞检查 | wire；不拥有重试、资源收尾、会话、发现或业务授权 |
| resource | dock、port、bell、pile 和私有能力生命周期 | env；不依赖软件接口 |
| execution | room、unit、memory、lock、boot；任务闭包结果、TLS、分配与退场 | env；不拥有 System 任务表 |
| schedule | 资源注入、依赖图、顺序和可恢复执行 | 无依赖；不依赖 env 或 execution |
| ipc | 通用消息、对偶 request/reply、持久 Session、期限和传输 | wire、resource、env；不收集软件客户端 |
| 提供方 API | 公共数据、接口码、帧、角色与授权元数据 | env、wire、编译期 mold；允许真实的 API 数据依赖 |
| 提供方 client | 语义操作、单次调用和多步骤流程 | 对应 API 与通用基础库；不依赖程序实现 |
| image | 程序清单、部署与装配检查 | 各提供方纯 API；不依赖执行库或客户端 |

保留 dock、port、bell、pile；execution 子模块使用 room、unit、memory、lock、boot。room::park 接收 Duration 并向上取整，env::room::park 保留一次调用的毫秒参数。原始导入叫 from_raw，与 unseal 创建区分；Hole/Pole 等原始种类留在 ABI 与 raw 边界。

env 的字节 codec 路径重导出 wire 的同一实现，寄存器 Wire/FromPair 保持独立。没有第二份 Field/Span 或 env↔wire 循环依赖。

## 接口、Mark 与装配生成

mold 的 Frame/WireCodes 生成固定帧及错误码编解码；interface 生成请求分派或独立的提供方元数据。Loader 使用完整 interface 声明；其余提供方复用 metadata 模式生成 Channel、Grant、声明登记和 PUBLICATIONS。contract 生成 wire::Contract、Request/Response 配对及明示的 BACK/back 路由。

Grant 的线上码显式固定，数组下标与线上码分开。Mark 从稳定接口 ID/key 生成；旧接口明确保留 legacy 字符串。接口角色与 Grant 各只有一份权威声明。复杂变长数据、非法动作后的路由保留、授权选择和业务字段校验继续显式编码；生成器不猜 PieToken 的所有权，也不代替凭据验证。

公共客户端只暴露语义操作，软件专用 transport glue 由通用 IPC 及生成的 Contract/Message 接合。Build→Claim、身份安装、发布与补偿仍是显式流程；生成分派把请求交给队列，不同步执行完整任务创建。

Hub、Terminal、Router 的程序发布项读取生成的 PUBLICATIONS；RTC 复用 Router 的 ENTRY_MARK。UART 的 Mark::NONE 是无标记设备入口。清单继续显式拥有部署、启动顺序、权限与资源需求，不手抄接口字符串／Mark。

unit/interfaces 组合七个提供方 REGISTRY，各 API 检查组内碰撞，装配检查组内和跨提供方碰撞。别名引用原定义，定义登记一次；多个实例使用同一 READY 通道合法。image 直接消费纯 API 描述。

## 通信与资源生命周期

request/reply 是成对的通用端点，绑定同一个 wire::Contract。接收入口校验 native 来源和回信能力，回复权由类型持有并一次消费。每次 RPC 创建新的回信端，派生能力按 revoke→seal→release 清理。

Capability 负责本地 release；Loan 负责指定 peer 的派生撤销；Reply 校验归属并 seal→release。seal 不用作所有资源的通用析构。授权失败不形成撤销责任，显式清理失败不自动重复执行。

持久 Session 的 Clone 共享 Ready/Busy/Closed。发送前失败恢复 Ready；请求已入队后的超时、非法来源／回复或展开中断先标 Closed，再封印本地回信孔。并发别名不发第二个请求，已排队旧回复关闭会话；不重发、不自动重建。

线上格式没有关联字段，服务端须对每个请求恰好回复一次；任意延迟的重复成功回执不能与新回执区分。这是保留既有字节格式的边界，不宣称已消除。

Loader 校验映像能力来源、权限、offset 和长度，构建消费受控快照。客户端的资源准备、Build、Claim 共用一个 Deadline，失败清理明确；实例认领超时、对端退出、回复失败和准备失败交给 Control 回收。

## 保留的 schedule 语义

- sequence 按注册顺序推进；局部图用 new/before，同阶段无依赖节点按注册顺序执行，名字不参与排序。
- Pending／失败保留 Cursor，已完成步骤不重复；每个 invocation 独立持有 Cursor，更换计划或新一轮显式 reset。
- prepare 缓存资源位置，执行核对类型；登记顺序变化后重新绑定。
- Dispatch begin 设置预算，select 提交 invocation，take_result 消费 invocation 与结果；skip 占预算、stop 提前结束。
- finish 必须消费最后一次结果；finish 自身 Pending 时恢复 finish，不重复选取或推进子计划。

## 固定的 Loader 兼容基线

黄金期望位于 programs/tests/loader-api，不通过被测生成器重算期望。

| 项目 | 固定值 |
| --- | --- |
| Build / Claim 操作码 | 1 / 2；共用 Build Grant，授权码 1 |
| 请求长度 | Build：42 + 8 × 参数数，最多 554 字节；Claim：17 字节 |
| 回复 | status:u8、team:u64 LE、task:u64 LE，共 17 字节 |
| 状态 | 成功 0；Unknown 1、BadImage 2、Full 3、NotReady 4、Bad 5、Denied 6 |
| BACK | loader-back：0xc1bc7bad2c8f22ca |
| IMAGE | loader-image：0x280b1d49733b5ae2 |
| Build 入口 | loader-entry-build：0xa1c2ab202871949a |
| 限制 | 参数最多 64，映像最多 16 MiB，Claim 3000 ms；svc/sys/loader |

黄金套件同时覆盖截断、尾部、非法动作、参数数 0/64/65 和失败回复零 task/team；Identity、Operator、Control、Hub、Terminal／Router 的现有固定字节和 Mark 期望也保留。

## 验收与复现

宿主套件检查编码兼容、生成拒绝条件、Grant/Mark、语义客户端、RPC 来源与所有权、Session 失败隔离、资源析构、schedule、内存／启动参数、Identity 模型与架构边界。Loader client 新套件直接复用生产流程，检查共用期限、发送／接收／服务拒绝、Claim 不匹配和 Loan 清理。

真实 system-fault Loader probe 验证 Build/Claim、认领超时、任务退场与失败回收；检查远端映像 loan 已由服务端释放，以及 cache 清空后条目归零、内核资源表减少。accept、product、system-fault 共同覆盖正常装配和故障生命周期。静态模型测试不替代这些内核资源验证。

```sh
cargo check -p programs --all-targets --offline
cargo check -p system-api --target x86_64-unknown-linux-gnu --offline
cargo check -p schedule --target x86_64-unknown-linux-gnu --offline
cargo check -p resource --target x86_64-unknown-linux-gnu --offline
cargo check -p wire --target x86_64-unknown-linux-gnu --offline
cargo check -p execution --target x86_64-unknown-linux-gnu --offline
cargo check -p ipc --offline
cargo check --manifest-path crates/image/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo metadata --format-version 1 --offline
cargo test -p mold --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/schedule/src/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/execution/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/resource/tests/capability/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/wire/tests/host/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/ipc/tests/host/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/ipc/tests/session-exchange/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/ipc/tests/session-establish/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/interface-marks/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/publication-admission/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/identity-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/loader-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/hub-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/device-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/router-handoff/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/terminal/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/login/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/system-shape/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/identity-rpc/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/operator-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/operator-handoff/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-rpc/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-instance/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/loader-client/Cargo.toml --target x86_64-unknown-linux-gnu --offline
sh programs/src/system/identity/book/test-host.sh
nu scripts/qtest.nu --package kernel --scene accept
nu scripts/qtest.nu --package kernel --scene product
nu scripts/qtest.nu --package kernel --scene system-fault
```

Cargo 依赖图确认没有用户态 protocol/runtime 包；wire、schedule 无依赖，纯 API 无执行库依赖，ipc 无软件实现依赖。内核内部 runtime 不在这次用户态迁移范围内。


## 下一轮：Mark 的职责与发现规则重构计划

状态：第1至4阶段已落地，第5／6阶段尚未完成。下面同时保留迁移约束与各阶段完成条件。

### 目标与范围

服务实例通过运行时名称和能力发布；发布资格由命名空间与授权策略判断。接口协议独立定义，客户端按其协议操作已获得的能力。Mark 只在必要的能力交付、启动发现和外来能力导入边界标记角色。

不要求每个服务实例生成 Mark，不要求新实例进入编译期登记表，不根据调用方自行选择的 Mark 授权。保留内核的 Mark ABI 与 capability badge 语义：Mark 附在每份能力引用上，转授时 NONE 表示继承，非 NONE 可给派生能力重新标记。

本轮不修改 Kernel ABI、资源模型、任务生命周期或 schedule 语义。既有接口的 Mark 裸值、操作码和失败码先固定；接入握手若确实需要增加字段，明确修改用户态协议与全部调用方，不把布局变化伪装成透明重构。

### 职责划分

| 内容 | 拥有者 | 确定时机 | 判断依据 |
| --- | --- | --- | --- |
| 服务实例名称与路径 | publication / Operator 命名树 | 运行时 | 命名空间权利、名称合法性、冲突和生命周期 |
| 入口能力 | resource 与提供方 | 创建／交付时 | 类型、存活、来源、所有者、实际权限 |
| 请求与回复协议 | 提供方 API | 接口定义时 | codec、操作码、版本与边界校验 |
| 发布及调用授权 | publication / 对应服务 | 每次准入或受限会话建立时 | 可信来源、身份、Permit、可信方签发的允许范围 |
| Mark 角色 | 提供方 API 或私有交付流程 | 必要识别边界 | 明示角色与来源；不承担服务身份或授权 |

两个实例可以实现同一个接口并使用相同的角色声明；明确交付的实例入口也可以是 Mark::NONE。类型封装提供调用形式与本地限制，不能把调用方自己构造的 Rust 类型或标签当成服务端授权证据。

### 1. 固定实际基线并建立用途清单

从现有定义、创建、转授、发现和导入调用反向整理每个生产 Mark 的用途，列出创建者、预期持有者、交付途径、来源检查和实例范围。区分：

- 启动时只能通过能力目录发现的角色。
- 已通过参数／报文交付 token 或 seed 的角色。
- Operator 当前用于区分操作面的请求标签。
- 测具的故意错误标签和普通无标记能力。

重点文件：env/wire/handle.rs、mold/interface.rs、API 声明、resource/port.rs、ipc/session/establish.rs、Operator service/claim.rs、publication/policy.rs 和 unit/mod.rs。

保留现有固定数值及字节黄金期望。补基线证明：转授可重新标记但不能扩大实际权限；相同 owner/Mark 的多个实例确实可能存在；Mark::NONE 能力可被合法持有和使用；Operator 的请求 Mark 由调用方选择而不是可信授权者签发。

完成条件：每个生产用途都有明确来源与范围，不新增逐个服务实例的全局登记或新的中央管理库。

### 第一阶段用途清单与冻结基线

七个提供方共 63 个登记角色：29 个 Channel、34 个 Grant。`programs/tests/interface-marks/baseline.rs` 固定完整 legacy 名称及 64 位数值，测试直接比较实际 REGISTRY；不通过当前生成器或被测 Mark::of 重算期望。新增或删除角色时必须明确其兼容处理，不能静默重生成黄金数据。

| 角色集合 | 创建／预期持有者与交付 | 第1阶段识别和来源验证基线 | 实例范围与迁移决定 |
| --- | --- | --- | --- |
| Loader Build 入口、IMAGE、BACK（3） | 入口由 System 创建；调用方授出映像副本并新建回信端；Control/Loader 收明确 seed | 客户端验证入口角色；服务端核映像 vestor/from、类型／范围；回复核来源与回信能力 | Build 入口按预期 authority 发现；映像与回信可多实例，显式交付优先 |
| Identity BACK + 17 个 Grant（18） | 权威任务创建分面入口；调用方每次创建回信端；入口经安装／命名交付，回信随报文 | 注入 authority 验 Sire/vestor/owner，入口核 Grant；服务端继续按实际调用者及模型授权 | 分面入口需在确定 authority 下定位；每次回信不要求全局唯一 |
| Control ASK、BACK、LINK、ACCOUNT_ENTRY/BACK、PUBLICATION_ENTRY/BACK、IDENTITY_REF + 5 个 Grant（13） | System 创建服务入口；调用方授出回信；对象引用由可信发布／名称机制生成 | 入口核来源与角色；请求端核 native 发件人，实例命令核 owner；Object/Permit 核 authority | ASK 对应实例接入；BACK 可多实例；LINK 仅保留兼容定义／断言，阶段5复核删除；其他入口按实际授权方式交付 |
| Operator ASK/LINK、8 个操作面标签（10） | 普通客户端自己选择问话标签、创建会话两端并交给持树者 | 目前 server 用 owner/Mark 扫表后推断操作面；实际写权限另查 Control 调用者与 Permit | 当前会话识别限制一任务／角色一对，且首末枚规则不一致；阶段3/4改明确会话交付，标签不能视为可信 Grant |
| Operator TIP/TIP_BACK/WATCH（3） | 管理端创建 TIP；请求方创建 TIP_BACK；每个订阅方创建事件端并传 seed | 管理请求验证来源及回信 vestor/owner/Mark；WATCH 以报文中明确能力建立订阅 | TIP 是私有管理单例；回信和订阅端可多实例，不能全局排重 |
| Hub LIST/CLAIM/BOND、BACK/ALIVE/ACTIVATE_ENTRY/ACTIVATE_BACK（7） | Hub 创建公共入口；设备创建存活通知；System launch 创建私有 activation；回信由调用方建 | 公共客户端核入口／来源；activation核真实 live Hub 身份及任务关系；设备与组织准入另查 Identity | 公共服务名与角色解绑；设备通知及回信是明确交付能力；私有入口只在启动关系内发现 |
| Terminal ENTRY/AUTHORITY/BACK/INPUT/OUTPUT/CONTROL（6） | Terminal 提供 attach 入口，建立每个 attachment 的流端点；可信 capability 作为AUTHORITY交付 | attach/流握手检查对端；authority查 vestor/from；流端点在每个 attachment 关系内认领 | 同一角色允许多个 attachment；阶段3优先交付明确句柄，不能按全任务首末枚选取 |
| Router ENTRY/LINE_MARK/LINE_BACK（3） | Router 建服务入口；客户端/驱动创建每条 line 关系并授出能力 | 核 host/creator及对应角色；绑定消息／pair维护关系 | ENTRY 供启动／命名发现；lane/back是连接角色，允许独立 line 关系 |
| 通用 Port 的 `back`（未进入API REGISTRY） | Port::open 为已知对端新建回信端，seed已显式交付 | Port 保存自己的 reply，native权限和对端决定收发 | 每次端口一枚，不应参与全局发现；阶段5确认能否直接使用NONE |
| Hub `hub`/`hub-ready` 与共享 `ready`（启动角色） | Control按Setup创建load端点；服务按启动约定创建ready并转授Control | Control按实际已spawn task/role等待，副本来源随native事实核对 | 同一READY可由不同task使用；是启动协调标签，不是服务发布条件 |
| RTC私有 `rtc-back`（未进入API REGISTRY） | RTC请求方每次创建回信能力并传入报文 | 由具体请求的能力与来源维护回复关系 | 纳入阶段5角色复核，不新增运行时中心；明确交付能力无需静态实例登记 |
| Mark::NONE 与测具自有标签 | NONE表示无角色，转授参数中表示继承；测具标签仅隔离测试 | NONE能力仍由native来源／权限控制；错误标签不代替真实授权负证据 | 不进入生产角色唯一性表；可有任意多独立能力实例 |

第1阶段记录的发现基线：Operator claim 取第一枚并最多告警，通用 establish::find 取最后一枚；两者都不是唯一性验证。kernel允许多个相同 owner/Mark 实例，因此阶段3须显式区分单实例发现与多实例交付，不能靠目录顺序解决。

真实目标探针 `harness/probe/marks.rs` 验证重新标记不修改源引用、NONE继承、重标记不扩大FETCH-only权限、同owner/Mark多实例、NONE孔收发，以及正常seal/release后资源表回到基线；第1阶段还调用生产 `granted_berth` 验证请求标签确实由客户端选择；第4阶段删除该封装后，探针保留原生能力边界断言。纯API宿主黄金测试与真实内核探针各自验证对应边界。

### 2. 服务发布解除 Mark 准入绑定

调整 publication::policy::service，删除普通服务入口的 Mark 匹配条件。发布准入改为校验发布者获准的 scope/group/path 范围、合法名称、入口来源／权限／存活、访问策略和现有挂载冲突。

将 unit::PublishEntry 中入口名称与 Mark 的捆绑拆开。产品配置可以保留固定系统入口与保留路径，其他获准命名空间允许运行时命名的实例。权限按边界限定，不能把动态发布扩大成任意写入整棵树。

mold 的 PUBLICATIONS 不再以 env::marks::Definition 作为发布权利的表示。名称提示或默认部署描述与角色登记分别生成／维护，拥有接口描述不等于获准发布。

本阶段保留现有 publication 报文字段和 Operator 挂载格式，先改变准入依据，避免同时升级传输。

验收：

- 同一允许命名空间可注册两个不同运行时名称的服务实例，无需添加静态 Mark／入口声明。
- Mark::NONE 的合法入口可发布并通过路径获得。
- 正确 Mark 但无命名空间权利、越界路径、错误来源或权限不足仍被拒绝。
- 保留系统路径不能被普通发布者覆盖；重名、撤销、重新发布和事件交付保持既有规则。
- 动态能力的运行时有效性仍需验证，不以移除 Mark 检查为由跳过 native 事实检查。

第1／2阶段实现记录：63个既有角色的黄金数值保持不变；Mark真实探针固定原生继承／重标记／权限边界。PUBLICATIONS现为固定名称数组，unit::PublishEntry只表达名称提示，既有产品固定名策略不扩张；运行时准入归publication私有admission模块，unit保持纯部署元数据，image不引入服务实现依赖。Namespace仅开放声明的scope/group/base-path；普通发布不读取入口Mark，native来源、owner、存活、访问策略与挂载冲突校验保留。Devices以Control登记的可信Hub、MemberOf Permit、设备白名单与实际联盟资格准入。

真实accept probe已发布并取回两个未列入静态entries的NONE入口，验证错误group、越界名称和仅有已知API Mark而无namespace权利均被拒绝；撤销一个实例不影响另一实例。system-fault的Mark基线探针验证无权限扩张、重复角色实例和正常释放。product保留原有Hub/Terminal/UART/RTC路径与名称行为。第3阶段的发现规则变化见下文；第4阶段尚未移除Operator操作面标签。

### 3. 显式交付优先，统一扫描发现的歧义结果

盘点并迁移已有 token/seed 却仍扫描能力目录的路径。启动参数或握手明确交付入口、请求端和回信端，接收端在导入边界验证，建立后保存连接上下文。

保留必要的 bootstrap 查找，但不继续用首枚／末枚表达选择策略。为单实例查找引入明确结果：Missing、Unique、Ambiguous；等待只对 Missing 重试，Ambiguous 立即失败，不释放不属于本次导入的能力。

先迁移通用 ipc/session/establish 与 Operator claim 的实际调用方，再删除旧的首枚／末枚查找入口。测试期间可以分批适配，最终不保留语义不同的并行查找 API。

唯一性以明确的发现范围判断。多个 RPC 回信端、多订阅和多服务实例不能因相同角色 Mark 被全局判为非法；这类连接必须依靠明确交付和会话上下文区分。

验收：目录顺序不改变发现结果；多个匹配不能被任意选中；已有显式 seed 的流程不重新扫表；重复／错误来源、对端退出、导入失败与正常多会话均有测试。

第3阶段实现记录：`establish::find/claim` 统一返回唯一 token 或 `Missing/Ambiguous`。发现使用内核 inspect 的 owner/Mark 与存活事实，支持 owner=0 的普通能力；Endpoint 的通信绑定另外使用 reserve 验证孔类型。仅缺失等待，歧义立即失败；已有 tx 只检查原绑定，退出后不自动扫描替代能力。构造失败只撤销／释放本次创建的能力，不能清理扫描中发现的其他实例。

Operator 的私有 `Tip::Guest` 仍使用 opcode=2，但由原9字节改为17字节，增加接收方表中的明确 reply token；旧9字节被拒绝。管理端转授后携带 seed，服务端检查存活、vestor=Control、owner=客人和 LINK 角色后保存，删除按角色查找 reply 的延期重试。Bootstrap 请求仍要求在发现范围内唯一，已接入的请求只核验原绑定。第3阶段提交时 Desk 仍按 task 管理客人、请求 Mark 仍区分操作面；这两项已由第4阶段替代。

Router 新增 opcode=2 的 `OccupyLane`：21字节请求显式交付 line、lane seed、back seed，9字节应答交付状态和反向 lane seed。发起端使用仅创建并转授的 lend，不预扫描对端。双方验证实际应答者、能力来源、owner、存活与角色后保存明确 token，不再按 LINE_MARK／LINE_BACK 扫表。旧 opcode=1 的5字节 Occupy 仅保留固定 codec 黄金，生产服务不接受旧握手。普通操作和失败码、63个既有角色数值、Kernel ABI 与 schedule 语义保持不变；这些用户态握手的布局升级有独立字节黄金。

故障测具也显式交付 REF seed，保留同角色重复实例与错误 authority 的拒绝断言。新增 session-establish、operator-handoff、router-handoff 宿主套件覆盖目录顺序、歧义、失效绑定、明确实例、导入来源和失败清理；accept 的重复占线仍要求 TAKEN，未知线仍要求 UNKNOWN，并检查资源回到基线。本阶段相关宿主测试共130项通过；programs全目标与image宿主构建通过，QEMU accept、product、system-fault均通过。第4至6阶段保持待实施状态。

### 4. Operator 的操作选择与授权分离

Operator 请求由操作码分派，Mark 不再决定调用方获准执行的操作。将当前 Grant 概念按实际作用整理：操作枚举表达请求，实际权限由可信身份、入口／会话来源及 Permit 判断。

普通会话使用统一接入约定；移除每个操作单独打请求 Mark 的要求及 granted_berth 带来的授权误读。创建／销毁等已有 Control 专属操作仍检查可信调用者，不能在合并入口时放宽。

若产品确需只读或限定操作的可委托会话，由可信管理方或 Operator 签发对应入口／会话，并保存真实允许范围。调用方只能持有和使用，不能通过自行改 Mark、提交请求字段或构造客户端包装扩权。内核当前不把派生 badge 作为每条消息的发送者标签返回，因此不提出依赖新增 message badge ABI 的方案。

明确请求／回信能力由握手交付，双方保持对偶关系、统一期限、一次响应及失败隔离。调整 Face/Rein 等客户端表面与相应探针，保留树、订阅及事件语义。已有操作码和数据帧不因删除操作面标签而重排；若握手变更，单独固定其新旧边界。

验收：

- 自己选择或重新打标签不能获得写权限。
- 普通会话可以正常完成查询和订阅，多会话相互独立。
- Control 专属操作仍拒绝非 Control 调用者。
- 限定操作委托如存在，调用方改变标签或包装仍不能绕过服务端允许范围。
- 错误来源、迟到／重复回复、忙碌别名、对端退出及订阅者退场继续受现有客户端／Session 检查保护。

第4阶段第一批实现：普通 Operator 会话统一使用 ASK，服务端不再从请求能力 Mark 解出 Grant，也不以其约束请求操作。SDK 删除 granted_berth、Rein 与调用者自选 Grant 字段；Face 的语义方法直接构造 Req，Wire 本身承担操作选择，不另建重复操作枚举。实际发件人仍由 Receiver::recv_from 校验；Part／Land／Trim 的 native 调用者必须是 Control。Find 的条目 Permit、Watch 的身份绑定与 mutation 的归属校验仍按原流程执行。

旧8个操作角色的数值／API登记及目录暂时保留兼容基线，已不参与请求接入或授权；其保留价值由第5阶段清理，不能把目录中的标签入口描述成可委托操作权限。本批没有签发限定操作委托，不存在由调用方标签或本地包装生成授权的路径。本批提交时同任务多会话仍受旧 Desk／Session bootstrap 限制；第二批已迁移显式会话接入，见下文。

本批验收：68项相关宿主测试、programs全目标编译，以及QEMU accept、product、system-fault均通过。真实探针验证统一会话的查询成功、非Control的Part／Land／Trim被拒绝；SDK与服务端守卫防止恢复Rein、granted_berth和按Mark选择操作面的路径。普通请求／应答布局、操作码、失败码及既有角色数值不变。

第4阶段第二批实现：每次 Session::open 都新建独立 LINK 和 ASK，不复用同任务／角色的请求端。Control 把每个经存活、owner、vestor、LINK角色核验的请求逐条处理，以16字节 bootstrap交付host与Control transport seed；客户端核验真实bootstrap发件人和transport事实，创建新ASK并发回其在Operator表中的8字节seed。Control的私有Guest仍使用opcode=2，但由17字节升级为25字节，明确携带who、reply、ask；9／17字节旧形都被拒绝。Operator核验reply来自Control、ask由who直接交付及其native事实，记录完整端点对、挂载后回1字节OK，客户端核验确认来自host后才返回。普通请求／应答布局、Mark裸值与Kernel ABI不变，握手升级有独立黄金与真实测具。

Desk以(who,reply,ask)区分会话，同task可有多条；共享任一端点的冲突不能替换既有项，精确重放不重复发ACK，拒绝消息不能污染已有回复端。删除pending发现、arm与Settling；outbox以reply索引。任一端失效只移除相应会话、监听和outbox，挂载／ACK失败回滚本次接入。Control也以实际LINK区分接入，单条bootstrap失败不终止整个System。

Session::open使用同一截止期限覆盖握手各步；克隆共享调用状态与所有权，最后一份释放本次创建的本地reply／talk，失败路径只回收自己创建的资源。Session::from_raw保持借用，不接管外来能力；give_at显式返回收件方seed，转授或窄化失败撤销本次交付并释放本地资源。必要的bootstrap单实例find仍保持Missing／Ambiguous规则，多会话不用它选端点。

本批验收：92项相关宿主测试、programs全目标编译，以及QEMU accept、product、system-fault均通过。真实accept在任务原会话存活时连续打开多条会话，验证别名不提前回收、独立查询、关闭一条后另一条继续工作、再次接入和最终资源表回到基线；直接导入的私有故障测具显式消费新增ACK。第4阶段完成，第5／6阶段待实施，旧8个操作角色的声明／目录仍属于下一阶段复核范围。

### 5. 收敛角色声明与 import 验证

将仍需要 Mark 的生产角色集中在所属 API 或明确的私有交付边界，通过生成常量使用；普通调用方不再现场计算协议字符串。

对明确传入能力且无需扫描识别的用途，删除无作用的 Mark，例如评估 Port 内部固定的 "back" 标签是否可用 NONE。保留或删除必须由实际导入验证与交付关系证明，不按字符串相似性合并不同角色。

保留编译期 REGISTRY 对已声明角色的碰撞检查。提供方声明或其局部 import 函数表达预期用途与来源检查，不再让 Definition 的名字／数值承担授权。无需为了服务实例扩大编译期根登记，也不另造接口类型或服务 ID 的全局运行时登记中心。

验收：每个剩余 Mark 都有实际识别用途；每个外来能力导入都有明确的 native 验证；旧 bare-string／重复 Mark 计算路径及歧义发现兼容层删除；角色重复实例的规则与公共客户端封装一致。

### 6. 收尾与完整验收

更新这份文档的当前架构和边界描述，将本节待实施状态改为实际结果。每个实现阶段验证通过后提交，不以尚未迁移的旧机制支持最终完成声明。

既有套件全部保留，尤其 API 黄金字节、interface-marks、Operator gate/judge、Control/Identity/Loader 客户端、资源 capability、Session 与 system-shape。新增行为验收针对动态服务、NONE 入口、命名空间拒绝、查找歧义、显式交付和不可自授操作权限。

程序全目标编译及 image 纯 API 构建通过，QEMU accept、product、system-fault 验证正常发布、动态实例、授权拒绝、故障回收和订阅退场。最终检查 Kernel ABI 未变、操作／失败码未重排、schedule 行为未变，System 使用方未重新依赖私有实现。

阶段顺序为 1 → 2 → 3 → 4 → 5 → 6。先解除发布绑定，再改变发现和 Operator 接入，避免把注册政策、握手和授权改动混成一批。第4阶段的权限选择以真实使用需求和可信签发路径决定，不把调用方本地限制冒充权限。
