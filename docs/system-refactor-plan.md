# System 与用户态基础重构

本轮重构及 Mark 迁移已完成。中央 protocol、总括 runtime 及 System 的 core/serve/run 分类已退出。原有内核调用编号、普通请求／应答的操作码与失败码、资源模型、任务生命周期及多 binary 打包方式保持不变；Mail 追加 Discard 调用，用户态接入握手的升级边界见下文。

## 当前目录与状态所有权

```text
programs/src/
├── support/                 # 共享 face、Machine、启动时间常量
├── system/
│   ├── api/                 # 独立 system-api：提供方契约
│   ├── client/              # 独立 system-client：公共语义客户端
│   ├── app/                 # 引导、配置、程序阶段、组合计划、总等待器
│   ├── control/
│   │   ├── unit/            # 任务登记、启动物料、就绪与收割
│   │   ├── instance/        # 实例命令、认领、超时、Prepare/Retire 扩展
│   │   ├── lifecycle/       # Mint/Embark/Debark/Ruin、队列、游标、补偿
│   │   ├── endpoint/        # 拥有私有入口登记；子模块适配请求／实例命令
│   │   └── identity.rs      # 任务身份安装账、继承、绑定与回收
│   ├── identity/
│   │   ├── book/            # 主体、联盟、成员关系、身份快照
│   │   ├── service/         # 权威接口接入和服务计划
│   │   └── revision.rs      # 身份修订信号
│   ├── operator/
│   │   ├── tree/            # 命名树、条目与授权判据
│   │   ├── session.rs       # 完整会话账；字段私有
│   │   ├── watch.rs         # 经导入核验的订阅状态与事件投递
│   │   ├── connection.rs    # Control侧非阻塞接入状态与期限
│   │   ├── runtime/         # 注册运行资源、组合计划与报文适配
│   │   └── management.rs    # System 私有命名管理通道
│   ├── loader/              # 映像、缓存、映射、构建、任务与私有服务
│   ├── publication/         # 发布账、名称注册、运行时命名空间、撤销
│   └── launch/              # 身份准备、授权接入、命名空间准备、就绪交付
├── service/account/         # 独立 Account ELF、部署声明、API 与客户端
└── service/hub/、user/terminal/、driver/router/
    ├── api/                 # 软件自己的接口声明和生成元数据
    └── client/              # 软件自己的公共使用库

```

System 只对独立程序开放 app::run；实现组件限制在 crate 内。普通驱动、服务和用户程序使用公共 API/客户端及 support，不导入 System 私有目录。压测与 accept 的独立二进制为薄入口，具体测具留在 harness。

Control 唯一持有任务表、实例表和待放行任务，字段仅 Control 子树可见；Loader 自己持有镜像缓存。创建依次执行容量检查、构建、登记；外部通过命令与只读观察访问状态。受管任务的启动进度归 Control；app 保留程序阶段、空闲、整体退出策略与程序层 Fault。

IdentityBook 只拥有权威身份模型。Control 的 Roster 拥有安装账，普通调用方的受信 authority 发现归 system-client。Operator management 是私有管理入口，不承担公共客户端职责。

publication 拥有发布、名称与运行时命名空间的账；Operator 自己拥有会话接入，publication 不再保存连接候选账。launch 统一持有排队请求和等待交付的请求；等待项只保存 task 与交付策略，通过 Control 查询实例结果，不复制实例状态。创建先预留交付队列容量，再由 Control 检查实例容量、调用 Loader 并立即登记，不存在独立 register 阶段或 registered 标记。Account 独立拥有帐号选择、用户身份与会话启动策略，通过公共 System 能力处理可信 Login 请求。

Hub 的公共 activation 调用归 hub-client，接收、验证与授权安装归 launch。Control 接受 activation 和实例 Prepare/Retire 子计划，由 app 注入 launch 流程；Control 不依赖 launch、publication 或 Account 的实现；System 装配也不导入 Account 实现、API 或消费者名。

## 安装、计划和等待边界

组件自行登记私有资源并构造局部计划。app/install 只调用组件安装入口，再登记 app 自己的阶段、配置和等待资源；app/schedule 只组合稳定流程段，不引用 Request、Decision、CurrentTip、Inbox 或私有 Dispatch，不排序组件内部请求管线。

顺序保留为：

1. 引导内部服务、安装身份与公共入口、开始静态任务。
2. 发布维护、健康检查、Control 接收、Launch 创建子计划（授权请求接入、Loader 接收、停止期拒绝、容量检查/构建/登记）。Account 由自己的任务接收请求。
3. Control 命令推进、发布维护、实例回收、回复、实例扩展与交付。
4. 启动进度与运行阶段、退出策略、等待兴趣汇总与等待。
5. 关闭 Loader 入口和缓存、通知内部服务退出、等待结束。

Mint/Embark 失败保留原失败，必要时转入 Ruin、刷新补偿期限并重置游标；携带任务的 Ruin 失败终止程序流程。实例准备失败转入 Stopping，清理 Pending 或失败时继续等待／重试，清理完成才报告 Dead。Ruin 请求也在回收完成后才回复。

整体退场先回收动态实例，再由 Control 根据原有 `after` 声明选择没有存活后继的静态服务停止。后继处于 Stopping 时也要等其回收完成；独立服务可以同时停止。scene 后继先于其余服务退场，不额外保存反向依赖图或退场状态。详见 [Launch 设计及退场修复](launch-design.md)。

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

mold 的 Frame/WireCodes 生成固定帧及错误码编解码；interface 生成请求分派或独立的提供方元数据。Loader 使用完整 interface 声明；其余提供方复用 metadata 模式生成所需的 Channel／Grant、声明登记和 PUBLICATIONS；Operator 只保留 Channel，不再声明操作 Grant。contract 生成 wire::Contract、Request/Response 配对及明示的 BACK/back 路由。

