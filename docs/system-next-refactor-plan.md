# System 本轮重构方案与验收

状态：本轮实现与约定验收已完成（2026-10-12）。本文是 docs 中唯一保留的方案和验收记录。用户授权的身份迁移、来源校验、旧源码清理与测试迁移已落实；提交由用户明确指示执行。

## 总体边界

- UnitFile 是声明式装配配置，由各功能模块的纯配置组成；模块拥有自己的机制和状态，Assembly 组合操作，schedule 执行计划。
- 创建按声明配置的实例与直接运行镜像共用构造机制，但直接运行镜像不要求 UnitFile；程序内部原生线程继续使用现有 Spawn。
- 可执行镜像资源与 UnitFile 分别登记。部署选择、运行依赖、打包依赖分别处理；镜像来源不决定执行特权。
- Control 拥有实例与生命周期，Assembly 拥有作业、游标和安装清理记录。Unit 名称与运行实例分开，同一声明或所有者可创建多个实例。
- Account 负责身份选择和会话创建；Shell 的命令目录、命令镜像获取和缓存归 Shell 所属提供方。Account 声明不包含 workers、Hub 或命令镜像清单。
- 自举复用现有 schedule 的 Plan、Cursor 和 Resources，以可信初始根资源和明确依赖驱动，不另建执行器。
- Mark 继续用于能力定位，另行核验来源、种类、权限与存活；撤回启动描述页和为句柄编号修改 Spawn ABI 的提案。
- 重构协议允许不兼容，迁移所有调用方并删除旧协议和客户端，不保留兼容层。

## Operator 通用条目与授权机制

- 提取通用的目录取得、资源绑定、授权、权限收窄、撤销和清理机制，不限定于 /uit/{team}/{task}。
- /uit/{team}/{task} 保留为一种运行实例布局；其他布局由使用者和组合处决定，不预设 public / bound / hole 子目录。
- Operator 是条目、绑定和授权事实的唯一拥有者。关联对象的模块保存并直接返回真实条目引用，不另算路径或维护另一份绑定事实。
- 条目引用明确所属 Operator；引用标识对象，授权能力限制操作，裸编号和路径均不代表权限。
- 同一父目录下的 part：缺失则创建；已有目录则返回同一条目且不重复产生创建事件；已有资源绑定则报类型冲突。
- 根目录 / 的修改权限只属于可信初始化指定的 System 任务，不向其他任务授予根写权限。System 可以创建子空间并向其他任务授予局部权限；子授权不能扩大范围、修改父目录或替换授权锚点。
- System 建立或取得 /svc、/svc/drv 等共享目录，再直接交付实际条目和受限发布能力；各程序不从根重复创建公共路径。
- 驱动可以获得自身子目录的权限，也可以获得共享目录内指定名字的绑定权限；不强制每个程序拥有独立目录。
- Service、RuntimeResource 等专用发布类别由通用绑定与授权机制替代；接口访问策略仍由 Permit 表达，资源来源和可设置 Permit 由授权限制。
- 程序退出只撤销其授权并清理归属于它的绑定。共享目录由其管理者控制生命周期，不因单个程序退出而被删除。
- 同位置争用返回冲突；替换和清理使用实际绑定及版本检查，避免旧实例操作新实例。名称解绑与已交付资源能力的撤销分别处理。

## 构造结果与回收

- 创建请求在发送前分配 CreateId；同调用者同编号同参数查询同一结果，参数不一致冲突。查询与取消处理回复丢失，取消后晚到的创建不能复活请求。
- 动态实例准备完成后等待 Claim，之后安装运行资源并启动。未 Claim 期限从 Prepare 完成计算；重复 Claim 不延长期限，回复失败不撤销已成功的 Claim。
- 失败、取消和退出执行明确清理流程，清理完成后才报告最终结果；保留原始失败。资源租约和名称空间授权分别可查询、撤销和回收。


## 最终模块与状态归属

