# System 与用户态基础重构

本轮重构及 Mark 迁移已完成。中央 protocol、总括 runtime 及 System 的 core/serve/run 分类已退出。内核 ABI、普通请求／应答的操作码与失败码、资源模型、任务生命周期及多 binary 打包方式保持不变；用户态接入握手的升级边界见下文。

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
│   ├── launch/              # 身份准备、授权接入、命名空间准备、就绪交付
│   └── account/             # 帐号会话机制；配置由 app 注入
└── service/hub/、user/terminal/、driver/router/
    ├── api/                 # 软件自己的接口声明和生成元数据
    └── client/              # 软件自己的公共使用库

```

System 只对独立程序开放 app::run；实现组件限制在 crate 内。普通驱动、服务和用户程序使用公共 API/客户端及 support，不导入 System 私有目录。压测与 accept 的独立二进制为薄入口，具体测具留在 harness。

Control 唯一持有任务表、实例表、待放行任务和 Loader 缓存，字段仅 Control 子树可见。创建依次执行容量检查、构建、登记；外部通过命令与只读观察访问状态。受管任务的启动进度归 Control；app 保留程序阶段、空闲、整体退出策略与程序层 Fault。

IdentityBook 只拥有权威身份模型。Control 的 Roster 拥有安装账，普通调用方的受信 authority 发现归 system-client。Operator management 是私有管理入口，不承担公共客户端职责。

publication 拥有发布、名称与运行时命名空间的账；Operator 自己拥有会话接入，publication 不再保存连接候选账。launch 只持有等待交付的请求，通过 Control 查询实例结果，不复制实例状态。account 根据注入的帐号与镜像配置处理可信 Login 请求。

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