Grant 的线上码显式固定，数组下标与线上码分开。Mark 从稳定接口 ID/key 生成；旧接口明确保留 legacy 字符串。接口角色与 Grant 各只有一份权威声明。复杂变长数据、非法动作后的路由保留、授权选择和业务字段校验继续显式编码；生成器不猜 PieToken 的所有权，也不代替凭据验证。

公共客户端只暴露语义操作，软件专用 transport glue 由通用 IPC 及生成的 Contract/Message 接合。Build→Claim、身份安装、发布与补偿仍是显式流程；生成分派把请求交给队列，不同步执行完整任务创建。

Hub、Terminal、Router 的程序发布项读取生成的 PUBLICATIONS；RTC 复用 Router 的 ENTRY_MARK。UART 的 Mark::NONE 是无标记设备入口。清单继续显式拥有部署、启动顺序、权限与资源需求，不手抄接口字符串／Mark。

unit/interfaces 组合七个提供方 REGISTRY，各 API 检查组内碰撞，装配检查组内和跨提供方碰撞。别名引用原定义，定义登记一次；多个实例使用同一 READY 通道合法。image 直接消费纯 API 描述。

## 通信与资源生命周期

request/reply 是成对的通用端点，绑定同一个 wire::Contract。接收入口校验 native 来源和回信能力，回复权由类型持有并一次消费。每次 RPC 创建新的回信端，派生能力按 revoke→seal→release 清理。

Capability 负责本地 release；Loan 负责指定 peer 的派生撤销；Reply 校验归属并 seal→release。seal 不用作所有资源的通用析构。授权失败不形成撤销责任，显式清理失败不自动重复执行。

每次 Session::open 新建独立请求端与回复端，接入确认后才返回；Clone 共享 Ready/Busy/Closed 及所有权，最后一份释放本次创建的本地端点。Session::from_raw保持借用，并以unsafe契约要求对端点交换状态有独占控制；安全别名通过Clone建立。发送前失败恢复 Ready；请求已入队后的超时、非法来源／回复或展开中断先标 Closed，再封印本地回信孔。并发别名不发第二个请求，已排队旧回复关闭会话；不重发、不自动重建。

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