- app：组合根、部署选择、模块资源登记、稳定计划段、整体运行与退场策略、总等待器；不拥有各模块请求流水线。
- Assembly：统一 Construct、Prepare、Activate、Retire 作业，持有待完成调用、每作业 Cursor、原始失败和安装清理记录。瞬时作业完成后释放临时记录，运行中实例仍需保留的清理记录随实例关联保存。
- Control：任务和实例登记、真实 Task/Team/owner、认领、就绪、启动、终止、观察和生命周期命令。静态和动态实例共用生命周期，作业进度迁入 Assembly，不保留第二份实例状态。
- Loader：校验镜像、受控快照、不可变镜像缓存、映射及任务构造；不拥有作业策略和任务身份。保留先预留容量再构造、成功立即登记的现有保障，不新增 registered 标志。
- Identity：主体、联盟、绑定和身份安装机制。安装账归身份安装操作的拥有者，Control 只保存关联和生命周期事实。身份引用发布通过 Operator 通用绑定；不复制一套别名权限模型。
- Operator：实际树、条目、绑定版本、授权账、会话、导入核验和订阅。布局及业务对象到条目的关联由组合处或所属模块建立。
- Supply：通用资源获取、交付和租约流程；真实设备、镜像、命令目录等资源定义及其业务状态归提供方。
- api：host-safe 的纯配置、协议、类型和生成元数据；client：公共语义调用。普通程序不依赖 System 私有实现；image 宿主工具不能依赖执行库或客户端。

现有 app/assembly 的静态装配、launch 的动态准备/交付和 Control 的装配游标合并进 Assembly；launch 只剩入口适配需要时并入对应构造入口。Publication 的名称空间授权及绑定迁入 Operator，身份引用业务迁入 Identity，供给迁入 Supply；职责迁完删除 Publication 及其旧 Target/协议/客户端，不保留空转发壳。

## 声明、部署与镜像

UnitFile 以 name 加 loader/control/identity/operator/supply 的纯 Config 组合，不把打包选择、源路径、镜像来源类型和业务专用字段放进 UnitFile。各 Config 的类型只依赖纯 API。自己的可执行镜像逻辑资源引用归 loader::Config，space/args/stack 归 Loader 配置；执行特权不由镜像供应来源决定。

Control 配置明确 after 的 Unit/Target 引用、Ending、Ready 约定。Ready 显式区分放行即就绪与程序交回信号；不再按全局依赖扫描推导 Setup::Ready，Ending 不叫 restart。普通动态镜像请求不自动取得静态服务发布、声明供给或信号 Ready 约定，但可按基线安装自己的运行名称空间。

部署是纯组合配置，不是服务。可沿用 Scene 命名；明确 entry、start、targets、bootstrap 和 sources。after 不自动选择服务；选择之外的依赖、缺失镜像、非法引用和循环必须报错。Target 独立声明成员和就绪屏障，不再使用空 UnitFile、SCENE_UNIT 或“所有其他服务”隐式屏障；不再靠 operator/identity 名称特判排序。

Package 管宿主 binary、物理路径和打包方式。部署选择、运行等待依赖和打包资源引用分别计算。包闭包从部署入口、显式选择、bootstrap 和资源引用展开、去重、校验；取得供给镜像不等于启动该程序。cat/emit/upper/fail/spin/workers 等纯镜像资源不为了打包保留空 UnitFile。本轮实现 initrd 资源后端和通用来源契约，不扩展成完整文件系统服务。

## Operator 授权契约

- 每份权限以真实目录条目锚定；EntryId 不是凭证，Where::Root 与 EntryId(0) 必须区分。删除重建不继承旧授权；绑定 replace 保持位置也须用版本避免误操作。
- 操作包括 grant、narrow、part、unpart、bind、replace、unbind、inspect、revoke。有限相对名字或子树范围、操作权限、Permit 约束以及可选真实来源 Task/角色分别表达。子权限同时受所有祖先限制；不猜测身份 Selector 的子集关系。
- 每份授权使用独立真实入口，只交付所需 STORE，不交付原始 VEST 绕过语义授权。授权直接由 Operator 交付给 holder，签发方获得交付元信息；本地引用 Drop 不隐式撤销整棵授权分支。
- Binding 包含 entry 和 revision。新绑定占空位；相同授权、同资源及同 Permit 的重放可返回原结果；replace 使用期望版本；解绑已不存在可幂等成功，版本冲突不能删除新绑定。
- GrantId 由调用方发送前确定，记录以实际授权入口、native caller 和 GrantId 识别；inspect 查询 Unseen/Pending/Granted/Revoking/Closed。相同请求复用结果，参数不同冲突。revoke 可针对授权或 GrantId，取消未见请求形成关闭记录，晚到请求不能复活。
- 请求编号不复用；每个实际入口按首次发送顺序接收，使用有界记录及水位拒绝旧请求复活。授权失效立即禁止写入及发现；完成回收后才报告撤销成功。
- 初始可修改根的权限仅可信初始化创建，固定绑定 System native 任务；普通调用者和 Mark 不能签发根权限。旧私有转发及公开修改旁路必须一起迁移。
- 容量有界，活动及回收授权初始上限 256，路径最多 8 段/255 字节，名字沿用合法性与 31 字节界限；满容量明确 Full 并清理本次已获资源。不要为旧目录根哨兵或旧码值保留兼容分支。