黄金套件同时覆盖截断、尾部、非法动作、参数数 0/64/65 和失败回复零 task/team；Identity、Operator、Control、Hub、Terminal／Router 的现有固定字节和 存续 Mark 期望也保留；退役角色单独固定为不得登记或重用的基线。

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
cargo test --manifest-path programs/tests/capability-import/Cargo.toml --target x86_64-unknown-linux-gnu --offline
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
cargo test --manifest-path programs/tests/operator-client/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/operator-watch/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/operator-handoff/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/operator-connection/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-api/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-rpc/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-instance/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/control-lifetime/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path programs/tests/loader-client/Cargo.toml --target x86_64-unknown-linux-gnu --offline
sh programs/src/system/identity/book/test-host.sh
nu scripts/qtest.nu --package kernel --scene accept
nu scripts/qtest.nu --package kernel --scene product
nu scripts/qtest.nu --package kernel --scene system-fault
```

Cargo 依赖图确认没有用户态 protocol/runtime 包；wire、schedule 无依赖，纯 API 无执行库依赖，ipc 无软件实现依赖。内核内部 runtime 不在这次用户态迁移范围内。


## Mark 的最终职责与边界

六个阶段均已完成：原生基线 → 发布准入 → 唯一发现与显式交付 → 操作／授权分离与独立多会话 → 角色清理 → 收尾审计。

服务实例通过运行时名称和能力发布，发布资格由命名空间与授权策略判断。接口角色属于提供方声明，不是服务实例编号；新实例不需要生成Mark，也不加入编译期角色登记。两个实例可以实现同一接口并共享角色声明，明确交付的合法入口可以是NONE。

Mark附在每份能力引用上。转授中的NONE表示继承，非NONE可以重新标记派生引用；标记不扩大真实权限，也不证明身份或允许操作。来源、所有者、存活、类型、权限及业务授权分别核验。raw机制的显式原始构造由调用者负责，语义import先核验native事实。

### 发布、发现与会话

PUBLICATIONS只给出名称提示，unit::PublishEntry不捆绑Mark。普通发布不读取入口Mark，仍校验native来源、owner、存活、访问策略、名称合法性和冲突。Namespace仅开放声明的scope/group/base-path；保留路径不能由普通发布者覆盖。Devices继续核对Control登记的可信Hub、MemberOf Permit、设备白名单及实际联盟资格。

establish::find/claim返回唯一token或Missing／Ambiguous。发现以native owner/Mark与存活为依据，支持owner=0的普通能力；通信绑定另用reserve校验孔类型。只对Missing等待，歧义立即失败，不选择首枚／末枚，不清理外来候选。已绑定端点只核验原连接，失效后不自动查找替代能力。

Operator普通会话统一使用ASK，Wire本身表达操作，SDK由Face直接提供语义方法。granted_berth、Rein和操作Grant已删除。Receiver校验真实请求发件人；Part／Land／Trim仍仅允许Control，Find查条目Permit，Watch查身份绑定，修改仍查归属。没有由调用者标签或本地包装签发操作权限的路径。

Control逐条处理经过校验的LINK请求，将明确端点对交给Operator。Desk以(who,reply,ask)识别会话，同任务可建立多条；共享任一端点的冲突不能替换旧项，重放不重复发ACK，拒绝不污染既有回复端。挂载／ACK失败回滚本次接入；任一端失效只移除对应监听、会话与outbox。删除pending发现、arm和Settling。

Session握手各步共用一个截止期限。give_at明确返回收件方seed，转授或窄化失败撤销本次交付并释放本地资源。打开失败只回收自己创建的端点；克隆共享寿命，释放一条会话不影响同任务其他会话。

### 用户态兼容边界

| 接入形状 | 当前布局 | 旧形状处理 |
| --- | --- | --- |
| Session bootstrap | host:u64 + Control transport seed:u64，16字节；随后交回8字节ASK seed，接收1字节Operator ACK | bootstrap核真实发件人和transport事实；ACK须来自host，不接受旧8字节bootstrap |
| Operator私有Guest，opcode=2 | who:u64 + reply:u64 + ask:u64，加kind共25字节 | 原9／17字节均拒绝；普通操作／失败码不变 |
| Router OccupyLane，opcode=2 | line:u32 + lane:u64 + back:u64，加op共21字节；应答status:u8 + lane:u64，共9字节 | opcode=1的5字节Occupy仅保留codec黄金，生产服务不接受；双方不扫表找lane/back |

每次握手交付的seed属于接收方表，接收端核验真实发件人、vestor、owner、存活和角色。布局变化有精确字节黄金及真实内核探针，不能视为透明重构。普通请求／应答不加关联字段，延迟重复成功回复仍无法和新回复区分；现有会话失败隔离不改变这项限制。

### 角色登记与退役

七个提供方共54个存续角色：28个Channel、26个Grant。原63个固定数值保留在测试基线，9个退役角色列入RETIRED，要求不再登记且不得重用其值；存续角色不重算、不改值。各API与装配继续检查已声明角色的碰撞。

| 提供方 | 角色数 | 实际识别用途与边界 |
| --- | --- | --- |
| Control | 12 | 可信服务／实例入口、回信和Identity引用；校验native调用者、来源、owner与角色，敏感操作另查授权和实例关系 |
| Identity | 18 | 权威签发的17个查询／管理分面与回信；入口绑定已知authority，身份事实不由调用者Mark声明 |
| Operator | 5 | ASK、LINK、TIP、TIP_BACK、WATCH；显式交付、分类与来源核验，操作按Wire分派 |
| Loader | 3 | Build入口、映像和回信；明确seed、来源、类型／范围与接入权限 |
| Hub | 7 | 公共入口、回信、设备存活和私有activation；核对native来源与可信Hub关系，设备准入查Permit／联盟 |
| Terminal | 6 | attach、authority、回信及三条流；attachment能力分类与来源验证 |
| Router | 3 | 入口、lane、back；明确双向seed及完整导入核验 |

退役Operator的8个操作角色及Control LINK。svc/sys/operator/{part,land,find,trim,list,seek,name,watch}的废入口、创建和发布步骤已删除；Operator LINK保留生成常量，移除重复字符串别名。通用Port的私有back没有读取方，改为NONE；回复关系由明确seed、peer和native来源确定。

其余回信、映像、authority和流标签保留，因为所属接口实际核验它们以分类、防止误用，不能把它们当作调用者授权。RTC的rtc-back属于私有导入边界，校验owner、角色及vestor=真实发件人，不参与实例登记。通用ready与Hub的hub／hub-ready各自集中常量，值和原启动关系不变，不能按名字相似合并。Control只在部署／配置边界转换load/ready名称，普通服务与驱动使用声明常量。

### 首轮 Mark 迁移验收

全部宿主套件共237项通过；mold的6个文档示例保持原有ignored状态，不计入通过数。programs全目标、各基础库、system-api与image宿主构建通过。QEMU accept、product、system-fault在最终实现提交41d25a1上均通过，收尾仅更新本文档，没有继续修改实现。

真实场景覆盖动态NONE实例、命名空间越界与无权发布、重新标记不扩权、唯一发现、错误来源、普通写请求拒绝、同任务多会话／别名、单会话退场和资源表回到基线。旧路径检查收取有界RPC的真实Unknown回复，不以零预算超时后的Closed状态冒充服务拒绝。宿主模型不替代这些内核资源验证。

对Mark迁移前的88c217c核对：kernel、schedule、wire实现没有变化；env差异只涉及Mark注释，ABI类型／布局和envcall语义不变。依赖图没有用户态protocol/runtime包；wire、schedule零依赖；image只经纯API依赖env／wire／mold；System实现仍限制在crate内。publication/runtime.rs是局部运行时命名空间，不是被移除的总括runtime库。

文档目录仅保留本文件。阶段1至6的工作已完成，后续新设计单独确定范围，不继续保留待实施阶段或旧操作兼容入口。

## Review 修复与模块层次

这轮同时修复review问题与暴露这些问题的状态边界。目录层次按“功能拥有者 → 完整操作 → 接入适配”确定；父模块拥有状态或完整构造，子模块封装独立机制。纯聚合／转发不额外占一层，模型不依赖运行接入模块。

| 范围 | 拥有者与父子边界 |
| --- | --- |
| Operator | tree拥有命名状态、完整放置／回滚及授权契约；session拥有会话账；watch拥有已核验订阅；connection拥有Control侧接入。runtime注册并适配这些功能，不直接构造未经核验的订阅，不深入tree的内部gate/judge模块。原service/run转发层删除。 |
| Control | unit::Service封装任务与供给端点，提供task/connect/ready/claim_supply并承担Drop；lifecycle通过完整操作推进，不访问裸tuple。endpoint父模块拥有私有Entries，request/instance子模块通过方法适配，不直接改入口字段。 |
| Operator SDK | operator父模块聚合Face、Pane、Tile、Watch，Face私有持有Session；折掉operator/client重复层，兄弟模块通过窄方法协作。 |

半开LINK接入改为有期限的Offer／Request／Handoff状态，每轮只使用POLL。Control维护不等待客户端，失败或过期LINK在原能力退场前不重新接入。等待器监听pending transport并合并其最早期限，忙碌TIP排队仍有界，失败只撤销本次交付和释放本地transport。

Watch登记前核验Hole类型、存活、native giver/owner对应请求者和WATCH_MARK；Hub认领前核验报活端点的相同事实及ALIVE_MARK。错误token不释放，旧账不变，合法认领者退场仍可回收设备。

Service建立供给端点使用事务暂存，部分失败和旧连接替换立即释放本地端点；成功／失败流程离开作用域也由拥有者回收。退出等待的Waited／Unsettled判决已纠正。

Land与Pane::bind共用Loan补偿：Busy／Closed／发送失败和明确拒绝撤销本次转授；明确接纳或已入队后回复丢失则保留，避免误撤服务端已经接纳的能力。无法确认接纳的丢回复仍是明确的责任边界，不能声称全部失败都可立即清零。

Session原始导入与raw访问器显式标为unsafe，要求协调端点交换且不能为同一pair建立并发独立State；from_raw仍不接管释放责任。Clone共享状态及寿命的安全使用保持不变。

新增生产代码回归：operator-connection、capability-import、control-lifetime、operator-client，以及Session导入契约和结构守卫。真实accept额外保留半开LINK并验证健康会话不等待它的5秒期限，还验证错误Watch角色被明确拒绝、源能力保留且后续调用正常。普通操作／失败码、Mark值、Kernel ABI与schedule引擎未改。本轮最终宿主回归257项通过；programs全目标、image宿主构建及QEMU accept、product、system-fault通过。6个具体review问题已修复，原始Session导入的契约风险以unsafe边界收紧；结构守卫约束新的状态所有权、父模块接口和依赖方向。

### 复审后的失败边界补齐

握手兴趣按阶段给出方向：Offer等待调用者reply可写，Request等待本地transport可读，Handoff等待Operator TIP可写。总Waiting以(token,direction)登记/移除成员，背压解除会在截止期限前唤醒Control，不再仅靠期限或其他事件重试。回归实际执行生产Waiting与接入状态机，验证TIP/回复队列恢复及阶段换向。

Hub的Ledger以claim_with封装核可用、交付及提交：实际转授成功后才记Held，错误类型、内核拒绝或死旧主替换失败不产生新活占用。旧claim入口移除；测试执行生产grant，不能用恒成功mock掩盖转授失败。

Watch以Capability拥有事件根，构造任一步失败和正常Drop都封印/释放；服务器登记前摘除失活订阅，同任务同路径可立即重订。源Face/Session仍为借用。回归涵盖完整Watch构造/Drop及实际内核重订。

Land对生产Said只接受完整一字节、已知失败Status为明确拒绝；错误成功形状、未知状态和失败码带尾巴按接纳不确定保留交付，避免错误撤销服务端已保留的资源。语义回执错误发生在完整帧已消费后，客户端不自动重试；下一次调用仍检查残留回复，现有无关联字段的协议限制继续成立。

复审记录：首轮6个具体问题与原始导入契约修复后，第二轮发现背压唤醒、Hub转授补偿、Watch生命周期和Land回执分类4个遗漏；这些分支修复后，授权、会话及失败补偿三个独立只读审查均未发现新的可成立问题。审查结论来自生产源码、Kernel唤醒/回收链与实际执行生产逻辑的测试，不把全绿或字符串守卫作为失败路径的唯一证据。

当前最终验证：267项宿主测试通过，programs全目标与image宿主构建通过；QEMU accept、product、system-fault通过。accept实际验证Watch Drop后同任务同路径立即重订，保留半开连接时正常会话继续接入；背压恢复测试实际执行Waiting和Connection，底层Pile为边界模拟，并核对Kernel出队→Push→Pile的唤醒实现。

| 完成条件 | 当前证据 |
| --- | --- |
| 半开连接有界、不阻塞或自动重启；背压恢复可继续 | production Connection/Waiting状态机5项回归、阶段读写兴趣、真实半开探针 |
| Watch/Hub不接受第三方能力；失败不篡改旧账 | 私有已核验Subscription、Hub事务认领、production import/grant 8项回归 |
| Watch创建失败／Drop回收且可重订 | production Watch/Capability 3项回归、无事件重订模型、真实accept重订 |
| 启动供给资源有明确拥有者，等待判决正确 | Service事务与Drop，control-lifetime 6项、真实启动/故障场景 |
| Land仅明确拒绝撤销，未确定接纳不误撤 | production Said分类与Loan 6项回归，Pane复用同路径 |
| 原始会话导入不冒充安全别名 | unsafe独占/协调契约，安全Clone共享状态，production Session回归 |
| 层次对应功能拥有者和接入适配 | 私有字段、tree父接口、入口登记拥有者、16项架构/依赖守卫 |

结论为上述范围内复审合格。原有无请求关联字段、丢回复接纳不确定、显式raw调用者协调责任等限制保留；不承诺未限定范围内不存在任何缺陷。


## PR #4 本轮职责与失败边界修复

本轮修复八个仍成立的 review 问题，以下记录更新上一轮的最终证据。

| 问题 | 最终边界 |
| --- | --- |
| 关闭的 Operator Session 被永久重试 | 只重试服务器明确返回的 Unknown；传输失败及畸形回执立即结束。生产 Session 回归先造成回复超时，再对已关闭会话使用 Forever，确认不会退避或重发。 |
| Account 混入 System | Account 是独立 ELF，拥有部署声明、API、客户端、账户选择和会话启动策略。装配按声明交付映像与最小能力；通过公开 Control 构造入口和 Identity 事实完成创建。Control 只接受明确授予的构造者、存活 owner 和构造者当前身份子树内的 Subject。 |
| Hub 激活依赖 System 私有协议 | Hub 通过来源绑定的公开 Identity Activate 分面激活联盟；Identity 校验实际管理者、资格、权威、容量和身份衰减，一批请求全部校验后才提交。旧 Hub 私有报文与两个角色退役。 |
| Publication 写死设备拓扑 | 装配生成 owner、scope、group、目录、条目白名单和 Permit 规则；Publication 执行通用命名空间规则，保留设备白名单及联盟准入。 |
| Control 包办 Hub Enroll | 机器供给与 Hub Enroll 属于可信装配 hook；Control 推进通用生命周期。 |
| Images 混合目录与发布入口 | 镜像目录由装配提供，Publication 拥有发布入口及其注入。 |
| Control 拥有 Loader cache | Loader 独占镜像缓存和构造；Control 只登记暂停实例。登记拒绝时回收已构造任务和 Team，登记前不发送完成回复。 |
| Loader 接入层重复转发 | service 父模块直接重导出各功能入口，移除纯转发函数。 |

Account 使用 `/svc/account/create`，保留原两个账户通道的数值；新增 Control construction 两个角色和 Identity Activate 一个分面。登记角色现为 55 个，旧 Hub 激活角色的数值保留在退役清单中，禁止复用。服务 scope 改为部署配置的数字键，原 1–5 以及身份／运行时协议编号保持不变。新构造报文严格承载映像请求、owner 和 Subject，拒绝截断与尾随数据。

此前集成验证修复了内部服务接入边界。本次 Account 已退出内部任务配置：Derive 能力由需求声明授予，消费者通过可信 Control 查询核对，Control 在构造准入时检查实际 owner 存活。测具按构造、登记、准备、完成的次序推进。

QEMU 验证脚本改为保留外部命令的执行结果，避免 `try` 作用域把失败退出码丢成 0。以退出码 23 的外部命令验证错误码传播；真实场景同时检查测试判决及交互驱动结果。

宿主验证共 273 项通过：独立宿主套件 252 项、IdentityBook 13 项、mold 8 项；mold 原有 6 个 ignored 文档示例不计入通过数。programs 全目标检查通过。构造报文覆盖完整／截断／尾随数据，Identity 覆盖 18 个动作的编解码和错面拒绝矩阵，并检验激活的原子性、资格、错误管理者、过期权威和衰减边界。

最终同一份实现的 QEMU release 验证全部通过：accept 28.95 秒、system-fault 33.06 秒、product 登录交互 8.47 秒。交互脚本覆盖错误账户／密码、密码编辑且不回显、取消登录、两次不同 Task 但相同 Principal 的 cat 会话、EOF／中断注销及终端恢复。此前运行出现过仓库已记录的 Operator 连接超时签名；本轮不声称消除了既有时序抖动。

复现：

```sh
cargo check -p programs --all-targets
for manifest in crates/schedule/src/tests/Cargo.toml crates/execution/tests/Cargo.toml crates/resource/tests/capability/Cargo.toml crates/ipc/tests/*/Cargo.toml crates/wire/tests/host/Cargo.toml programs/tests/*/Cargo.toml; do
  cargo test --manifest-path "$manifest" --target x86_64-unknown-linux-gnu || exit
done
sh programs/src/system/identity/book/test-host.sh
cargo test -p mold --target x86_64-unknown-linux-gnu
nu scripts/qtest.nu --package kernel --scene accept
nu scripts/qtest.nu --package kernel --scene system-fault
nu scripts/qtest.nu --package kernel --scene product --feed-script programs/tests/terminal/session.py
```


### 2026-10-09：连接超时与终端发现竞态复查

“时序抖动”不足以描述测试失败。复查在 `275e0f92` 上七次运行得到六次通过、一次 Terminal 失败；其父提交 `bd557643` 的隔离工作树也出现 Terminal 移交／恢复失败，不能把所有失败归入同一个 Operator 签名。

Operator 接入原先对每个登记任务重新枚举整张能力表，并对每项执行 Alive／Reserve 查询。控制任务在这段工作中无法推进下一步握手。旧实现加入耗时诊断后，复现 `probe-control: no tree link`，紧邻的两轮扫描分别耗时 450 和 806 毫秒，超出探针 1000 毫秒的共享握手预算。改为每轮只枚举一次，用 Collect 返回的 owner／mark 筛选，再对新 LINK 候选核验 giver、owner、mark 和存活状态。没有延长探针期限。回归用 32 个请求任务和 2000 个无关能力检验接入成本界，同时确认伪造来源仍被拒绝。

Terminal 的另一类失败表现为恢复成功后发现 OUTPUT 返回 Missing，或者探针随后再次查找数据能力返回 Missing。按下标调用 Collect 的枚举不是快照；并发撤销删除前面表项时，后续索引会跳过一项。关闭 Operator 会话后的运输能力清理也会修改调用者的能力表。Terminal 响应现在明确返回实际接收者的 input／output／control 编号，Connection 在 attach 和前台恢复确认后更新它们，Io 使用这些编号。探针也直接使用已确认的数据句柄，仍检查旧句柄被撤销、独占控制移交及子任务死亡后的恢复。Terminal Reply 由 9 字节扩为 33 字节，服务端与客户端须一起更新；角色数值、权限与 Kernel ABI 未改。子任务启动时的 injected 数据发现接口保留。

新增生产 Connection 回归覆盖不枚举能力表的接入、失败移交后的新端点恢复，以及正常前台恢复时旧 Io 的撤销；编解码回归核对三个接收者编号和截断拒绝。诊断耗时与临时失败打印不进入最终实现。

最终验证：277 项宿主测试通过（独立套件 256、IdentityBook 13、mold 8；6 个 ignored 文档示例不计入），programs 全目标检查与 diff 检查通过。最终 release accept 的同镜像重复运行 20/20 通过，范围 8.09–10.79 秒，中位数 9.70 秒；完整脚本的首次 accept 也通过。system-fault 15.75 秒通过；product 登录交互 5.00 秒通过，驱动结果为 ok。登录验证仍覆盖两个不同 Task、相同 Principal 的 cat 会话及终端恢复。重复运行记录的是这 20 次的结果，不据此声称所有运行条件下永不超时。


### 2026-10-09：Account 退出 System 装配实现

Account 以 `prog-account` 独立运行，部署声明归 `service/account/program.rs`。移除 `system/app/account.rs` 及其安装、消费者刷新、接收、等待、健康检查和退出专用分支；不存在跨任务共享的 Arc／Atomic 配置。帐号名、Login 策略、cat 会话映像选择和 Terminal 接入路径均由 Account 拥有。

System 只提供通用声明驱动的能力：

- `Demand.construction` 明示构造资格；实际转授成功后才登记构造者。每次创建仍校验 native sender、owner 存活与真实 Identity 子树，转授入口本身不赋予构造资格。
- `Demand.identity` 明示附加身份分面；Account 仅申请 Derive。它派生独立的用户 principal，避免把服务身份直接作为用户会话身份。
- `Setup::Image { name, load }` 交付页对齐的只读映像能力与严格 16 字节 recipient seed／length 帧。映像供给必须声明 Ready；服务完整复制后才交 Ready，供给根在 Ready 或失败回收时释放。空、重复及碰撞通道在启动前被拒绝。
- Control State 新增 `Task(name)`，操作码 10；原 1–9 和 Grant／Mark 保持不变。查询只返回可信部署登记且仍存活的真实 TaskId。Account 同时核对 native sender 与 Login 身份，继承同一 principal 的子任务仍不能冒充 Login。
- `Identity.aliases` 声明允许发布的名字，不能直接把名字绑定到服务 principal。公开 IdentityName 发布要求存活的命名部署任务、允许的名字、NONE 载荷、Bound Permit、Principal 类型及真实身份子树授权；别名随发布任务退场清理。Account 将 anran 绑定到派生的用户 principal。

发布 `/svc/account/create`、领取映像、身份派生与别名安装完成后，Account 才报告就绪。Login 的部署顺序显式等待 Account。服务失败与正常退场走普通 Control 生命周期及 Publication 清理，不需要 System 识别 Account。

验证覆盖生产 Construction 转授、Service 供给持有／回收、Control 查询与 RPC 来源校验、IdentityName 发布准入。完整宿主套件通过；架构守卫禁止 System 导入 Account 实现／API／客户端或持有帐号与消费者名。真实 system-fault 保留认领过期、准备／启动失败、清理 Pending／失败重试、owner 死亡与资源回收，并新增相同 Login principal 的非 Login 任务拒绝、获构造资格后的越界 Subject 拒绝。

最终 release 场景验证：accept 5.39 秒、system-fault 10.76 秒、product 登录交互 2.93 秒通过；交互驱动结果为 `ok`，两个 cat 会话 Task 不同、Principal 相同，终端在 EOF／中断后恢复。权限与生命周期两项独立复审均未发现新的阻断问题。

### 2026-10-09：能力转授记录与任务锁

来源查询原先通过父 token 扫描所有任务的能力表；撤销和 Forget 也靠全系统扫描寻找子能力，并由全局 `GRAPH` 串行化修改。现在删除 `GRAPH` 和能力快照提供方，不新增内核全局变量。

`Pie` 保留父 token 字段 `sire`，增加 `lord: Weak<Task>` 指向父能力所在任务。根能力的 lord 为空；转授和 Forget 同时设置 sire／lord。`Task` 增加 `heirs`，按父 token 排序保存父 token、接收任务弱引用和子 token；原有 `Pie.heir` 仍只表达独占转授。来源查询只访问当前任务和父任务；撤销只收集实际后代及需要解除记录的父任务，不再扫描名册。任务内能力查找和排序 Vec 的插入／删除仍有线性成本，本次不宣称它们都变为常数时间。

每个 Task 的 `gate` 锁保护能力及转授关系修改，锁内 bool 表示退出清理是否已关闭能力接收；`version` 记录修改次数。转授按 TaskId 顺序锁住双方，自转授只锁一次。撤销／Forget 先逐任务收集范围及 version，再按 TaskId 顺序锁住涉及任务。Forget 复核全部 version；撤销在 version 变化后复核实际父子关系，确认没有遗漏后代、所有涉及任务均已加锁时仍可提交。普通调用最多重试八轮，相关关系持续变化返回 Busy。退出没有调用者替它重试，因此继续处理版本冲突；退出仍保留原先分配失败时先封印自有资源、尽力清表的行为。

授予先为两边记录预留空间，再修改独占状态并发布子能力；分配失败不提交半条关系。Forget 将直接子能力的 sire／lord 及父任务记录一起更新；释放、撤销、退出同步解除父子记录。本地创建、启动资源交付、缩权与构造提交也取得任务锁并更新 version。来源查询复核当前父关系和父 token 存在性，缓存任务身份不代替验证。

任务锁按编号取得并逆序释放，最后才恢复中断；资源取消映射和通知在任务锁之外完成。debug 锁检查原来只有八个固定记录槽，现改为在多任务加锁前可失败地预留记录空间，检查本身不分配；真实自检覆盖同时取得十七个任务锁的撤销。独占来源在接收端能力发布前进入转授状态；清理旧 heir 会在取得来源任务锁后复核接收端，既不误清尚在提交的转授，也不擦掉后来建立的新 heir。

退出清理须撤回借入后再次授出的内存能力，即使相关内存操作正在进行。普通释放／撤销保留 Busy 检查；退出走强制撤回路径，与原来的退出语义一致。新增回归先复现误用普通 Busy 检查导致的遗漏，再验证修复；真实内核自检也覆盖该情形。

新增 `kernel/tests/gate` 直接编译生产 gate 模块，以宿主线程运行并发转授、撤销、Forget 和退出，用线程内分配失败注入验证回滚。仅任务调度、映射和通知由宿主替代，实际自旋锁、中断恢复、Permit 与映射另外由 QEMU 自检和整机场景覆盖。用例也确认：持有一个无关任务锁时，其他任务间的转授仍能完成。

复现宿主测试与内核检查：

```sh
cargo test --manifest-path kernel/tests/gate/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check -p kernel --all-targets
```

独立运行健康面时，cargo-qtest 可通过字面 `--` 将 `--skip scene` 交给测试过滤器；QEMU 参数仍来自 `scripts/qemu-args.nu`。这避免将未提供 initrd 的预期 scene 失败混入健康面结果：

```sh
mapfile -t board < <(nu scripts/qemu-args.nu --board-only)
qargs=()
for arg in "${board[@]}"; do qargs+=("--qemu-arg=$arg"); done
RUSTFLAGS='-Crelocation-model=static -Cforce-frame-pointers=yes -Ccode-model=medium' \
  cargo qtest --target riscv64gc-unknown-none-elf --package kernel "${qargs[@]}" -- --skip scene