## 通用供给与身份安装

Supply 配置由通用 Binding 组成：source、resource 逻辑键、slot Mark、permission。资源定义归提供方，组合根注册；不增加 Hub/Workers/Account 专用字段或巨型业务枚举分派。

供给操作 acquire(LeaseId,target,resource,slot,permission)、inspect(LeaseId)、release(LeaseId)；交付直接到收件方，返回收件方 seed 和有界描述。集合用只读载体，不以远端裸编号当能力。每项租约有独立私有资源根，release 使用 Revoke 回收转授以及 Keep 后留下的后代，不影响兄弟租约。Ready 与租约寿命分开，实例实际回收前保留资源。

租约发送前编号，绑定真实请求者和来源入口，有界状态 Acquiring/Granted/Releasing/Released；支持 Pending/未见请求的取消、水位及相同参数幂等。提供方失联保持结果未知，不能假报释放成功；提供方拥有内部依赖清理。

身份安装以 InstallId 关联一次完整安装，并提供查询与回收。选择/派生、校验、绑定和授予在安装操作中闭合，丢失回复后不能重复派生主体。回收任务绑定和已授资源，不删除仍被引用的主体。本轮不增加主体 GC。

## 统一构造、启动和清理

两类语义输入：Unit 请求带 UnitRef、获准的 owner/identity/args 覆盖；Image 请求带有效镜像能力/范围/args/stack。Image 默认 owner 与身份继承 native caller，固定 User；改变 owner/identity 或特权需明确授权，存活或数字编号不是授权证明。构造能力按可接受输入和参数范围授权，删除 construction bool、constructor bool 以及全局 creators 仅按任务放行的粗授权。

保留现有镜像 Loan 与来源核验、快照上限 16 MiB、参数上限 64。若实现来源直接交镜像，必须闭合真实 requester/CreateId/接收者的关联和凭据，不以数字 seed 跨任务引用代替授权；初期可继续 Loan+snapshot，不能为了简化删除校验。

Construct 解析输入、校验授权、预留容量、取得镜像、构造并立即登记。Prepare 安装身份、名称空间及供给；静态 Mint 可只 Construct，Embark 走 Prepare+Activate；动态返回准备好但尚未启动的实例。Claim 后安装管道等运行资源，再 Activate。

InstanceRef 标识一次实例，旧回调不得击中新建同名实例。同一实例一次只有一个修改作业；重复 retire 合并，其他冲突 Busy。内部操作 construct/prepare/activate/retire 返回 JobId，take 只消费最终完成结果。用已有 Dispatch/Invocation/Completion 有界轮转；单作业 Pending 不阻塞其他独立作业，不为每个实例重复插入相同 TypeId 的 Resources。

CreateId 记录绑定 native caller 和实际构造授权入口；同参数重放查询原结果，不产生新 Task。取消形成终态，不延迟复活。Account 外层会话请求编号关联内层创建编号，避免回复丢失重复创建 Shell。Claim 同 owner/实例幂等，未领取期限从 Prepare 完成算起，默认保留 3000 ms；已领取后的重复查询不延长期限也不受未领取超时影响。送回复失败不立即停止已可查询的创建结果或撤销 Claim，未领取期限和 owner 退出负责最终回收。

Retire：禁止新工作并失效名称空间授权，停止/等待任务，释放供给租约及身份安装、清理绑定，再回收实例。按明确清理责任推进，不写泛用反向 rollback。失败保留原始原因，清理 Pending 可恢复，不能资源未回收就报告成功。整体退出动态依赖者先于静态提供方，独立项允许同时清理，停止中的后继回收完成前不能停止其提供方。

## 自举和业务调用方

自举复用现有 Operator/Identity 内部任务的发布 ID 与共享初始资源。必要互相引用先建任务/交付可信标识再释放，不用名称排序、字符串路径猜来源或另一套握手机制代替 schedule。模块安装自身私有 Resources，提供完整局部 Plan；app 只连接明确依赖、组合稳定阶段和汇总等待兴趣。现有 schedule Missing Resource 是错误，不是 Pending，资源必须预登记。

根和共享目录由 System 的可信初始权限建立。首次发布不依赖已发布的自身入口；由初始资源和直接条目/能力启动。启动能力使用唯一 Mark 加可信来源/种类/权限核验；多份同角色能力使用显式交付描述，Mark 不证明权限或实例身份。

Account 选身份并用限定 Shell Unit 的构造能力创建会话，不缓存 Shell/命令 ELF，不持有命令目录。Shell 所属提供方拥有命令定义、镜像获取/缓存及目录交付，Shell 负责管道和作业。Shell 创建命令走 Image 请求，不使用“命令 Unit 白名单”。Hub 自己定义设备资源与安装契约，System 组合注册资源，不通过 UnitFile 的业务标签 special-case Hub。

删除同 owner 只能创建一个实例的 owns_team_instance 准入限制；各实例身份、终端、目录、授权、租约与 CreateId 分开。支持多实例所需机制，不将实际多用户认证/多终端 UI 扩成新业务范围。

## 实施次序与验收

用户已明确要求派 Luna 实现本方案。先做既有装配菱形、重复状态及调用路径的归并，再落实新增授权/编号/租约机制；按纯 API/模型、模块操作、接入/客户端、组合、全部调用方、打包、真实验证推进。局部签名和编码在已定边界内可自主落实，重大偏离必须报告，不用设计关卡为已授权实施重复请求许可。

开始记录实际基线：Loader::construct 的静态/动态两个调用点，UnitFile 的四类混杂字段、55 项登记，静态 lifecycle 与动态 hook 两套游标，Publication 专用 Target 类别与绑定账、Account 命令供给表。结束报告实际重复路径和状态字段的减少；新增必要机制单独说明，不伪造净减数字。

必须完成：

1. 全调用方和纯宿主工具迁移，删除旧配置字段、协议、接口和废代码；不留 legacy API/转发层，以新行为更新受影响测试，不删除有效回归验证掩盖失败。
2. 测试根写隔离、共享 part 幂等及类型冲突、条目直接引用、授权范围/转授/撤销、Permit/来源限制、版本冲突和旧实例清理；按本次新增语义补测试。
3. 测试无 UnitFile 命令运行、同 owner/同声明多实例、创建/Claim 回复丢失、取消晚到请求、容量失败、逐阶段失败清理、公平 Pending、自举和 Ready/租约寿命分离。
4. 宿主模型之外执行 accept/product/system-fault 的真实 QEMU 验证，核验实例退出、能力与租约回收；无法执行时准确记录环境障碍，不能以静态源码断言冒充通过。
5. 完成图和架构边界检查：api 无运行时依赖，image 无执行库/client 依赖，Control 无业务实现依赖，无新泛用转发壳、没有巨大 Assembly 业务 switch。

现有验证入口：

```sh
cargo check -p programs --all-targets --offline
cargo check -p system-api --target x86_64-unknown-linux-gnu --offline
cargo check --manifest-path crates/image/Cargo.toml --target x86_64-unknown-linux-gnu --offline
cargo test --manifest-path crates/schedule/src/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline
sh programs/src/system/identity/book/test-host.sh
nu scripts/qtest.nu --package kernel --scene accept
nu scripts/qtest.nu --package kernel --scene product
nu scripts/qtest.nu --package kernel --scene system-fault
```

受影响宿主套件位于 programs/tests：system-shape、startup-authority、control-api/rpc/query/lifetime/instance、operator-api/client/connection/handoff/watch、identity-api/rpc/publication、loader-api/client、interface-marks、publication-admission、capability-import、login、shell、terminal/terminal-connection、router-handoff 等，按实际改动迁移名称与覆盖后执行。基础 wire/ipc/resource/execution/mold 套件仅在相关行为变化时扩展验证。仓库明确不以 cargo fmt --all 为验收，也不要用其大面积改写无关代码。

## 文档约束

用户要求 docs 只保留本轮要用的文档。本文件是唯一保留项，实施进展、实际测试结果、限制和最终结构更新在此，不重新创建历史/阶段/重复设计文档。不要把未执行的检查写成通过，不要自动提交、推送或发布。

## 当前实施与验收（2026-10-12）

本轮迁移和真实整机验收已完成；本节取代此前阶段性记录。此前只看 embedded scene 返回成功的结果不计入本轮完成证明：子任务失败或 System 被级联终止时，旧工具可能仍显示通过。