```

验证过程中，同步 Account 新提交之前的中间版本曾在第十五次 accept 出现通用安装失败：`accept: install unit: 3`，随后 `PUSHED=29 REAPED=28` 导致整机等待超时。增加失败上下文的诊断版连续二十次未再出现该失败，无法据此确定那次安装失败的原因；退出 Busy 回归的修复也不作为它的确定归因。诊断代码已移除。同步期间保留了 `2c7fa231` 的全部 Account 部署与测试改动，并重新验证合并后的实现；被停止的同步前运行不计入最终重复结果。

最终合并版验证：312 项宿主测试通过（独立套件 272、内核转授 19、IdentityBook 13、mold 8；原有 6 个 ignored 文档示例不计入）。新增内核转授用例连续二十轮全部通过；真实内核健康面 32 项通过，scene 单独运行。kernel／programs 全目标检查与 diff 检查通过。无临时诊断的最终 accept 连续 20/20 通过，耗时 10.02–18.03 秒、中位数 11.70 秒；system-fault 18.41 秒通过，product 完整登录交互 7.47 秒通过且驱动结果为 ok。重复次数只描述本轮观察结果，不据此保证所有运行条件下没有超时。

### PR #4 review 后的两项修复

Account 的请求缓冲区为 64 字节。原接收循环在 Pull 返回 Denied 后重试，但超过缓冲容量的消息仍停在队头，因此正常请求无法继续。Mail 在原六个调用之后追加 `Discard { token, max }`（class 5、操作号 6）：要求 FETCH，在队列锁内检查并仅移除长度超过 max 的队头；正在读取返回 Busy，正常长度、空队列和未完成的发送预留均不消费。销毁载荷、更新原有诊断计数和唤醒发送方在锁外完成。普通 Pull 的不消费语义保持。RPC 接收端在 Denied 后显式尝试丢弃超长消息，成功时报告 Decode；不根据 Peek 的旧结果移除另一条正常消息，也不分配与外来消息等长的缓冲。

撤销收集范围后，无关能力的缩权也会增加任务 version。原实现据此反复拒绝提交，可能耗尽八次重试。现在 version 相同仍走原检查；发生变化时，在全部任务锁下复核实际父子关系，确认每份后代仍在、父关系一致、新增后代没有遗漏，且当前父任务已加锁。关系扩展到未收集任务或发生移除时重新收集。Forget 仍使用完整 version 检查。未增加内核全局变量。

Capability、Loan、Reply 和 RPC 服务端回信能力的清理使用 resource 的 release／revoke 包装，Busy 时保留本次清理并通过 Starve 让出处理器后重试，不把忙碌结果静默当成完成；其他错误仍按原契约返回。直接 env 调用的单次尝试及原失败码不变。

回归覆盖超长消息后正常请求继续、12 次连续 Busy 后完成释放／撤销、无关缩权时直接完成清理、锁前新增后代时重新收集、Forget 把父关系迁往未加锁任务时重新收集，以及 Mail 原调用编号不变。真实内核覆盖正在读取、正常长度、发送预留、封印和诊断计数；Account 整机探针先发 65 字节消息，再执行原有账户创建和实例故障流程。

本轮验证：318 项宿主测试通过（独立套件 274、内核转授及 ABI 23、IdentityBook 13、mold 8；原有 6 个 ignored 文档示例不计入）。真实内核健康面 33 项通过；kernel／programs 全目标检查及 diff 检查通过。QEMU release：accept 14.29 秒、system-fault 21.46 秒、product 完整登录交互 6.31 秒，交互驱动结果为 ok。每个整机场景各运行一次，本轮不据此声称消除了所有时序抖动。

### 内核引用表改用 trait 对象（0c53dff5）

保留 `Pie<Hole>`、`Pie<Pole>`、`Pie<Nole>`、`Pie<Tole>` 的字段和资源实体，将 `AnyPie` 枚举改为 `Box<dyn PieOps>`。公共引用方法在 `impl<T> PieOps for Pie<T>` 中实现一次；父引用、lord、独占 heir 和权限修改不再逐一转发四个枚举分支。资源 trait Mail 提供类型、所有者、存活、关闭、派生引用及安全的类型访问；类型不符仍由调用返回原有 Denied。

每份创建或授予的引用独立分配 Box，不共享引用的权限或父子关系。locate／accede 返回 PieSnapshot，复制引用事实并持有原资源的 Arc 和原 Permit；创建和读取快照不分配堆内存。内存快照通过共享 Permit 读取有效权限，能观察缩权和撤销；持有快照只保证对象内存有效，不代替资源存活和当前转授关系检查。Same 比较原资源 Arc 的身份，类型擦除不创建替代资源实体。

保留任务内 gate、version、heirs 索引和多任务锁顺序，不增加全局变量。PieToken 的线程限制不变，PieOps 只要求 Send，表内访问仍由现有锁保护。新 Box 分配失败返回 OoM；Accord 完成新引用分配和两端记录预留后才提交，UnsealPole 在 Box 或表项插入失败时撤掉自动映射。清理映射和通知继续在原来的锁外阶段完成。

本轮保持 Pie、Mail、Tole 的现有调用编号、参数、返回值及错误码；统一 Unseal、调整等待条件等 API 设计尚未实施。每份引用增加一次 Box 分配并使用动态分派，本轮没有据整机场景耗时宣称性能改善。

本轮验证：内核转授及 ABI 宿主测试 28 项通过，其中新增 5 项覆盖快照零分配、四种资源身份与类型访问、引用权限和父关系隔离、内存快照观察缩权／撤销、独占授予 Box 分配失败回滚。真实内核健康面 33 项通过，kernel／programs 全目标检查及 diff 检查通过。QEMU release：accept 11.71 秒、system-fault 17.95 秒、product 完整登录交互 6.75 秒，交互驱动结果为 ok；每个整机场景各运行一次。此前其他宿主套件的结果见上一轮记录，本轮没有重新运行并重复计入。

### Pie、Mail、Tole 调用统一

落实已确认的调用设计；按用户要求删除旧入口，不保留兼容调用。Pie、Mail、Tole 的 class 仍为 7、5、9，操作号分别显式指定为 0–10、0–5、0–4。内核与程序 ELF 一起更新，其他调用域的 ABI 不变。

| 调用域 | 当前入口 |
| --- | --- |
| Pie（11） | Unseal、Open、Shut、Seal、Accord、Narrow、Revoke、Release、Inspect、Collect、Same |
| Mail（6） | Push、Pull、Wait、Hush、Ring、Peek |
| Tole（5） | Attach、Detach、Await、Subscribe、Unsubscribe |

`Pie::Unseal(UnsealArgs)` 统一创建 Hole、Pole、Nole、Tole；四种资源保留自己的参数。Hole 的 `HoleLimits { max_len, max_messages, max_bytes }` 限定单条长度、包含发送预留的消息数和总字节。默认保留四个消息位置；长度与总字节默认不另设应用限制。单条超限返回 Denied，队列或字节预算暂时不足返回 Busy。预留先计入字节，失败、取消和接收归还预算，复制及分配在队列锁外进行。Nole、Tole 创建使用可失败分配，返回 OoM；Pole 创建失败仍撤回自动映射。

`Release(token, ReleaseMode::Revoke)` 撤回引用及其后代；`Keep` 保留原 Forget 的限制和父子迁移语义，只移除借入的非独占引用，后代仍受上游撤销控制。用户态 RAII 的默认清理仍选择 Revoke，并在 Busy 时让出处理器继续重试。

`Inspect` 返回 `PieInfo { token, kind, permission, owner, vestor, mark, alive }`。封印后仍可查询原所有者和完整 64 位 mark；表里没有引用才返回 Denied。跨 ABI 使用七个完整机器字，不依赖 Rust 结构体填充。resource::raw::inspect 返回类型化记录；Alive、Reserve 只保留为用户态便捷函数，授权判据显式检查 alive。枚举根引用直接返回无授予者，避免逐个重新查表；借入引用继续复核实际父关系。

`Collect(after, buf, capacity)` 按递增 token 返回批次；游标是已返回的最后一个 token，不使用易变化的表下标。内核每批最多写 16 条，使用栈上快照，释放引用表锁后复制给调用者。resource::raw::pies 每次读取八条。删除先前表项不会使后续位置前移；并发增加和撤销仍以实际调用时状态为准，不保证整张表的冻结快照。

Mail 与 Tole 共用 `MailCondition::Pull / Push / Empty`：Pull 表示可读，Push 表示队列有消息位置及字节空间，Empty 表示没有消息、发送预留或待应通知。Pull 需要 FETCH，Push／Empty 需要 STORE；Push 只是空间提示，不能保证某个长度的消息一定放得下。原先等待发送完成的调用点改用 Empty，发送重试等待 Push。Nole／Pole 只支持 Pull，Tole 不能嵌套。Tole Attach／Detach 按所选条件检查成员权限，Await 只返回当前持有且权限相符、未移交的成员引用。状态订阅仍要求复核实际状态，不承诺逐条变化历史。

`Pull(..., Oversize::Keep / Discard)` 一次决定接收或拒绝超长队头；成功返回 `PullOutcome::Received / Discarded { len, sender }`。Discard 只用于确实超长的消息：非法缓冲区、类型或权限失败均保留消息。RPC 收包直接使用 Discard 策略，不再先 Pull 再调用另一个 Discard。读取守卫一次取得队头与共享载荷，失败自动恢复可读，完成后才移除。共享载荷允许读取期间资源关闭，不使字节引用失效。原来的用户态 wait 把首次阻塞排除在期限之外，这轮改为首次调用前计算期限，回归验证不会消耗两份等待预算。

本轮没有新增内核全局变量，原有任务 gate、version、heirs、跨任务锁顺序和 PieSnapshot 机制保留。查询及等待都不能替代后续操作的权限检查。

验证：330 项宿主测试通过（33 个独立套件 276、内核转授及 ABI 30、IdentityBook 13、mold 11；mold 原有 6 个 ignored 文档示例不计入）。真实 QEMU 内核健康面 35 项通过；kernel／programs 全目标检查和 diff 检查通过。accept 新增真实调用探针，覆盖四类创建、消息／字节限制、三种等待与成员权限、无效缓冲区及权限失败不丢消息、超长拒收后正常帧继续、封印后查询、批次之间删除前项仍不跳项。最终 release：accept 11.91 秒、system-fault 22.04 秒、product 完整登录交互 6.53 秒，驱动结果为 ok。最终版本的三个整机场景各运行一次，未进行专项性能比较。