已实现：

- 48 个 UnitFile 只组合 Loader、Control、Identity、Operator、Supply 的纯配置；54 个 Package 独立登记镜像，8 个 Deployment 显式选择入口、启动项、资源与 Target。cat/emit/upper/fail/spin/workers 的六个空 UnitFile 已删除，保留镜像、命令定义与 Supply 绑定。
- Assembly 统一 JobId、WorkQueue、Construct/Lifecycle/Phase/Command/Completed 消费与实例安装回执。Control 保留真实实例生命周期事实；CreateId 按原生调用者和实际构造入口隔离，重放、查询、取消与终态清理归同一作业账。
- Operator 使用实际 Entry/Binding revision、独立授予入口、范围及操作约束。System 建立共享目录，程序持有局部能力；描述符只读取一次并保留得到的能力。路由接收 Control 中真实的 task/team 坐标，可用同一 prefix 机制表达 `uit/{team}/{task}`、服务目录或其他布局；每项准备在自身安装回执保留坐标，失败不会阻塞其他任务。Control、Identity、Loader 的真实入口使用同一资源准备与退休机制。
- Supply 接入 initrd 镜像、Hub 清单与 Shell 命令资源。提供方保留共享根，每个租约持有独立私有根；失败清理保留回执，关停先结束租约和 provider，再释放来源及启动资源。Operator 发布尚未发送成功时只撤销本次交付副本，保留原提供方根和已有绑定。
- Account 只选身份与创建 Shell；身份导出核验声明、原生调用者、身份子树、实际 Account/create Entry、版本与资源来源后写入 Identity 关联。Login 保存父任务签发的启动上下文到 Shell 退出，Shell 对命令走 Image + Loan + snapshot，固定 User。
- 旧 Publication 的15个实现文件、Control Publication API/client、Loader publication 和 Unit publication 共19个旧源码已删除；旧 Loader Build RPC、旧配置字段与测试构件依赖已迁移。本地 bench 的 crate-private Loader 构造辅助不属于对外 RPC。
- Ready 使用单向交付，避免 System 接受 Ready 后撤销等待孔与对端对称握手之间的竞态。能力定位仍使用 Mark，同时验证实际 owner、vestor、种类、权限和存活；没有扩大 Spawn ABI。

已执行的有效验证：

| 检查 | 当前证据 |
| --- | --- |
| programs 全目标离线编译 | 通过，仍有 unused/dead-code 警告 |
| image 宿主模型 | 10/10 通过，含构造引用递归闭包 |
| 架构边界 | system-shape 18/18 通过 |
| 构造授权边界 | startup-authority 6/6，含满容量幂等、交付/撤销失败保留回执 |
| Identity 客户端与语义 | identity-rpc 13/13；Identity book 16/16 |
| 宿主套件 | 最终32套共282项通过；另行执行 Identity book 16项通过，合计298项 |
| product 登录脚本 | 最终真实通过：错误密码、取消、两个不同 Shell 保持同一 principal、终端恢复（2.62s） |
| product Shell smoke | 最终真实通过：Image 构造、双命令管道、1 MiB 传输、暂停/恢复/取消、前后台切换（11.65s） |
| accept | 最终通过：入口和20项探针明确成功退出，使用原3s跨任务等待窗口（5.33s） |
| system-fault | 最终通过：真实双实例、原编号重放、所有者隔离、准备/回收失败、领取超时、身份引用、容量、版本与服务故障回收（9.29s） |

最终全目标 programs 编译、纯宿主 system-api 编译和差异检查通过。构建仍存在 unused/dead-code 警告；这些警告未作为完成证明，整机以原生成功退出日志及交互脚本成功结果为依据。没有保留未实施的必做项，也没有增加多用户认证或多终端 UI。

运行链路：app 组合 schedule 的资源和计划；Assembly 推进统一作业；Loader 构造真实任务，Control 保存实例事实，Identity、Operator、Supply 完成各自安装与清理。Login 经 Account 的 Unit 构造能力创建 Shell；Shell 从自身 Supply 取得命令镜像，并通过 Image 构造能力运行命令。运行镜像不要求 UnitFile。

验收工具现在检查入口任务明确成功退出；accept 还要求20项探针各自成功退出。交互脚本必须写出成功结果，编译失败直接传播。实际原生日志是整机证据，不以源码断言或宿主模拟替代。
