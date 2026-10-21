# System 本轮重构方案与验收

状态：此前检查点和验收范围保留在下文。当前派生生命周期增量已按本轮限定范围完成实现、自查、实跑及收尾记录：首次交付、Forget 与父关闭子图的七个新增内核回归通过；最终验收暴露的已封闭引用无法释放问题已修复，新增宿主回归有修前红、修后绿。五个场景和四 Hart race 均重新通过，宿主 46 项与 workspace dev/release、kernel tests 检查通过。健康集仍为 117 passed、1 个缺 initrd 哨兵失败，不称全部测试全绿。本增量没有独立复审，不沿用首检查点的 DeepSeek 批准。本检查点以本地分支 checkpoint/sub-pole-lifecycle 提交封存，后续修改另作提交；没有推送或合入共享分支。

后续状态：保留授权沿既有 Accord/Grant 链委托的第一笔已实施、实跑并经新的 DeepSeek reviewer 独立静态复核，无阻塞项。健康集 118 passed、1 个缺 initrd 哨兵失败，宿主 46 项、三项构建检查、五场景与四 Hart race 通过。没有新增调用、确认凭据或全局持有根；恢复方的生产装配及 Driver 迁移仍未实施。

## 总体边界

- UnitFile 是声明式装配配置，由各功能模块的纯配置组成；模块拥有自己的机制和状态，Assembly 组合操作，schedule 执行计划。
- 创建按声明配置的实例与直接运行镜像共用构造机制，但直接运行镜像不要求 UnitFile；程序内部原生线程继续使用现有 Spawn。
- 可执行镜像资源与 UnitFile 分别登记。部署选择、运行依赖、打包依赖分别处理；镜像来源不决定执行特权。
- Control 拥有实例与生命周期，Assembly 拥有作业、游标和安装清理记录。Unit 名称与运行实例分开，同一声明或所有者可创建多个实例。
- Account 负责身份选择和会话创建；Shell 的命令目录、命令镜像获取和缓存归 Shell 所属提供方。Account 声明不包含 workers、Hub 或命令镜像清单。
- 自举复用现有 schedule 的 Plan、Cursor 和 Resources，以可信初始根资源和明确依赖驱动，不另建执行器。
- Mark 继续用于能力定位，另行核验来源、种类、权限与存活；撤回启动描述页和为句柄编号修改 Spawn ABI 的提案。
- 重构协议允许不兼容，迁移所有调用方并删除旧协议和客户端，不保留兼容层。

## 资源授权与独立 Driver：功能基线与设计边界

本节合并资源 ABI 重构与 PCI、IOMMU、DMAC Driver 化要求。以下职责边界是用户指定的目标；
交付后保留来源依赖、分支撤销与资源关闭分离、外部访问未确认结束时保留必要帧持有的功能规则已确认。
持有与依赖结构及原语效果已确认，具体签名、权限接口及 Driver 恢复装配尚待后续审定，不把候选协议作为已批准实现。
现有 `programs/src/system/device/` 是迁出对象，不以“没有新增同名模块”替代 System 去硬件业务化。

### 当前代码检查点

以下是基线 `4fe28fe3` 的静态审计事实，不是新增主线已实现的声明：

| 现有部分 | 已有能力 | 尚缺的职责边界 |
| --- | --- | --- |
| `driver/pci/`、`driver/iommu/` | 可复用的硬件实现模块 | 尚不是各自独立的 PCI、IOMMU 程序，不能以目录存在作为服务化证据 |
| `service/dma/` | 由 Loader 构造独立 Team/地址空间；经 IPC 接收 BAR 能力及 IOVA；不读取控制者裸 VA | 只实现端点一侧的设备操作；控制、内存发布和停止确认仍在 System，尚非完整独立 Driver 协作 |
| `system/device/`、`system/supply/retention/` | 产品侧已有真实 IOMMU 编程、DMA 保留、发布及故障收口 | 硬件控制迁 Driver，DMA 安全持有归内存/资源层，故障协调迁 Hub；不能仅搬 trait 定义而仍由 System 持有运行状态 |
| `service/hub/` | 独立服务，已有设备账、认领、资源交付及主人探活 | 接入 PCI 发现结果、驱动绑定及安全故障协调；主人消失清格不等于硬件已停止 |
| `harness/probe/iommu/` | 同 Team 探针已有真实 DMA 正负验证 | 仍有共享全局交接数据及裸 VA；须替换为独立地址空间的服务装配与请求验证 |
| 当前板级及 QEMU 测试配置 | IOMMU 与具备自身 DMA 能力的 virtio PCI 端点 | 尚无独立 DMAC 的配置和验证对象，不能凭已有 virtio 队列宣称 DMAC 服务完成 |

产品侧端点服务与同地址空间 harness 是两条不同路径：
前者由控制者观察完成记录，端点服务不读控制者内存；后者依赖共享地址空间。
迁移必须分别保留已有安全判据，不用历史文档或某一条路径的注释推断另一条的实现。
内核的板级资源识别和初始授权不等于 PCI/IOMMU 服务；
边界验收检查实际职责及依赖，不以硬件词汇的文本零命中作为判据。

静态审计另发现停止执行者与 DMA 持有者同处 System 故障域：
任务退出的 `pend::doom` 会清空引用表，独立保留引用随之释放 Residency，
该路径没有硬件停止确认前置。现有证据不足以保证控制者异常死亡后的 DMA 持有安全；
不能把此静态缺口写成已通过故障注入观测到帧复用或设备越界。
新增合同必须覆盖停止执行者自身死亡，而不只覆盖端点服务死亡。

### 职责与状态权威

| 归属 | 操作及独立效果 | 状态权威与失败责任 |
| --- | --- | --- |
| System | 通用程序装配、依赖解析、构造、启动、任务观察与回收 | 实例与任务生命周期；等待提供方完成清理，不解释 BAR、设备页表、DMA 通道或硬件停止状态 |
| Hub Service | 接收发现结果、设备登记、驱动绑定、资源交付、停止接收新请求与故障协调 | 设备绑定及协调进度；硬件未确认停止时保持隔离，不假报可回收 |
| PCI Driver | 总线枚举、配置空间访问、能力发现、BAR 分配与设备配置 | 总线与分配事实；拒绝不能按硬件保护粒度隔离的分配，配置失败保留恢复责任 |
| IOMMU Driver | 设备地址空间、上下文、I/O 页表、映射、失效与完成确认 | 已发布的设备翻译及同步进度；确认完成前不得解除必要帧持有 |
| DMAC Driver | 实际 DMA 控制器的通道、描述符、队列、启动、停止与完成确认 | 对应控制器的传输状态；未确认停止不得释放仍可能被访问的资源 |
| 设备专用 Driver | 设备协议、请求提交、中断与完成处理，使用获授的 MMIO 和 DMA 资源 | 该设备请求与队列；不得扩大寄存器范围或绕过 IOMMU 授权 |
| 内存与资源层 | 精确范围、授权、引用、DMA 所需物理帧持有与受约束的 PA 交付 | 范围、权限及帧持有事实；不依赖设备 Driver 的正常返回或析构释放安全关键持有 |
| Kernel | Task、Space、Flat、Nest、View、Pie、IPC、授权与内存生命周期机制 | 通用机制的不变量与回收；不实现 PCI、IOMMU 或 DMAC 服务 |
| Harness | 场景装配、启动 Driver、发请求、注入故障与断言结果 | 测试过程及判据；不代替服务实现硬件操作或故障恢复 |

Hub 是现有用户态服务，不是 System 内的设备子系统。System 可以按通用声明启动这些程序，
但不增加硬件专用配置字段、设备业务枚举或第二个设备管理者。
当前由 System 路径解释 DTB 并生产设备清单的职责也纳入迁移；
System 只按通用配置转交资源，不保留硬件清单生产业务。
Hub 的现有设备账和认领不等于已完成新增的驱动绑定与故障协调。
已有 `driver/pci/`、`driver/iommu/` 硬件实现按实际职责复用，不为目录形状整体搬家。
有硬件实现模块不等于已有独立服务；独立程序须有真实入口、自己的 Task、地址空间和授权集合。

DMAC 对应实际存在的独立 DMA 控制器。设备自身具备 DMA 能力时可由设备 Driver 直接使用，
不强制经过 DMAC。是否存在可验证的独立控制器，须用目标板描述及硬件证据确认；
没有硬件时不建立空服务，也不把 virtio 队列当成独立 DMAC 的实现证据。

### 资源与授权的功能边界

资源操作按 Hole、Pole、Nole、Tole 分开，Pie 只处理授权关系及查询；
Memory 的来源引用、CPU 映射与内存转移保持独立。

| 调用域 | Class | 操作 |
| --- | --- | --- |
| Hole | 5 | Unseal、Seal、Push、Pull、Peek、Wait、Ring、Hush |
| Pole | 10 | Unseal、Seal、Open、Shut、Wait、Ring、Hush |
| Nole | 11 | Unseal、Seal、Wait、Ring、Hush |
| Tole | 12 | Unseal、Seal、Attach、Detach、Await |
| Pie | 7 | Accord(4)、Narrow(5)、Revoke(6)、Inspect(8)、Collect(9)、Origin(10)、Forget(12) |

Class 9 不分配，其他未声明 class/slot 一律拒绝。四种资源使用各自紧凑 slot 表；
Pie 移走的 slot 0、1、2、3、7、11 不复用、不设别名。各资源有自己的错误词汇、
用户态封装和内核入口；Tole 的事件源失败不借用某个资源的通用错误全集。
Hole/Nole 的信号操作不接受无意义的 bit，Pole 拥有自己的信号位 API。

Accord 派生整份资源授权。Origin 只判断两枚 Pie 是否指向同一资源，不查询来源偏移或祖先。
Revoke 的 `dst = TaskId::SELF` 表示本地撤销，仅此参数使用零值的当前任务语义；
远程撤销保留直接授权关系检查。Forget 移除合法中间节点、保留合法后代；
父节点失效时在修改前拒绝，不重新挂接、不顺带销毁后代。
Narrow 维持收窄当前授权及其已有映射的语义，不追溯收窄已授后代。

精确 BAR 的流程是：从来源取得指定范围的引用，创建以该范围为自身边界的 Pole，
再通过普通 Accord 交付整份资源。创建、授权与映射是三个独立效果；
不能只删除创建检查、把子资源伪装成私有根，或在 Grant 中保留隐藏窗口补足边界。
创建失败必须回滚本次引用、授权登记、映射与物理持有，不重复消费私有来源的一次性资格。

页对齐后的 CPU 映射范围也必须可安全交付。若同一保护页覆盖其他主体的寄存器，
仅记录 BAR 的字节长度不能形成隔离；必须拒绝该分配，或先裁决明确的资源合并规则。
本轮建议采用保守拒绝：缺乏保护页独占证据时不交付，不引入隐式资源合并。
小于一页不自动证明泄露；验收要检查实际授权的整页及分配关系，
也不能仅以 BAR 长度或向上取整后的页数证明没有其他资源。
创建子资源不能复制父来源的全部映射登记；Seal 按资源清理依赖，
Revoke 按授权分支清理，撤销一个分支不得误伤兄弟分支或其他资源。
关闭同步未确认时保留身份、必要持有及重试责任；父自身没有映射、子图尚欠清理时仍可推进重试。

多设备分配遵循已确认规则：aperture 的范围管理权与各 BAR 的使用授权分开。
允许同一管理者分配多个互不重叠、可独立撤销的 BAR 资源，
不能因交付第一根 BAR 就占住整个 aperture 的唯一 heir；重叠范围不能同时形成互斥使用承诺。
这不是批准直接移除 ONLY 或放宽现有授权形态检查：
子资源形态跟源，但各子资源使用自己的移交槽，不占父的单一 heir；
范围分割与使用授权按下文规则分别校验，内核不引入 PCI 专用规则。

### DMA 发布、持有与故障回收

DMA 合同必须分别表达以下效果，不能由“知道 PA”推导出设备访问权：

1. 核验可用于 DMA 的内存资源、范围与读写权限。
2. 取得明确范围的帧持有；持有期间不得提前回收或重新分配。
3. 向获授权的一方交付该范围的 PA 信息，不提供无条件物理地址访问。
4. 授权具体设备对该范围的访问并发布 IOVA 映射。
5. 停止新请求，停止设备发起访问，撤销翻译并等待失效同步及硬件状态确认。
6. 确认不再可达后，才释放映射和必要帧持有；确认失败保持隔离并保留清理责任。

发现设备、分配 BAR 与开启 Bus Master 是不同效果；
不能因完成 BAR 落位就默认设备可以发起 DMA。
帧持有、设备翻译约束及独立恢复权限必须在允许硬件访问之前就绪；
部分发布失败同样进入停止与确认流程。
CPU 映射或 Pie 分支已撤销不等于设备已停止，也不能单独触发安全关键帧持有的解除。

现有 Memory Traverse 只作为待审计的机制，不预定为最终通用 DMA API。
若选择 Frame Lease 或其他持有机制，其事实归内存与资源层，
不在 System 中保留设备内存持有业务模块。只读诊断 Pole 可以观察状态，
但不能代替帧持有、PA 读取授权或设备访问授权。

故障路径由 Hub 协调：停止新请求，调用独立 Driver 控制路径禁用 Bus Master、必要时复位，
再由 IOMMU 控制路径撤销翻译并确认。停止确认前不能释放帧；
无法确认时隔离故障设备及相关帧，不以服务退出作为“硬件已停”的证明。
PCI Driver 自身死亡时仍须有具备独立权限、可以实际执行的恢复控制路径；
IOMMU Driver 死亡也不能使帧持有随其消失。恢复职责留在 Driver 层，
不能迁回 System，不能把关闭整机当成单设备故障处理。
恢复路径的执行方、权限范围及自身故障时的保守行为，在硬件访问发布前审定。
所需 ECAM、设备寄存器与 IOMMU 恢复权限须在故障前取得；
不能依赖故障发生后再向已死亡的服务申请。
持有责任不能与 PCI、IOMMU、设备 Driver 或恢复执行者的普通任务清理绑定：
即使恢复执行者也死亡，缺少停止证明时仍保持设备及相关帧隔离，不自动回收。

### 合并后的工程依赖

| 阶段 | 工作与输入 | 完成证据 |
| --- | --- | --- |
| A Driver 边界审计 | 核对硬件实现、服务入口、Hub、System、Harness 与目标硬件 | 逐项迁移清单；库与独立程序明确区分；已有 System 设备业务有迁出目标 |
| B 精确资源模型 | 明确范围、创建资格、授权派生、依赖与关闭责任 | 精确 Pole 的范围、权限、失败回滚及最终回收测试；驱动不能访问范围外页 |
| C Driver 独立化 | 复用硬件实现，分离服务入口和协议适配 | 各服务真实独立 Task/地址空间；仅经 IPC 与授权资源协作，无共享全局 VA |
| D DMA 资源合同 | 明确帧持有、PA 交付、设备访问、映射发布及停止确认责任 | 越权与提前回收负证；同步失败保留持有；恢复权限不依赖已故障服务 |
| E Hub 协作 | 接收发现结果、绑定、分配、发布与故障协调 | 独立 Driver 间授权交付、隔离及恢复闭环；System 无硬件业务 |
| F ABI 收敛 | 资源调用分离、Pie 授权收敛、全调用方迁移及旧路径清除 | 编解码及未知号拒绝测试；源码、测试、当前文档无旧 API 或兼容分派 |
| G 全链路验证 | 跨地址空间 DMA、服务故障、停止失败与净回收 | 正负 DMA、隔离及回收的真实硬件路径证据，不只宿主桩或编译通过 |

当前在 B：精确子 Pole 的创建、分区使用与结清首检查点及本轮限定的生命周期增量已收尾；
system-fault 验收回归已修复并重跑五场景、四 Hart race，不继续扩范围或开启下一阶段。
D 的跨任务死亡持有/停止确认尚未实施；C、E、F 最终收口和 G 不能由本批通过代替。

阶段字母不是允许提前发布硬件访问的流水顺序：
A 先界定职责；B 与 D 的功能合同先于最终硬件发布。
C 的硬件实现/测试协调分离和独立程序骨架可并行推进，
但跨地址空间的真实 DMA 发布须通过 B、D 的安全门；E 依赖该门及可用的 Driver 接口。
四资源 ABI 的新增入口可独立推进；F 中删除窗口授权的部分必须等 B、D 成立，
不能用临时接口的成功宣称最终架构完成。

`AccordWindow` 在精确范围、授权、依赖和 DMA 持有闭环得到证明之前保留实现；
证明后迁移探针、产品 DMA、供给适配、资源封装及全部测试，再彻底删除。
最终文档改写为当前模型，不保留旧接口说明或迁移兼容记录。
Git 历史不改写。内部资源类型擦除与 Task 退出统一清理可保留，
但不恢复通用资源业务分派或授权域的资源操作。

### 验收边界与待裁决项

必须覆盖：独立地址空间中的真实 DMA 成功、范围外访问失败、BAR 保护页隔离、
普通授权分支撤销不伤兄弟分支、父资源失效后的子图清理、PCI/IOMMU/设备 Driver 故障、
独立恢复路径、硬件确认失败不回收、可靠确认安全后最终帧和映射回到基线。
失效或硬件停止始终无法确认的场景继续禁止复用，不以重试次数换取回收。
停止确认必须有实际硬件状态证据；同地址空间测试、库函数调用、Task 已退出或超时本身均不能代替。

静态审计确认的覆盖差距：

| 验收命题 | 当前证据及缺口 |
| --- | --- |
| 跨地址空间真实 DMA | 产品已有独立端点服务正证；同 Team harness 不能作为独立 Driver 证据 |
| 访问拒绝与隔离负证 | 现有探针有相关负证；迁移后须在独立地址空间服务链上重证 |
| 保护页覆盖其他资源 | 当前代码未形成拒绝或合并规则，当前 QEMU 场景未提供这种设备；先用受控布局测试拒绝判据 |
| PCI/IOMMU Driver 死亡 | 尚无独立服务及相应故障场景，不能用端点服务死亡替代 |
| 恢复执行者自身死亡 | 无独立路径及验证；必须证明持有不随退出消失，不能靠正常析构保证 |
| 确认安全后的最终回收 | 保留既有净回收断言，并补跨服务关闭且可靠确认安全后帧、映射与残留持有回到基线的证据 |

已确认：派生 Pole 首次 Spawn 交付后仍保留来源依赖；使用权撤销不等于帧可以回收。
只有明确参与外部访问的持有需要停止确认，普通内存退出照旧回收，
不按任务名称或 Driver 身份对内核清理作特判，原私有整根交付语义不因此默认改变。
aperture 管理与多 BAR 使用授权的关系及 ONLY 的对应规则已确认，见下文；
不能据此任意移除形态检查，也不沿用只能交付一个子资源的旧候选。
DMA 持有和独立恢复控制方须满足上述失败合同；具体原语、最终 PA 接口与恢复装配仍待审定。

### 已确认结构与使用规则

以下结构及两组分区使用规则已获用户确认，资源终止、物理持有与控制绑定三项评审条件在下文明确；
具体签名尚未通过，当前先核查现有生命周期机制，不预置新的 CPU 清理对象：

| 事实 | 载体与所有权 | 生命周期边界 |
| --- | --- | --- |
| 资源范围与来源控制 | 每份资源自己的来源授权记录；规范范围一处拥有，派生记录无私有创建资格 | 不因普通引用仍在就永久强持已撤销来源；范围元数据不充当物理持有 |
| 来源依赖 | 父记录保存分支索引与子记录弱引用，子记录强持父记录 | 子记录的寿命须覆盖真实授权、View 及未结义务；token 退表不是删除依赖的条件 |
| 使用授权 | 既有 Grant 及 Pie 关系；权限为创建时快照 | Narrow 不追溯后代；Forget 只改授权节点，不重新计算范围或洗白权限 |
| 交付后的控制 | 目标 Space 中的 ViewEntry 身份、controller、source 与来源侧空间登记 | 登记先于交付消费；依赖须明确绑定到交付后控制，不能靠已退表 token 判断存活 |
| 未完成的 CPU 清理 | Nest 现有页表、失效状态、退休资源与占用票据 | CPU 翻译可靠失效前保留所需资源；任务退出或尝试重试不构成结清，Forget 不等于来源关闭 |
| 外部访问的物理持有 | 复用 Residency/Flat 的独立保留，不只依赖使用任务的引用表 | 保留须覆盖设备仍可能访问的周期；参与方全部死亡也不自动解除，具体承载按已定位缺口审定 |
| 停止确认与解除授权 | 当前义务及发布代际下的授权门，由获授权 Driver 核验并提交确认 | 旧代际确认不能解除新发布。权限根在发布前确定；全部权限丢失时保隔离，不凭空补发 |

提交前失败返回原 token 可重试的 Busy，且不产生业务副作用；
摘除 token 后的收尾由控制记录或 Nest 票据接管，不要求使用失效 token 重试。
Seal 保留 Pie 的查询和补关身份；父自身无映射、子图仍欠清理时仍可推进。
授权节点移除或析构不作无条件级联；CPU 地址空间的撤下与同步责任仍归 Nest，
须先核查其既有生命周期，不从额外清理载体倒推新的析构规则。
所有显式退出、撤回、交付控制终止与合法 Forget 的事件必须分别闭合。

资源控制整体终止也属于显式触发集：最后一份有效控制释放且没有合法承接者时，
进入关闭路径，未结清时继续保留所需资源。Closing 描述未结清阶段，不要求另建载荷或状态枚举。
Forget 和消费式交付先在同一事务中移交控制绑定，原 Pie 的移除或析构不单独触发资源终止；
不得据此杀掉合法改挂的后代，也不得以 Arc 计数代替控制是否结束的事实。

规范几何自规范 source 一次生成，不设可独立修改的第二份范围权威。
普通 Hold 不新增独立物理持有，撤回后不能仅因普通引用存活就继续 pin 帧；
资源自身 source、现有活动 View/Nest 同步退休所需持有，以及外部登记的 Residency 各按真实生命周期承担物理持有。
待关闭或外部仍可达的分配身份不能因弱依赖账项被压缩而提前重新分配；
CPU 关系及退休资源由 Nest 保留，设备访问由独立物理保留覆盖，分别核对其真实生命周期。

历史索引到 ViewEntry 身份、controller、source 的控制绑定，在创建、Forget、
消费式交接三处同事务更新；控制终止也同事务解除绑定。
历史 token 只作索引，授权判据取对应控制记录当前状态，不取决于旧 token 能否在表中找到；
子资源的权限仍为合法创建时的快照，不因父后续 Narrow 重新计算。

两组已确认的可观察规则：

1. **父范围的新使用**：派生段仍被占用时，不允许对覆盖这些段的父范围建立新使用。
   Open、Accord 仍操作整份资源，发生冲突就拒绝，不能隐式裁成剩余范围；
   带范围的 Acquire 核验请求段，管理者可继续分配未占用段。
   已存在的旧映射不因此自动撤销；旧引用建立新映射仍须经过同一使用判据，不得绕过。
2. **子资源形态与并列范围**：子形态跟源，形态不一致的请求拒绝；
   同父的并列子资源不重叠，需要共享同一范围时对同一子资源 Accord，
   不重复创建多个重叠资源；祖孙嵌套不当作并列冲突。
   这不决定派生数量，不把派生操作塞回父的单一 heir 槽。
   只有真正结束占用后才能重用范围；Closing、未完成同步及外部尚可达的范围继续占用，
   普通旧引用保留元数据不等于范围永久占用。

首检查点限于精确资源、来源依赖、CPU 关闭和交付载体；外部持有登记及 Driver 接入另设检查点。
首检查点通过不表示当前 DMA 持有缺口已修复，不允许据此删除窗口授权或宣称新增硬件发布安全。

### 已确认原语清单

原语按独立效果划分，不把内部准备、谓词或账项维护机械提升为公共调用。
既定四资源/Pie 调用名与槽位不重审；下表中持有机制的具体 Memory 入口及签名在当前签名关审定。

| 效果与候选名 | 入口归属 | 提交与失败责任 |
| --- | --- | --- |
| 精确资源派生 `derive` | 内部，由最终 Pole Unseal 进入 | 核父分支 FETCH、VEST、范围、形态与并列占用；登记和创建同事务，不消费父私有资格，失败全回滚 |
| 新使用判据 `check_use` | 内部，被既有资源与 Memory 路径复用 | 校验目标资源自己分出的子段及当前义务，不将子资源自身与父账撞成冲突；选段与占用登记原子提交 |
| 移除中间节点 `forget` | 既定 Pie Forget | 同一来源链、新父当前存活；所需权限覆盖不足则修改前整体拒绝，绝不重新计算子权限或部分复制 |
| 分支撤销 `revoke` | 既定 Pie Revoke，含 SELF | 分支精确，远程保留直接授权父检查；摘后清理由控制记录/Nest 票接管 |
| 资源关闭 `seal` | 既定各资源 Seal，Pole 负责其映射与来源依赖 | 属主检查、资源全量；保留 Pie 查询与补关身份，父无本地映射但子 pending 时仍可推进 |
| 交付控制移交 `hand_over` | 内部，被既有 Unit 构造/Spawn 使用 | 预登记目标 Space，消费前原子更新控制指定与绑定，不丢来源依赖 |
| 关闭推进 | 内部，复用既有控制记录及 Nest 清理机制 | 控制移交不丢责任；CPU 失效是否完成由 Nest 判定，物理来源寿命由 Flat 机制维持，不另设并行清理权威 |
| 建立外部持有 `acquire` | 确需用户态可达的 Memory 通用效果 | 核有效来源及保留权限，容量有界；登记关联范围、分配身份、当前代际及预置证据权/解除权，复用 Residency/Flat，不进入 Task 引用表生命周期 |
| 提交确认 `confirm` | 确需用户态可达的 Memory 通用效果 | 仅证据提交权可对当前义务、当前代际确认；旧代际或无权请求拒绝，仍持帧。Driver 核验硬件，内核不解释设备寄存器 |
| 解除外部持有 `release` | 确需用户态可达的 Memory 通用效果 | 当前代际有效确认及最终解除权同时满足才解除；普通 Memory Release 仍只释放 FlatReference，不隐式 Detach 或确认硬件 |
| 受限 PA 读取 | 现有 Traverse 是候选，最终句柄留签名关 | 仅读取有效持有授权覆盖的物理页信息，不产生设备访问权，也不解除持有 |

过渡期 AccordWindow 同样接入 `check_use`，按实际有效窗口核占用，不遗漏仍被保留的旧入口。
Acquire 的 Source、Reference、View 路径以及 Attach、Transfer、新映射发布，
都必须在各自提交边界核当前新使用条件；旧引用不得绕过。
Open、Accord 的整资源语义不改成子集，带范围的操作不隐式裁剪。
占用谓词、派生边增删与保留接管只是上述原语内部工序，不另外开公共 API。

PA 读取是受授权的信息读取，不单独视为建立新使用；
派生不自动剥夺原先合法保留引用的 PA 读取权。
但建立新的外部持有登记须核 `check_use`，Driver 发布新的设备映射须核当前范围、
持有义务代际及设备访问权。只有 PA 数字不能取得这些权利，
不能将“读取不是新使用”解释成允许用旧 PA 绕过新访问门。

每个外部持有登记只对应一个外部可达发布周期：
登记成功即保守标记可能可达，真实硬件发布在后，失败不得发布。
确认结束后不得再次用该登记发布；下一周期须新登记及新代际，不支持同项静默多轮发布。
周期内正常 DMA 请求不等于开启新的外部访问授权周期。
无人持解除权时保持隔离，获准观察者仍可看见义务，不能从空权限集合凭空恢复权利。

范围可重用的提交点必须同时满足 CPU 清理结清与外部义务全部结清，
不得先释放分区占用、再慢慢撤掉交付映射。
这两个条件独立：CPU 地址翻译归 Nest，设备访问的撤销与停止确认归用户态 Driver。
签名关须说明既有机制如何分别证明这些条件，并在改变范围占用前保持判据有效；
不能在锁外自行推断可重用，也不由此预设新的汇总账本或 DMA 恢复对象。
Creation 资格、资源控制存活、临时发布暂停/在途闸门是不同事实；
复用既有同步机制，不为同一事实新增平行状态，也不把可恢复的暂停当作永久撤销。

### 签名审查与现有生命周期核查（进行中）

当前签名草案未通过技术评审，不作为实施合同。先明确不变量，再检查现有机制；
只有定位到现有机制无法表达的具体缺口，才讨论补充内部机制。
独立 Close/CloseQueue、额外关闭槽、新增来源终止枚举及后台重试挂钩均不是已定结构。

CPU 生命周期的不变量：

- CPU 旧翻译未可靠失效时，相关物理帧、页表及地址空间身份不能被不安全地复用。
- 任务死亡不能消除内核负责的清理义务；待失效状态可以保留在现有内核对象内部。
- 重试只是尝试，只有可靠失效的结果才能放行回收；CPU 始终不能确认时继续禁止复用。
- 控制位置、授权节点与资源整体终止不同；Forget 的合法后代保留、Seal 的查询及补关身份不变。

静态核查已确认的现有机制：

| 环节 | 现有实现 | 结论边界 |
| --- | --- | --- |
| 发布前准备 | `Nest::commit` 调用 `Occupancy::prepare_cleanup`，为该关系身份准备 `CleanupNode` | 已有内部清理载体，不据此再增一套工作层账本 |
| 撤下后同步失败 | `Nest::withdraw` 将发布帧、表页、View 与占用票转入退休；`finish_invalidation` 失败时不取出退休资源 | 失败保留资源，只有 shootdown 成功才结算；重试次数不是安全判据 |
| 消费式关系撤回 | `Nest::dispose` 失败时将已有节点留在本 Nest；`retry_invalidation` 继续结算与收尾 | 消费调用方的凭证不等于丢失 Nest 内部责任 |
| 正常再次进入 | 环境调用、缺页处理及任务控制清理均有现有失效结算入口 | 不能把没有独立后台机制视为没有重试路径 |
| 任务退休失败 | `messenger::reap` 将任务交给既有 HUSKS；`bury` 在 `Task::retire` 失败时放回任务 | 子团队先于任务自身内存退休；失败仍保留 Task → heir/Team → Space/Nest 所有权链 |
| 最后持有者析构 | `Nest::drop` 在 close 或 ASID deallocate 失败时跳过 `NestState` 的析构；ASID 只在 shootdown 成功后归还 | 保留资源与最终完成清理是两回事，不能将此路径直接称为提前复用 |

新增 `kernel/tests/health/nest_exit.rs`，由 embedded-test 为每例启动独立 QEMU：

| 用例 | 动态结果 |
| --- | --- |
| 无故障的任务退出 | 走 Slay、pend 清理与既有 bury；目标帧释放，页表和物理分配计数回到基线，ASID 可再次分配 |
| 持续失效失败的任务退出 | 多次结算仍报 Shootdown，VA、物理帧、页表和 ASID 不提前复用；测试放掉 Task/Team 持有后，既有 HUSKS 仍保留任务与 Nest；恢复失效并调用既有 bury 后，帧、页表及 ASID 回收 |
| 绕过任务退休链的原始最后引用析构 | Nest 弱引用失效，但必要资源继续保留；恢复注入后，新空间的正常结算及空的 bury 不能回收旧状态。该例只证明隔离，不证明最终回收 |
| 不直接调用 bury 的运行槽退出与调度 | 任务通过正常 quit 退出，失败时资源保留；恢复后通过正常运行环完成退休，无须另一任务退出，帧与页表回到基线 |
| 无任务子团队的 Oust | 修补前返回成功并丢失责任；修补后失败保留同一身份及强持有关系，禁止重新 Spawn，多次失败不提前复用；恢复后完成退休，页表净回收 |
| 父任务退出时的空子团队 | 子团队同步失败时，原父任务及 heir 仍由 HUSKS 保留；恢复后任务及子团队退休，页表与物理分配回到基线 |
| Live 团队全部任务已退出 | 普通 bury 不推断团队终止，团队仍可再次 Spawn；显式 Oust 才关闭团队 |
| 成员已 prune、TaskMemory 尚在 | Oust 从既有 roster 找到真实已发布任务，先结算 TaskMemory 再关闭 Nest，后续原 bury 完成记账，无固定残留 |
| 关闭已成功、最后 ASID 归还单独失败 | 预先成功 close 后注入下一次 shootdown 失败；Oust 保留同一 owner/TeamId，恢复重试归还一次，重复退休及旧对象析构不二次释放复用中的 ASID |
| 旧 Team/Space 存活时 ASID 已复用 | 新空间注册真实 Space/View 转发等待；重试旧退休、析构旧对象后站点仍在，signal → Tole → wait 通知仍送达，最终资源净回收 |

净回收基线在全局 ASID 位图初始化之后取得；其首次 8192 字节分配来自
`BitmapAllocator::ensure`，不是退出泄漏。测试清理名册死项及观察用 Task 弱引用后，
保留严格的物理分配及页表净回收断言，不允许固定额外残留。

新增调度用例先复现了具体断点：既有 bury 只在 quit 中调用，
失效恢复后，普通运行环与时间片切换不会继续处理退休任务；另一任务退出才会触发回收。
最小修补将该动作从 quit 专属路径移到 `conductor::run::run` 的提交锁之前，
不改变 Nest/Flat 的持有与失效判据，不增加队列、账本或平行状态。
bury 仍在取出退休任务、释放链锁后才执行可能失败的物理退休；失败照旧放回既有 HUSKS。

故障夹具仅在 debug/test 模式下按 ASID 持续返回失败，再显式恢复；
注入模拟 shootdown 返回失败，不是实物 CPU 永久失联的验证。
新增调度用例使用真实运行槽的 OnHart 状态及正常 quit/调度入口，不直接调用 bury，
但未恢复执行用户指令；真实程序路径另由 accept、product、system-fault 验证。
本次修补不宣称覆盖完全空闲时的失效恢复，也不据此增加后台机制。

修补前核查定位到一条实际绕过 HUSKS 的调用路径：

- Unit Build 可以创建尚无任务的子团队；Oust 的 all_reaped 判据对此成立。
- Oust 在撤离前调用 cancel_staging，但无暂存项时不会触发 Nest 失效结算；
  随后摘掉父任务保存的团队，最后的 Nest close / ASID deallocate 落在析构中。
- 故障用例放掉观察用强引用，持续注入目标 ASID 的失效失败，再走同一核心 Oust 操作；
  结果为 Ok，clan 已不可查询，Nest 弱引用不可升级，HUSKS 没有接管。
  故障解除并继续新空间结算与既有 bury 后，目标 ASID 仍未复用、五张表页仍未回收。
- 用例保留“失效失败时 ASID 禁止复用”和“最终页表净回收”的严格断言。
  不把失去清理责任后的永久保留登记为正常通过；当时该回归明确为红，后续修补结果见当前检查点。
  这是核心操作的 QEMU 验证，不是用户指令执行下的 Unit ABI 端到端测试。

完全空闲路径的静态结论：

- fetch/wait 在 WFI 前后都不调用 bury；没有等待截止时间且不在停机协调中时，
  使用既有 WFI_FAR 时限，不为待退休任务另定唤醒。
- 内核态、无当前任务的 timer/soft trap 可以直接返回原内核帧；
  一次中断不等于必然重新进入 run。
- tally 在 bury 成功后才 exit，因此失败的任务退休会阻止 done；
  这保留了“没收完不能正常停机”的判据，但不能证明空闲时会继续完成退休。
- 尚未增加真实 WFI 的故障恢复动态用例；不将静态调用关系当作此项已验证。

已确认并实施的局部失败合同：Oust 在 Nest 关闭及 ASID 归还可靠完成之前，
不摘掉现有团队的可重试身份；失败由现有操作返回，恢复后重试同一身份。
不能只提前调用 close 后仍把可能失败的 ASID deallocate 留给最后析构。
先结算已 Reaped 但尚未退休的 TaskMemory 和持有子团队，并封住并发新发布，不能直接给 Oust 塞一次 close。
完全空闲时依靠哪种既有事件继续退休另列待决，不据此预先批准周期轮询、队列或新状态。

#### Oust 局部合同复核与实施约束

继续静态核查后的约束：

- Reaped 与内存退休不同。`TaskIdent::retire` 仍需撤下 TaskMemory 的 frame、stack，
  并交还 guard；先 close Nest 会使其后续 detach 返回 NoRegion。
  不以忽略 NoRegion、丢掉 TaskMemory 字段或放宽净回收断言补救顺序错误。
- `TeamLife::prune` 按退出收据清理成员，可能早于 HUSKS 完成物理退休。
  因此 members 为空、all_reaped 或一次成功 Join 均不能证明没有待退 TaskMemory。
  已发布且仍可升级的任务由既有 roster 查找，不另设待退成员表；
  现有 roster 全量快照在分配失败时返回空 Vec，不能直接把它用作“没有任务”的安全证明。
- `Task.heir` 才是子团队的强持有位置，TeamLife.children 只是生命周期关系。
  子团队的关闭责任须在持有它的任务退休前完成；失败继续保留原任务及 heir，
  由 HUSKS 保持这条所有权链。嵌套处理必须向后代推进，不能从团队再次递归关闭同一团队自身。
- Spawn、外域 Memory 构造操作和 Oust 使用目标团队的 operation，
  但它不是整棵子树的锁。PreparedTask 发布和 make_team 的父关系登记有提交时检查；
  make_team 登记 TeamLife 与 Task::adopt 登记强持有却分属两次提交，后者只检查任务 dead。
  不能据此宣称整个子树构造已与 Oust 原子互斥。
- 父任务死亡的 doom/cull 当前在进入后续退休之前对 cancel_staging 使用 expect。
  可失败的同步清理应归入仍保留所有者的退休步骤，死亡标记与级联停用不承担成功回收证明；
  不能改成忽略返回值来绕开 panic。
- ASID 同时用于现有 Space/View 等待键。提前归还 ASID 不能只看 PTE 已撤；
  还须先封住旧域操作并清理旧等待键，防止旧身份操作影响复用后的空间。
  这由工作层安排，不让 Nest 反向调用 messenger。

建议采用“先退休，后关闭”的顺序，保留现有 detach 的错误语义：

1. 在现有 operation 与提交边界下校验 Oust 权限、全子树 all_reaped；
   活任务或正在进行的操作仍按现有 Busy 判据拒绝，不代替调用者杀任务。
2. 一旦进入实际退休，在同一提交边界把目标子树置为现有 Doomed，
   不置 Ousted；关闭新发布入口，但保留 TeamId 和原强持有关系。
   各域的在途构造还须经各自 operation 收尾，发布须复核祖先终态。
3. 从真实任务所有权出发，先处理任务持有的子团队，再处理其自身 TaskMemory；
   对已退出但尚在 HUSKS 的任务使用同一退休动作，不从 HUSKS 摘走它们代替收尸。
   只有原 bury 路径负责每个任务一次性的退出记账。
4. 任务退休与暂存关系结算成功后，清理旧域等待身份，关闭目标 Nest 并可靠归还 ASID。
   使用已有关闭、失效和 ASID 归还状态，不新增第二份完成位；
   归还动作须在现有所有者互斥下只执行一次，不能仅靠原子 bool 的先读后写。
5. 上述动作成功后，才清收据、置 Ousted 并摘掉团队身份。
   中间任一失败均保留可重试身份和剩余责任，不恢复已经撤下的映射。

用户已确认的可观察语义：进入退休后的 Oust 失败是“继续补关”，不是“恢复可运行”。
复用 Doomed：失败后仍可查询并重试同一 TeamId，但不能重新 Spawn、Attach 或 Build 后代。
不选择失败后重新开放团队；它要求恢复已经撤下的映射和构造关系，不承诺为简单回滚。
仅 all_reaped 而没有 Oust/终止请求的团队不自动进入 Doomed，保留其现有再 Spawn 能力。

成员遍历已采用既有 roster 的无分配游标，TeamLife 与 heir 已归并为原子发布。
未发布 Space/Nest 构造回滚，以及非内核 Live 根团队的最后持有者退休仍未闭合；
新失败若只发生在最后 raw Drop，仍可能隔离资源却失去可达的清理对象，不称为成功回收。
不能将 TaskMemory、TeamLife 或 Arc 引用数中的任何一个单独当作总完成判据。
完全空闲恢复策略与 DMA 均未纳入此次局部设计。
本节核查时只更新合同草案，未修改运行时或测试；后续实际实施进展单独记录如下。

上一轮运行环修补验证：

- `nu scripts/qtest.nu --package kernel`：当时 81 passed、1 failed，退出 101；
  唯一失败是未提供 initrd 的既有 scene 哨兵，四个新增用例均通过，不称全绿。
  修补前新增调度用例真实失败，保留于 `target/nest-exit-schedule-verification/before-schedule-fix.log`。
- `nu scripts/qtest.nu --package kernel --scene accept`、`--scene product`、
  `--scene system-fault`：各 1 passed，退出 0，与健康用例及彼此串行运行。
- `cargo check -p kernel --tests --offline`、`cargo check --workspace --offline`、
  `cargo check --workspace --release --offline` 通过；release 仍有七条既有 dead-code 警告。
- `cargo build --workspace --offline`、`cargo build --workspace --release --offline` 通过。
- 修改的 Rust 文件逐文件 rustfmt 检查（skip_children）及 `git diff --check` 通过；
  未执行全树格式重排。当前日志在 `target/nest-exit-schedule-verification/`。

本轮边界回归验证：

- `nu scripts/qtest.nu --package kernel`：81 passed、2 failed，退出 101；
  分别为新增空团队 Oust 清理责任回归、既有无 initrd 的 scene 哨兵。
  上轮四个 CPU 退出/调度用例仍通过。日志在 `target/nest-exit-boundaries-regression/qtest.log`。
- 本轮只增加测试及核查记录，未再修改运行时实现，未重跑产品场景或 workspace 构建。
- 修改的测试文件逐文件 rustfmt 检查（skip_children）及 `git diff --check` 通过；
  该红色回归尚未修复，不提交或推送为已完成变更。

#### 当前并行实施检查点

- 生命周期 worker 的 Oust、父任务死亡与既有 HUSKS 退休链修补已集成；不新增清理队列。
- 主线程已将 TeamLife.children 与 Task.heir 的发布归并到一次提交，
  parent/owner 从同一个固定 sire 强引用派生，删除独立 Task::adopt 生产入口。
  过期但非空的 sire 弱引用拒绝，不再默默创建无父团队。
  两个原手工 adopt 夹具改走真实构造；新增父身份、终态拒绝和页表净回收用例。
- 资源调用域第一 worker 的 Hole/Pole/Nole 检查点已集成：class 分别为 5/10/11，
  各自拥有 Call/Fail/Result，Hole/Nole 无假 bit，Bit 定义归 Pole。
  原 worker 因上下文预算封存于 `1eaf47d7-04d1-4c2c-8dfc-fe0066ae7140.12004987`，
  已停止写入。Tole 及异步结果已接续集成，封存于
  `4c81c381-e97f-46bc-90dc-c256ec1be78d.12033235`；ToleFail 与 ToleEventFail 分开。
  旧 Mail 槽 32–40、公共通用类型和旧 Pie 资源操作仍临时存在，不能登记为最终 API 完成；
  最终须删除，不作为兼容层交付。AccordWindow 和 DMA 安全持有不以占位实现替代。
- resource/IPC 客户端检查点已集成于 `5d03dbea-1d67-4c90-9702-d0639c31dc68.12032286`；
  主线程再将 Pile、Sender/Reply/Rack/RPC 的事件来源改为 typed ToleEventSource，
  同步迁移相应宿主夹具。程序调用方由新 worker 接续，env/kernel 公共 Mail 清理由另一 worker 处理；
  前序 worker 已封存，不并行写同一文件。
- 团队原子发布与集成退休链均经独立 reviewer 静态审查，未发现阻塞缺陷；
  不将静态审查称为并发验证通过。
- CPU 检查点的 workspace dev/release check/build 及 kernel tests check 当时均通过，
  release 七条既有 dead-code 警告。当前客户端签名迁移中，workspace dev/release 已真实失败于
  programs 的旧条件/失败类型；未将局部 crate 编译通过称为当前 workspace 全绿。
- CPU 检查点 `nu scripts/qtest.nu --package kernel` 为 88 passed、1 failed，退出 101；
  Oust 原红色回归及五个后续退休边界用例均通过，唯一失败为无 initrd 的既有 scene 哨兵。
  最新日志：`target/refactor-asid-retirement-regressions/qtest.log`。
  定向 ASID 用例前的 86 passed、1 failed 保留于 `target/refactor-retirement-integration/qtest.log`。
  原子发布单独检查的 82 passed、2 failed 保留于 `target/refactor-publication-verification/qtest.log`。
- 资源 ABI 集成之前，`accept`、`product`、`system-fault`、`iommu-product`、`iommu-dma` 场景严格串行运行，
  各 1 passed、退出 0，日志目录 `target/refactor-retirement-integration/<scene>/`。
  后两项是原有 DMA 行为的回归证据，不是新持有接口或 Driver 化已实现的证据。
- reviewer 指出的“最后 ASID deallocate 单独失败”和“旧对象存活时复用 ASID 的等待键隔离”
  两项定向测试已由原生命周期 worker 补充、主线程集成并经 QEMU 通过，运行时未额外修改。
  新测试 scoped rustfmt、diff check 及独立 reviewer 断言复核均通过。
  发布/死亡及并发退休的真实交错测试仍属未覆盖项。

资源调用域的当前动态证据：

- 独立 reviewer 查出 HoleCondition::Signal 被底层通用等待入口拒绝；新增 typed 调用 QEMU
  先真实失败（返回 Denied，而非未响时的 false），记录在
  `target/refactor-resource-domain-regression/before-signal-fix.log`。
- 修补使用原 Hole Pending::Rung.count 判据，队列数据不算 Signal，保留既有 Pull 就绪；
  同步补齐 wait/ready/key 接纳、Ring 的 Signal 唤醒及 Seal 的 Signal 清理，不增加通知状态。
- `typed_resource_call_domains` 走实际 slot/寄存器解码及内核 dispatch，验证三域创建、
  错种类拒绝、Hole 数据与三值返回、通知计数逐次确认、Ring/Seal 转发、
  Nole 通知、Pole Open 双值返回及 Shut/Seal，并严格检查站点/页表/物理分配净回收。
- 修补后最新健康检查 89 passed、1 failed，退出 101；唯一失败仍是无 initrd 的 scene 哨兵。
  日志：`target/refactor-resource-domain-regression/qtest.log`。
  独立 reviewer 复核无新缺陷；该例未覆盖真实阻塞 Task 的调度恢复或用户侧生成返回解码器。
- 主线程 `cargo test -p env --target x86_64-unknown-linux-gnu --offline`：31 tests passed，
  signals.rs 一条 unused-import 警告；workspace release check、dev build 通过，
  修改的核心/测试文件 scoped rustfmt 及 diff check 通过。
- 该中间 ABI 检查点尚未完成全部客户端迁移，尚未将此前五个场景结果当作新调用域产品验收。

Tole 与客户端接续验证：

- 主线程扩展 typed_resource_call_domains，实际走 class 12 创建、附加、Pending、
  成功来源三值返回、来源 Seal 后 ToleEventFail::Dead、解除、关闭及操作级 ToleFail::Dead；
  保留严格净回收断言。最新 QEMU 仍为 89 passed、1 个既有 scene 哨兵失败，
  日志 `target/refactor-typed-tole-integration/qtest.log`。
- typed Tole 经独立 reviewer 审查未发现 P0–P2 缺陷；真实阻塞/重排队、原截止时间及并发失权
  的交错验收仍未完成，不用 POLL 与转发用例替代这些证据。
- 主线程 env 宿主全套现为 34 passed；kernel tests check 通过。
- resource/IPC 的 dev/release 检查通过，独立宿主配置全部实际运行：
  capability 19、RPC 20、rack-reader 3、session-establish 23、session-exchange 17，均退出 0。
  相同命令使用各自 Cargo.toml 与 `--target x86_64-unknown-linux-gnu --offline`。
- pend 宿主夹具补齐真实 new_grant 的 window 参数，并按实际新域更新未分配调用号边界断言，
  未删除失败路径断言；原 E0061 编译断点已消除，37 tests passed，仍有三条既有 race-scene cfg 警告。
- OpenAI agent 因额度中断后的两个实际检查点已拉回，未丢弃其未完成源码。
  主线程补齐公共 env 拆分，删除 MailCall/env::mail 及通用 Mail 公开类型；
  Oversize/PullOutcome/Bit 保留在各自资源类型位置，未恢复兼容调用。
- programs 全部生产目标曾恢复 workspace dev check 通过，env 宿主 34 passed；
  随后移除 Pie 资源槽 0–3 与 UnsealArgs，当前旧内核用例的 typed Pole 调用迁移尚在进行，
  不把前一个检查点的通过结果称为当前完整构建通过。
- 首轮实际跑完 programs 的 33 套独立宿主配置：27 套通过、6 套夹具编译失败。
  用户指定 DeepSeek V4.1 Flash 后，六套修补已合入并由主线程逐套真跑通过：
  control-rpc 3、operator-connection 6、operator-watch 5、shell 14、
  startup-authority 6、terminal-connection 5；没有删除原有断言。
- 磁盘曾仅剩 7.5GB：只删除当前工作区内 39 个已确认 Git 忽略且无跟踪文件的测试 target，
  回收约 5.3GB，恢复约 13GB 可用；根 target 的 QEMU 日志、源码及 agent 检查点均保留。
  其他工作树缓存尚未删除，等待用户明确授权。
- 后续宿主测试统一使用 `CARGO_TARGET_DIR=$PWD/target/host-tests` 和 `CARGO_INCREMENTAL=0`；
  六套共用缓存首次验证仅 292MB，避免独立测试目录各自重复编译依赖。
  新 DeepSeek agent 仅改源码/静态审查，不运行构建。
- 内核私有通用资源分发及旧拼写由 DeepSeek worker 继续收口；资源/IPC 的 typed 所有权迁移
  另由 DeepSeek reviewer 独立审查。资源 ABI、精确子 Pole、DMA 持有及 Driver 主线尚未整体完成。

公共兼容移除与内核直调检查点（后于上述中间验证）：

- DeepSeek 内核检查点 `b15830ad-d371-4291-83c2-5030e6533875.12189836` 已集成。
  私有 MailCondition/MailFail 及 Mail 的业务分发已移除，Hole/Pole/Nole 入口直调资源机制；
  Mail 保留身份、生命周期及授权归属。Tole 按 typed source 获取许可、等待键及就绪状态。
- Pie 资源槽 0–3、UnsealArgs、公共 Mail 类型及全部兼容入口已移除；原 Mail 槽 32–40 拒绝解码。
  Pie Release/Same/AccordWindow 仍是后续批次的真实依赖，不称为最终 Pie 分区已完成。
- 主线程真实验证 workspace dev/release check、kernel tests check 均通过。
  合并后修正一处测试的 PoleFail 路径，以及非法 PoleCondition 编码断言：
  Signal 的 wire 值是 `3 + bit.index()`，不是裸 bit 下标；非法前缀及上界均继续拒绝。
- env 宿主 34 passed；共享 target 下重跑 pend 及全部 resource/IPC/programs 独立宿主配置，
  共 39 套 manifest 全部退出 0（含 programs 全部 33 套），pend 37 passed。
  仍有既有 race-scene cfg 与部分宿主夹具 dead-code/unused 警告。
- `CARGO_INCREMENTAL=0 TRACE_OUT=target/refactor-direct-resource-dispatch nu scripts/qtest.nu --package kernel`
  实跑 89 passed、1 failed，退出 101；唯一失败为未提供 initrd 的 scene 哨兵，
  全份保留于 `target/refactor-direct-resource-dispatch/qtest.log`。
- 完整迁移后的 `accept`、`product`、`system-fault`、`iommu-product`、`iommu-dma`
  严格串行实跑，各 1 passed、退出 0；现场目录 `target/refactor-resource-client-scenes/<scene>/`。
  这些是新调用域下的现有产品回归，不是新增 DMA 持有或独立 Driver 的实现证据。
- 资源专属 helper 归位检查点 `05650a62-d69c-413d-b3ee-5380f0325f2f.12388704` 已集成：
  Pole 创建/映射及五阶段失败注入归 pole.rs，usable 归 gate.rs，共享 Seal 归 seal.rs，
  Seal 参数不再接受无调用方的 None。Pie 保留纯授权操作。
  主线程编译查出两处 Accord/AccordWindow 漏迁 usable 路径并修正；原失败记录在
  `target/refactor-resource-helper-relocation/qtest.log`，未以静态解析替代编译。
- 修正后的 workspace dev/release、kernel tests check 均通过，release 仍有七条既有 dead-code 警告。
  QEMU 重新实跑 89 passed、1 个缺 initrd 哨兵失败，退出 101；
  日志 `target/refactor-resource-helper-verification/qtest.log`。六个归位入口文件 scoped rustfmt
  及 diff check 通过；不为 pend 宿主文件既有格式偏差做全文件重排。
- 最终五场景复跑时 accept/product/iommu-product/iommu-dma 通过，但 system-fault 出现一次
  非哨兵失败：Join 后单次 Oust 收到 UnitFail::Busy，探针 unwrap 退出。
  现场完整保留于 `target/refactor-resource-final-scenes/system-fault/scene-system-fault-qtest.log`。
  Join 只证明退出，不证明物理退休结清；Oust 与既有收尸并发允许 Busy，不据此放宽内核判据。
- 探针在既有 lifecycle 有界等待上统一处理 Oust：仅 Busy 重试，其他错误立即失败，
  超时仍失败，必须 Oust 返回成功后才报告回收；system-fault 与 copy 的三处同类假设一并迁移。
  修后 workspace dev/release、kernel tests check 通过；system-fault 连续串行五次通过、各退出 0，
  现场目录 `target/refactor-system-fault-retry/run-{1..5}/`。
- typed 客户端独立 reviewer 因 OpenCode Go 的 deepseek-flash 服务错误中断，
  没有形成完成的审查结果，不能登记为无 finding；内核独立 reviewer 随后也因该后端错误中断。
  已停止重复派发大范围审查，缩小范围的客户端复核随后完成，结果见下。
  主线程补做 Seal、Pole 创建/映射、Tole 来源权限与异步续等的关键路径自审，
  未发现新的已确认缺陷；这不是独立审查批准，也不是并发交错验证通过。
  磁盘目前约 12GB 可用，未清理其他工作树。

客户端独立复核（原两项 P2 的闭合过程见下）：

- DeepSeek reviewer 只读复核资源/IPC 客户端及依赖的内核释放路径，未发现确认的 P0/P1。
  确认失败分域、typed 事件来源及 raw::pole 的 fresh-reference 清理路径；
  fresh-reference 的 Memory Release 无可失败同步，不把这句 expect 与 Nest 关闭混为一谈。
- P2：Rack::open 映射失败及 Port::open 授出失败后的 raw::release 都是 best-effort。
  cull 备容量可返回 OoM；失败时原 token 及来源仍由任务能力表持有，直到任务清理。
  两处比基线未尝试释放有所改善，但不构成可靠回滚；现有通过测试未覆盖这两条叠加失败，
  Port::open 也没有在树调用点。此限制未作为构造成功或最终回收通过的证据。
- 本批不为两个构造器临时增加平行清理队列，也不无限重试 OoM/Denied。
  后续统一处理构造失败与 Capability/Loan 释放失败的身份、原始失败和重试责任；
  首次复核时两个 P2 尚未闭合，不将该次客户端审查结论写为零 finding。
  PortOpenFail 的分域说明及 Rack 的失效错误注释已校正，移除不存在的 PoleFail::Gone 描述。

单节点根释放的后续修补（已集成、验证并完成局部独立复核）：

- 只读核对定位到：本地新根也走 cull 的 collect、removed、changed 及 guards 容量预留，
  OoM 在 take 之前发生。选择核验严格前置条件后复用 take/notify/withdraw/drop 的单节点路径，
  不改公开错误/API，不新增状态、队列或无限重试；有父、后代、映射、在途义务仍走原规则。
  DeepSeek worker 的 `2f73ede8-ac25-4a81-97f4-e14a37d40aec.13001089` 已集成；
  主线程另补实际内核回归，没有并行改写 worker 文件。
- 新增 unpublished_roots_release_their_sources：实际 Hole 元数据释放、四页 RAM Pole 源页净回收、
  PoleOperation 持有时 Busy 且保留身份/帧、旧 SourceAuthorization 元数据不额外 pin 帧；
  最终站点、页表、物理帧必须回到基线。此例本身不证明释放无分配。
- 首次用例因夹具未终止 Held 任务而残留十张表页，失败保留在
  `target/refactor-single-root-baseline/qtest.log`；补正常 Slay/bury 后，所有净回收断言不变，
  当前旧 cull 基线实跑 90 passed、1 个缺 initrd 哨兵失败，退出 101，
  记录 `target/refactor-single-root-fixture/qtest.log`。未将它写成单节点新实现的验收。
- Rack 映射失败先尝试释放，再分配格式化诊断，避免用户堆分配失败抢在清理动作之前。
- 新路径主线程实际运行 pend 宿主 43 tests passed：三种单节点根在下一次分配必失败的钩子下
  释放成功，钩子未消费；有父、有后代、已有映射仍走原图，OoM 时身份保留；
  Pole 操作租约、预留和在途发布仍返回 Busy，解除后可释放。
- workspace dev/release、kernel tests check 均通过；新路径 QEMU 实跑 90 passed、
  1 个缺 initrd 哨兵失败，退出 101，日志 `target/refactor-single-root-integration/qtest.log`。
  内核真实四页净回收用例通过，不以 has_maps 为空证明其他 Nest 已结清。
- 新路径集成后，五个既有整机场景严格串行各通过、退出 0，
  现场 `target/refactor-single-root-scenes/<scene>/`；resource/IPC/programs 的 38 套宿主 manifest
  重新全过（加上单独执行的 pend 共 39 套）。定向 rustfmt 和 diff check 通过，磁盘约 12GB 可用。
- 独立 reviewer 只读复核确认原两项 P2 的单节点容量预留失败面已闭合，
  核对了 Dock 失败后的 Nest 票据持帧、旧来源元数据失权及 settle 的通知/撤图/析构等价。
  这只消除严格单节点条件下的容量预留失败，不保证一般图清理不失败；
  尚未直接注入完整用户态构造器的叠加失败。
- reviewer 另指出新路径直接返回 Busy 少了旧 cull 的 RETRIES 有界重试。
  主线程新增“第一次 Busy、随后恢复”的确定性宿主夹具，先真实复现 Err(Busy)；
  失败日志 `target/refactor-root-retry/before-retry-fix.log`，未修改测试期待制造通过。
- release 已对单节点检查复用原 RETRIES；仅 Busy 继续，其他失败原样返回，
  不适用单节点时仍走原图。增加预算耗尽与身份保留断言，两例都在下一次分配必失败时执行；
  pend 宿主现为 45 passed，仍有三条既有 race-scene cfg 警告。
  Busy 修补后的 workspace dev/release、kernel tests check 均通过；
  QEMU 90 passed、1 个缺 initrd 哨兵失败，日志 `target/refactor-root-retry/integration/qtest.log`。
  五个既有整机场景再次串行通过、各退出 0，现场 `target/refactor-root-retry/scenes/<scene>/`。
  同一 reviewer 随后读取主工作树复核 Busy 修补，确认该条 P2 闭合，无新增 P0/P1/P2。
  三条 P2 至此在各自确认范围内闭合，不将局部结论扩张为整批内核所有路径的审查批准。
- 保留验证限制：宿主“无分配”测试使用 messenger/Pole 假机制，不能单靠它证明生产通知绝无分配；
  真内核用例提供源页与净回收证据，生产通知与锁序另经静态复核。
  无直接用户态构造器叠加失败测试、真实发布/死亡并发及完整异步失权矩阵。
  race-scene 的暂停探针仍只针对带 sire 的分支，单节点根不经过该暂停点。

下一批精确子 Pole 的只读核对：

- 当前 Unseal 仅支持完整私有根；Grant.window/AccordWindow 是授权分支窗口，
  不是独立子资源。已有 Creation、来源发布闸门、PoleOperation、ViewEntry 和 Nest 票据可复用，
  但不能把私有根的 restore_private 机械套到无私有创建资格的派生记录。
- 子形态跟父、形态不符拒绝、父分支 FETCH/VEST、范围内分配及未结义务继续占用均已确认，
  不将这些重新列为产品待决。
- 实施前仍需明确三个内部落点：token 退表后的来源控制绑定与依赖寿命；
  CPU 清理和外部义务各自可靠结清的占用判据；派生记录构造与失败回滚。
  不用 Arc/Weak 存活或 Flat::is_pinned 代替这些事实；后者包含普通和临时 Residency。
- 普通引用表里的 retained Residency 随 pend 清理释放，尚不是独立安全持有。
  本次只读核对没有修改精确子资源机制，也不证明控制者死亡后的 DMA 安全。
- 控制绑定建议尚未进入实施：资源来源链不能混成 Pie.sire 授权树，
  ViewEntry.branch 的授权索引也不能直接换成 Pole 身份。当前 hand_over 实际清空 branch，
  消费后 Acquire::View 的旧拒绝判据保持不变；合法 Forget 的索引/控制绑定同事务更新已是既定要求。
- 尚须闭合一个具体几何缺口：中间子资源已无自身 Meta/授权/View，但孙资源仍有未结控制时，
  父侧仍须识别子分配的完整范围。不得用孙的较小范围代替，也不得因普通旧引用或 Weak 可升级就永久占位；
  需要确认现有记录如何保存不持帧的分配身份/几何，并递归核子图真实义务。
  未据未经证明的“Root 权限或本记录 View”二项判据开放范围重用。
- 内部候选已收敛为子来源记录中的一次写入父坐标窗口，随父记录引用一起构造；
  父侧只保留历史分支索引与弱依赖，不另存可改窗口，孙的未结义务使中间分配继续占完整窗口。
  此数据建议尚未实施，尚须证明 CPU 退休期间来源记录本身不会先消失。
- 主线程扩展既有持续失败用例，真实观察到：正常 Slay/HUSKS 路径在失效未结时保持来源记录，
  放掉外部 Task/Team 后仍可找到；恢复并 bury 后记录、帧及页表回收。
  原始最后 Space 析构失败的对照则保留了 View/帧，但来源记录已经消失，其他空间结算不会恢复它。
  这证明“持帧”不足以支撑父侧的分配身份判据，不将对照的保留现状称为新方案通过。
- 上述补充仅改测试，未改生产寿命机制；kernel embedded check、定向 rustfmt 通过，
  QEMU 为 90 passed、1 个缺 initrd 哨兵失败，日志 `target/refactor-source-record-lifetime/qtest.log`。
  来源记录与票据的责任缺口由独立只读核验继续收敛，不能跳过 CPU 确认开放范围复用。
- 独立核验确认正常退出/Oust 的强持链成立，raw 最后析构的来源记录丢失也成立。
  因此父分配索引只能由可靠结清的显式提交解除，Weak 失效不得自动释放；
  记录丢失时仍须保留该分配身份和几何，不能把未知范围误报为空闲。
- 下一步已进入局部生产修补：Attach 失败回滚须先让 Nest 接管撤图，未结时保留预留控制记录；
  包括 view=None 的准备槽，不能丢 controller，也不能在结清后永远收不掉。
  Space 完整退休后则须沿现有控制收尾清掉 registry，不依赖最后观察 Arc 析构。
- 主线程新增 retired_space_releases_control_records，分别覆盖 close 失败与最终 ASID deallocate 失败，
  要求失败保留、恢复后即使旧 Space 观察引用仍在也清掉控制/来源/帧，最终站点与页表净回收。
  该例在旧实现上真实失败于“Space 已退休但条目仍在”，当前是 90 passed、2 failed
  （新回归及缺 initrd 哨兵），记录 `target/refactor-control-retirement/before-registry-fix/qtest.log`。
  未放宽断言；生产修补由本批次 worker 独占相关文件，尚待集成验证。
- 首个 Attach/Space 控制修补检查点 `c7280f45-dfc2-49dc-882e-3207b742357f.13152230`
  已集成并注册真实回滚用例。Space 退休回归已通过；首轮新夹具漏初始化 scheduler，
  主线程补齐后进一步真实复现了责任丢失，未靠改断言收尾。
- 首轮两条非哨兵失败：持续 Shootdown 失败时 close_controls 的 mem::take 搬空原控制索引；
  rollback 新增的立即 retry 则改变了旧 syscall_sync_failure 的“下个入口结算”合同。
  主线程另指出 locate/pending 两次查表可漏掉其间完成提交的记录，要求改成单次原子登记查询。
- 该轮为 90 passed、3 failed（上述两条及缺 initrd 哨兵），日志
  `target/refactor-attach-rollback/with-scheduler-fixture/qtest.log`。
  同组返工要求是未结控制原地保留并由 Task 退休/HUSKS 继续重试，不能仅留下登记再让任务退休；
  不改旧同步失败断言，也不扩大未提交 View 的权限。
- 返工检查点 `c7280f45-dfc2-49dc-882e-3207b742357f.13179533` 已集成。
  close_controls 原地处理控制索引，未结项保留并返回失败；Task::retire 在自身内存退休前传播该失败，
  由既有 HUSKS 保持任务并重试。doom 只推进清理，不把未结控制报告为成功。
  registered 单次持锁查询准备槽和已提交槽，locate 仍只允许使用已提交控制。
- rollback 不再立即 retry，保留旧同步失败的续等合同；只在票已撤下时清掉登记和 controller。
  Space::retire 成功归还 ASID 后回收登记，已退休分支也执行回收但不再次 wipe 旧 ASID。
  原 mem::take 失败及 syscall_sync_failure 均已在原断言下转绿。
- 主线程增加独立目标 Space 的持续失效用例：调用者自己的 Space 可正常退出，
  但外域 Caller 控制未结时，Slay/bury 仍须把调用者留在 HUSKS，保持控制、来源和帧；
  恢复后再 bury 才清掉身份和物理资源。该例防止自身 TaskMemory 的失败掩盖控制责任丢失。
  测试夹具已单独放入 attach_rollback_fixture.rs，最终站点、表页和持帧计数均要求回到基线。
- 最新 workspace dev/release、kernel tests check、定向 rustfmt 和 diff check 全部通过；
  release 保持七条既有 dead-code 警告，测试用 pending 查询仅在 debug 构建存在。
  pend 宿主重新执行 45 passed；其余 38 套宿主 manifest 最近一次通过在本批内核控制修补之前，
  不将此前结果记作这次重新执行。env 的 34 项通过亦是此前 ABI 检查点证据。
- 当前 QEMU 健康集为 92 passed、1 个缺 initrd 哨兵失败，退出 101，日志
  `target/refactor-control-retirement/final-health/qtest.log`。
  Attach 回滚、外域控制/HUSKS、Space 退休净回收及旧 syscall_sync_failure 均通过。
  accept、product、system-fault、iommu-product、iommu-dma 五个场景严格串行重跑，
  各 1 passed、退出 0，现场 `target/refactor-control-retirement/scenes/<scene>/`。
- 本批首次独立复核未发现 P0/P1，但确认一条 P2：子团队的 Nest 已 close、
  最终 ASID 归还失败时，控制占用票不会自行消失；若 Task::retire 先等待 close_controls，
  就永远走不到该子团队的退休重试。未将首次复核记作批准。
  主线程新增 committed Caller 控制、子团队预 close、一次性 ASID 失效失败、
  Oust Busy 后 owner Slay/bury 的真实组合回归，修前失败于 child.nest.retired()，
  日志 `target/refactor-control-order/before-fix/qtest.log` 为 91 passed、2 failed。
- 生产修补仅将 Task::retire 恢复为子团队退休、Caller 控制结清、自身内存退休的顺序；
  不把 closed 视作票据或 ASID 已结清，不改变失败码、HUSKS 或 Nest 的清理机制。
  原断言下修后为 92 passed、1 个缺 initrd 哨兵失败，退出 101，
  日志 `target/refactor-control-order/after-fix/qtest.log`。
  新例验证子团队成功退休、HUSKS 回基线、controller/来源消失及最终帧/表页/站点净回收。
  workspace dev/release、kernel tests check、定向 rustfmt、diff check 再次通过；
  五个既有场景再次串行各 1 passed、退出 0，现场 `target/refactor-control-order/scenes/<scene>/`。
  同一 reviewer 随后读取主工作树与修前/修后日志，确认 owner 自身 heir 子团队这一条具体 P2 已闭合；
  未把该限定结论扩张为所有 closed Space 上的 Caller 控制均已闭合。
- 首次独立复核的覆盖边界：控制游标受并发删除时的重试兜底尚无动态覆盖，
  若干 post-reserve 及指令同步失败分支未由新测试逐项驱动真实 Attach；
  成功 Space::retire 也不保证已排空 Nest.pending_cleanup，残留 View 可继续持帧到 Nest 析构，
  本批退休用例不被外推为该残留形状下的即时帧回收。
  raw 最后析构丢失来源身份的隔离边界仍在，不能由 Weak 失效推出分配已结清。
  本批尚未实现父分配索引或开放子范围重用。
- 后续先补两条更强的回归，在未改生产结清机制时实跑：
  其一经真实编码 Attach 向同 Team 另一 Task 拥有的构造中 child 发布 Caller 控制，
  child close 成功、ASID 归还失败后，Caller 退出不能依赖自己不拥有的 heir 退休重试；
  其二先真实 dispose 失败产生 pending_cleanup，恢复后要求可靠 close 在旧 Space 观察引用仍在时
  释放该已消费 View/帧，同时 ASID 仍不得提前复用。
  两例都真实失败，日志 `target/refactor-closed-settlement/before-fix/qtest.log`
  为 90 passed、3 failed（新两例与缺 initrd 哨兵），不删除旧断言来制造通过。
- 同组只读核验确认内部合同可复用已有 ClosePhase::Closed：该状态只由失效成功后的 settle 进入，
  与开始关闭的 Closing 不同。核对“票已消失或全 Nest 已可靠 Closed”的统一关系结清判据，
  以及晚到的清理入链与 Closed 排链之间的串行点；不由 Caller 接管外域 Space 的退休权限，
  不新增清理队列、并行状态或 CPU 失效成功的替代证据。
- 主线程已实现 Nest::settled(id)，供登记回收、Caller 退出与 Attach 回滚共用；
  contains_reservation 保持原占用身份语义，ASID 归还仍独立。
  Space 在 close 同步成功后即回收登记，ASID 归还失败仍保留根表、号及原 owner 重试链。
  unpublish_page 增加 release 构建下的 Closed 拒绝，防止已结清状态再次写入 PTE/dirty。
- Nest 在 close 成功或已 Closed 早退时排空 consumed View 链，retry 遇 Closed 不再 detach/NoRegion 循环；
  enqueue 与 Closed 摘链共用 pending_cleanup 锁，锁内按 Space → Nest 顺序核阶段，
  全链摘下后逐节点锁外析构。晚到入链若已 Closed 则直接在锁外放下，不留到最后 Nest 析构。
  Open/Closing 下的清理链和失败持帧责任不变，没有无条件排链。
- 首次集成日志 `target/refactor-closed-settlement/first-integration/qtest.log` 仍为 90 passed、
  3 failed：两条失败此时是旧夹具把“仅 ASID 归还失败”也当作必须保留已结清关系。
  测试已按新合同明确区分：close 失败仍须保留全部登记/帧；仅 ASID 失败则必须即时释放登记/来源/帧，
  同时 !retired、根表仍在、其他 Space 不能取得该 ASID；没有删掉最终净回收检查。
- 非 heir 组合增加两档：直接 Nest 归还失败时登记仍在，由 Caller 自身 settled 分支解除；
  经 owner Oust 时 Space 提前 reap。两档都经过真实 Attach，Caller 退出后目标 !retired，
  之后必须由 owner 重试归还号，避免提前 reap 掩盖 Caller 路径或暗中扩张权限。
  另增 close 成功后晚到 dispose 的节点用例：原 NoRegion 错误保留，但消费的 View/帧立即释放，
  retry 不再因 Closed 反复报 NoRegion。持续失效时 settled=false、身份/帧保留的断言同时保留。
- 最新 QEMU 为 92 passed、1 个缺 initrd 哨兵失败，退出 101，
  日志 `target/refactor-closed-settlement/after-fix/qtest.log`；workspace dev/release、
  kernel tests check、定向 rustfmt 和 diff check 通过，release 仍为七条既有警告。
  五个既有整机场景严格串行各 1 passed、退出 0，
  现场 `target/refactor-closed-settlement/scenes/<scene>/`。
  同一 reviewer 正在复核实际实现及新断言；合同核验不代替实现批准。
  尚无晚到 enqueue 与 close 真正并发的调度注入，顺序用例加静态锁序证据不被称为并发实证。
- 精确子 Pole 的后续内部比较选择共享帧无关的不可变几何作为候选：
  只放在 child SA 会在记录丢失时连范围一起丢失；父强持 child SA 又与子强持父形成环。
  父索引与子派生共享几何可在 child 丢失时继续精确隔离该段，不阻塞无关范围；
  此候选尚未作为签名或实施批准，没有新增 derive 路径。
- 原签名稿被退回两处：Forget 会改指的历史 branch 不能放进不可变几何；
  历史分支索引及 child 弱引用应留在父索引槽，几何仅保存 offset/count，子派生绑定 parent 与同一几何。
  另外，“spaces 经 owned_by 查不到记录”不能证明所有控制结清：
  普通旧 Hold 可继续保持 SA，而一个 Space raw 析构失败已丢登记，另一个正常控制后来结清；
  后一次扫描会把未知义务误报为空。把所有过期缓存永远当未结又会让正常结清后的元数据永久占位。
  owned_by 仅枚举已提交条目，还不足以表达准备槽的义务。
  同组继续收敛每条控制绑定的正向登记与显式解除事实，不能靠 Weak 缺席、计数或附加布尔猜结清。
- Closed 实现复核确认统一谓词、清理链的晚到入链串行点及锁外析构成立，但又指出两处相邻调用未完全对齐：
  unpublish_page 在 published 查找前一概拒绝 Closed，会改变残余 View 已无 PTE 时的合法 transfer；
  TaskMemory 仍以 contains_reservation 决定再次 withdraw，在已可靠 Closed 的空间上返回 NoRegion。
  该次报告不作为这两项的闭合结论。
- 主线程新增 closed_space_settles_late_task_memory：真实 reaped Task 的 close/retire 持续失效时
  仍返回 Shootdown 并保留 View；恢复 close 后，即使占用身份仍在，迟到的 TaskIdent::retire
  也应清掉帧/栈关系且不归还 ASID，最终站点、表页、帧及 HUSKS 回基线。
  修前实跑在 Err(NoRegion) 断言处失败，日志 `target/refactor-late-task-memory/before-fix/qtest.log`
  为 92 passed、2 failed（该回归与缺 initrd 哨兵）。
  TaskMemory 的帧、栈与 guard 已共用 settled 判据；原断言下修后为 93 passed、1 个哨兵失败，
  日志 `target/refactor-late-task-memory/after-fix/qtest.log`，退出 101，定向格式及 diff check 通过。
- 更正“Caller 只清自己的 entry”的过强描述：现有 reap_views 是通用 CPU 结清收尾，
  会解除本空间所有已经可靠结清的登记，不只当前 controller 的条目；它不撤销活映射，
  不关闭空间、不归还 ASID。没有为这一内部收尾新加 owner 权限例外。
  新增双 controller、三条关系 A/B/A 的两档用例：Open 时 A 清理后 B 的登记/帧/真实翻译仍在；
  Closed 时通用 reap 清掉全部已结清登记和控制，目标仍 !retired。
  该例还驱动 reap 回调在 A 迭代中删除其另一控制项，已实际通过；
  这不是多 hart 同时删除的动态覆盖，原并发验证边界仍保留。
- 下一子资源批次的 Binding 登记候选已修正为随 prepared claim、在任何 PTE 发布前进入来源成员，
  失败且尚未 settled 时仍保留；不得等到 committed 才记录，否则发布失败的义务仍会丢失。
  容量准备与写入必须共享同一串行段，不能分开 reserve 后假设并发 push 不消耗容量；
  额外 settle 锁已撤回，拟核既有 work::commit 能否统一登记、最后解除及父槽摘取。
  这些仍是冻结的内部候选，主线程尚须核真实锁序、分配约束与全部调用点；没有实现派生或来源绑定账。
- transfer 回归 worker 的提供方失败通知先到，当时没有可用代码交付或执行证据；
  主线程接手新增 closed_transfer.rs。
  实际 Transfer::new/execute 先在 close 持续失效时返回 Busy，目标空槽及源 Frame/花纹保持；
  恢复后明确核 pressings_over 仍包含 Closed View 及其 raw Nest，虽然 owner() 已为 None、
  published 为零，该残余关系仍确实走到 unpublish。
  修前实跑为 Err(NotTransferable)，不是提前 Busy/空源；日志
  `target/refactor-closed-transfer/before-fix/qtest.log` 为 93 passed、2 failed（新回归及哨兵）。
- 生产只将 Closed/Pinned 检查移到 published 的 position 返回 Some 后、任何 PTE 修改前。
  无已发布页保持原 Ok；若 Closed 仍含该页发布记录则 release 构建也拒绝，不允许再次 dirty。
  新测试要求成功移动同一 Frame、PA 与整页花纹，源槽 Missing、两份原 PTE 已无，
  目标能重新映射到同一 PA；Closed 空间的 ASID 仍未归还，随后显式 retire。
  来源弱关联条数、站点、表页与持帧计数最终全部回基线，没有放宽净回收断言。
- 最终主线程实跑健康集为 94 passed、1 个缺 initrd 哨兵失败，退出 101，
  日志 `target/refactor-closed-transfer/after-fix/qtest.log`；包含真实 transfer、
  迟到 TaskMemory 及多 controller 收尾的新断言。
  workspace dev/release、kernel tests check、定向 rustfmt 与 diff check 通过，
  release 仍七条既有警告；五个场景严格串行各 1 passed、退出 0，
  现场 `target/refactor-closed-transfer/scenes/<scene>/`。
  最终三项的独立复核两次遭遇 DeepSeek 提供方错误（首次及一次有界重试），均没有交付结论；
  未把合同建议、前一检查点的复核或主线程通过记作后续修改的独立批准。
  当时停止继续重试，未擅自改模型或持久设置；用户随后明确要求继续使用 DeepSeek，已恢复原审查线程。
  后续来源绑定候选的主线程静态核对已确认 work::commit 使用 Level::Unit 的 RelLock，
  同 hart 可重入，锁级依次为 Unit、Space、Nest、View、Flat、L3 等；
  这只确认既有串行原语，尚不构成两表事务或容量准备全路径的证明，未据此开始派生实现。
- transfer worker 随后迟到交付同名文件的 299 行独立版本，未注册、未构建或实跑，已确认冻结。
  主线程保留已注册并实跑的版本，不合并同名候选，也不把实现报告当最终 review。
  独立候选封存在线程 `ksQQwKxhEIDfRmKsYfFDQ_ymxZLOAM15X5IBxBBnK9ozTelNwroye-tIjWUe`，
  源版本 `0fe2aaab-77aa-4b82-b6b4-9aa977a5602a.13521260`；后续若取用须另行比较，不自动替换。
- DeepSeek 随后完成最终只读复核：未确认 P0–P2，确认 unpublish 顺序、TaskMemory 统一 settled、
  多 controller 的通用 CPU 收尾三项已闭合。主线程核对 reviewer 使用的指定生产 diff 范围
  SHA-1 为 `b0c43c35d74b30e7dd219590cb0eada320aaa599`，与当前一致；
  该值不是全仓版本，也不替代未跟踪测试文件的实际阅读。reviewer 未自行运行构建或 QEMU。
  最后任务退出不再等同于 Space 退休：空 Live Team 保留复活能力，
  空间级 wait site 由 Space::retire/Drop 清理，逐 View 的 dispose/withdraw signal 仍保留。
- 按复核建议实际补跑四 Hart 的 memory-transfer-race，使用脚本要求的 release debug assertions、
  semihosting 与 race-scene。首轮内核测试为 1 passed，但宿主总退出 1：
  自定义嵌套 TRACE_OUT 与驱动固定按 capture.parent.parent 查结构化导出不匹配，
  导出实际在仓库根，peer 的 wait/wake 存在。失败现场及真实根导出保留在
  `target/refactor-closed-transfer/race-scene/`，不将该轮称为通过。
- 随后保持同一源码与断言，按默认根下 trace/ 布局重跑：
  `CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true QEMU_SMP=4 TRACE_OUT="$PWD/trace" nu scripts/qtest.nu --package kernel --scene memory-transfer-race --feed-script programs/tests/memory-transfer/race.py -- --features semihosting,race-scene -- scene`
  内核 1 passed、94 filtered out，宿主完整判定退出 0；现场另存
  `target/refactor-closed-transfer/race-scene-validated/`。
  summary 含 158 条事件，peer 真 wait/wake 各 1 次、park 0、reap 1，
  teardown 的 press 2→1、外域 PTE 1→0、resumed=1，两阶段均 armed。
  这是现有 transfer 与跨域 teardown 的真实交错证据，
  不将它扩张为 close↔claim、close↔drain 或跨 Hart 控制游标删除的专项实证。
- 来源控制绑定前置已交同模型 DeepSeek worker，在独立工作树实现 canonical ViewBinding、
  prepared claim 与来源成员的同事务登记、CPU 结清后的显式解除；不新增 derive 或父子分配几何。
  源侧 bindings 拟取代空间弱缓存，只有一份共享的 controller/branch 事实，
  以既有 work::commit 串行两表准备和写入，不增加计数、完成布尔、清理队列或互斥。
  旧 worker 已冻结；新阶段禁构建/缓存，由主线程集成后统一实跑。
  不把上一批复核用于批准新增来源绑定实现。
- 来源绑定检查点 `fa92bf5b-9a0d-4749-9482-00c233d9363e.13596270` 已集成，
  主线程注册 source_binding 四项健康测试。ViewEntry 与 SourceAuthorization 成员共享同一
  帧无关 ViewBinding，prepared claim 两表同提交段登记；retire_view 自身核 CPU settled，
  同提交段撤 Registry 与来源成员并解除 controller。raw 析构不自动解除未知成员；
  spaces() 改由成员派生空间快照，旧 remember_space 缓存及调用已删除。
- kernel tests check 通过，pend 宿主 45 passed（仍三条既有 race-scene cfg 警告）。
  首轮实际 QEMU 为 94 passed、5 failed，退出 101，日志
  `target/refactor-source-binding/first-integration/qtest.log`。
  非哨兵失败为旧 first_spawn_atomic_commit 的“发布提交中分配或 shootdown”护栏，
  以及三个新夹具保留 entry/after 观察副本，自己持续强持 Source/View 导致净回收断言失败。
  raw 最后析构保留未知成员用例已通过，不称其隔离残留为最终回收。
- 主线程仅修新夹具观察者寿命：读取身份后放掉强持 Source/View 的 entry/after，
  保留纯 Arc<ViewBinding> 直到 Space/Source/Frame 已释放，进一步证明绑定本身不持帧或空间。
  原弱引用、站点、表页及帧净账断言未删除。
  真实 Attach 发布失败与持续 dispose 失败也增加来源成员 1→保留 1→恢复后 0 的检查。
  修后实跑 `target/refactor-source-binding/observer-fixture-fix/qtest.log` 为 97 passed、2 failed，
  仅余 first_spawn_atomic_commit 护栏与缺 initrd 哨兵；新四项及实际失败路径成员断言均通过。
- 同组 worker 独占生产首 Spawn 返工，主线程未并行编辑生产文件，不移除 NoAllocation 或放宽旧断言。
  新 DeepSeek reviewer 已独立开始审查本批来源成员的所有权、prepared/OOM 原子性、
  显式结清与锁序；目前没有独立批准，也不声称整批运行全过。
- 首 Spawn 修补检查点 `7700dfcd-30d2-4e72-b613-815fd57e547e.13626587` 已集成，
  仅修改 PreparedTask::publish 与 ViewRegistry::hand_over。发布前按既有 reserve_depend 规则
  准备依赖栈容量；hand_over 在登记表短锁中克隆绑定，放锁后才取绑定控制锁，一次只持一把。
  worker 静态定位为旧路径八个并持槽已满，新嵌套锁占第九槽触发 HeldSet 溢出，
  溢出诊断自己的格式化分配再被 NoAllocation 拦住；该根因属于源码核对，不记为 worker 实跑。
  主线程核对六个生产文件 blob 与冻结报告一致，worker 未再改父线程测试或注册。
- 修后实际 QEMU 为 98 passed、1 个缺 initrd 哨兵失败，退出 101，
  日志 `target/refactor-source-binding/publication-fix/qtest.log`。
  原 first_spawn_atomic_commit 的 NoAllocation 与全部断言保持；四个新来源成员用例、
  真实 Attach 失败的成员保留/解除、旧 Closed/transfer/权限/最终净回收检查均通过。
- workspace dev/release、kernel tests check 再次通过（release 七条既有警告），
  pend 宿主再次 45 passed（三条既有 cfg 警告）；定向 rustfmt 与 diff check 通过。
  accept、product、system-fault、iommu-product、iommu-dma 严格串行各 1 passed、退出 0，
  现场 `target/refactor-source-binding/scenes/<scene>/`。
  本批再次执行四 Hart memory-transfer-race，内核 1 passed、98 filtered out，宿主退出 0；
  现场 `target/refactor-source-binding/race-scene-validated/`，158 条事件、peer wait/wake 各一次、
  teardown press 2→1、外域 PTE 1→0、resumed=1。仍不把现有 race 场景扩张为来源 claim/reap 的专项并发注入。
  resource/IPC/programs 的其他宿主 manifest 没有在本次内部内核改动后逐套重跑。
- 当前仅等待本批独立 reviewer 对最新生产检查点的结论；worker 已冻结，主线程未再改生产。
  不将本批通过写作 derive、父分配槽、外部持有或 Driver 的完成，也不把 raw 未知成员留存写成回收成功。
- 本批首次独立报告未发现 P0/P1，初列四项 P2。主线程要求核当前可达性后，同一 reviewer 更正：
  全局唯一关系号的未来改制反例撤为不变量注记；缺失 prepared 登记的静默提交，
  在当前构造租约/运行任务不能已 Reaped 的条件下不可达，降为断言建议；
  当前首 Spawn 提交分配问题接受已闭合，只保留 depend::report 的 format! 会掩盖诊断的可选改善；
  Closed 后 Pole 外层仍以 contains_reservation 再次撤图，确认是当前可达的 P2 活性/API 假失败。
  不为未来同号假设新增 (space,id) 副键；release_control 只补全局单调不复用的不变量。
- 主线程补 source_binding_closed_input_allows_pole_cleanup：外域 owner 为 child 的 Input
  创建真实 Pole，持续 child 失效时 Oust 与 Pole 关图均 Busy、成员和帧保留；
  恢复后覆盖 Space 已退休及仅 ASID 归还失败两档，观察 Arc 仍在，旧占用身份仍在但 CPU 已 settled。
  Pole 自己的 HeldMap 尚在时，补关必须成功且幂等。修前实际失败于
  `Err(PieFail::Denied)` 对 `Ok(())`，日志
  `target/refactor-source-binding/closed-input-before-fix/qtest.log` 为 98 passed、2 failed（该回归及哨兵）。
  该例驱动真实 core seal/close_maps 与 team::oust，不宣称执行了用户侧 Seal 结果解码器。
- Pole close_maps、withdraw、Drop，Team 暂存取消及 Memory Detach 的外层门控已统一到 Nest::settled；
  未结才重试/撤图，close_maps 失败且最终仍未结时保留原记录及原错误；
  若并发方已提供可靠结清事实，则按该事实解除登记，不把 Closed 的旧身份再当活 PTE 撤下。
  contains_reservation 身份语义、CPU/ASID 失效判据未改。
  ViewRegistry::commit 改为 expect prepared 登记，破不变量时不再静默成功，现有合法路径不变；
  原不可达场景未冒充新增漏洞复现。无分配诊断的 format! 改善记录为独立 follow-up，本批未扩展诊断设施。
- 修后健康集为 99 passed、1 个缺 initrd 哨兵失败，退出 101，
  日志 `target/refactor-source-binding/closed-input-after-fix/qtest.log`。
  新例同时保留持续失效的 Busy/成员/帧断言，恢复后两档均清掉 Pole maps、最后 root/站点/表页/帧/HUSKS 回基线。
  workspace dev/release、kernel tests check、pend 宿主 45 项再次通过，七条 release 与三条宿主 cfg 既有警告不变。
  五个场景严格串行各通过，现场 `target/refactor-source-binding/final-scenes/<scene>/`；
  四 Hart race-scene 再次完整判定通过，现场 `target/refactor-source-binding/final-race-scene/`。
  fmt 检查在 Pole 文件仅报告 HEAD 已有的 window 表达式折行差异（未为格式重排），其余本次文件检查及 diff check 通过。
  随后同一 reviewer 定向复核本次门控增量及新用例，确认 P2-4 正确、最小、真实红→绿闭合，
  无剩余代码级 blocker。不新增副键，不放宽授权，settled/retire_view 仍为唯一结清规则；
  其他已降级项按不变量注记、expect 加固及可选诊断 follow-up 保留。
- 本批新增冻结说明：只要本 Pole 这一条关系已 settled，close_maps 不再受同一 Space 上
  其他关系未结失效的无关 Busy 拖住；这不是忽略本关系失败，而是按本关系的 CPU 完成事实收尾。
  同一 id 的 settled 单调性来自全局不复用身份与 Closed 不回到 Open，并非另存完成标志。
  reviewer 只读源码与修前/修后日志，未自行执行 fmt、构建、宿主或 QEMU；运行证据仍归主线程。
- 新 DeepSeek scout 已开始只读核对后续分配合同：有效授权必须覆盖所有仍可用分支，
  不能只读 SA.phase 的原 root Grant；并核“最后授权、最后绑定、后代槽结束”三个显式事件的安全挂载点。
  当前没有 derive、父槽、子几何的代码交付，不把本批批准用于预先批准下一阶段实现。

#### Pole Open 的 CPU 关系登记（本轮已实施，尚未独立复核）

- 只读核对确认了实际缺口：Pole Open 不经过 ViewRegistry，失败撤图后仅靠 Pole maps
  无法持续表达未结 CPU 义务。仅以授权结束、原 bindings 为空及后代槽为空释放子段并不安全。
- 带 SourceAuthorization 的 Open 现于任何 PTE 发布前建立 prepared 登记，与来源侧共享
  同一份帧无关 ViewBinding；失败且未 settled 时登记及成员保留，恢复后沿现有结清路径移除。
  无来源记录的启动 Pole 保持既有路径；未据此宣称所有启动映射已迁移。
- ViewBinding 只保留关系 id 与 Space 弱引用；显式 Memory 控制独立为可选的
  ViewControlRecord。Open 登记无控制记录，locate 不向 Memory View 调用暴露它，
  不把记录 CPU 义务扩张成新增控制权。已有 Attach、交付及首 Spawn 仍读同一控制快照。
  TaskMemory 退休成功后 reap 已结登记，避免恢复后的 Open 清理等待最后 Space 引用析构。
- 新用例覆盖 Open 成功与发布失败两档：持续 shootdown 下授权及 Pole Meta 退出后，
  prepared/ready 登记仍保住来源和帧；恢复后 retry/reap 显式解除，纯绑定观察者不 pin
  来源、Space 或帧，HUSKS、站点、表页及帧净账回基线。首轮失败是测试重复启用持久注入，
  修正注入时序，未删弱引用或净账断言。
- 实跑健康集为 100 passed、1 个缺 initrd 的 scene 哨兵失败，退出 101，
  日志 `target/refactor-pole-open-bindings/cpu-obligations-fixed/qtest.log`，不称全绿。
  workspace dev/release、kernel tests check、pend 宿主 45 项通过；
  七条 release 与三条宿主 cfg 既有警告不变。
  五个场景分别通过，现场 `target/refactor-pole-open-bindings/scenes/<scene>/`；
  四 Hart transfer/teardown race 完整驱动通过，现场 `target/refactor-pole-open-bindings/race-scene/`。
  该 race 不覆盖来源登记/结清的全部跨 Hart 交错。
- 本次文件 scoped fmt 与 diff check 已核；Pole 文件仍仅保留 HEAD 既有 window 折行差异，
  不为格式检查重排。以上是主线程运行证据，旧批次 reviewer 批准不覆盖本次代码。
  未实现子分配、父槽释放或 retained Residency 的结清合同，不能据此开启子段复用。

#### 精确子 Pole 首检查点（已验证，范围限于本节）

- DeepSeek worker 检查点已接入：派生入口、父槽与共享几何、使用门、Forget 改指及七个新回归；
  主线程已注册全部新例。不是仅有设计，也不把尚未运行的测试列为通过。
- 首轮 `cargo check -p kernel --tests --offline` 失败于新例两处 `Weak<Task>` 缺借用，
  pend 宿主测试编译失败于新类型导入路径及夹具接口未同步，尚未执行 QEMU。
  worker 同阶段返工，主线程保留原回收断言，不以删测试制造通过。
- 接入走查已要求修正：派生容量预留与发表跨提交段、Transfer 漏接使用门、
  准入与实际发布的提交边界，以及用 `is_pinned` 和额外完成盖章承担分区最终结清的问题。
  独立 reviewer 只审已接入检查点，不能预先批准 worker 的后续改动。
- Open 复核另确认：失败发布可以没有 Pole maps 而仍有 CPU binding，
  Shut/Seal 成功或 has_maps 为空不是分区可复用证明；派生结清不得绕过未解除的 binding。
- 返工已接上 Transfer 使用门，派生预留与发表同提交段；移除完成盖章及 pin 推断，
  普通任务的保留引用显式登记 Residency 义务。它仍随任务引用表释放，不是未来 DMA 的独立持有。
  父槽结清须同时满足授权、CPU binding、后代及子记录自身保留都结束；
  末份判定与摘槽在提交段内，解除保留触发当次逐代结清。
- 主线程新增子记录自身保留的回归：Acquire(child) 后释放最后授权，父同段派生错误返回成功，
  修前 `retained-child-before-fix/qtest.log` 为 107 passed、2 failed（该红及 scene 哨兵）。
  修后覆盖 Release/Seal 两档，解除前 Busy、解除当场父槽归零、随后同段可用；
  原 `memory_traverse` 的重叠保留行为也保留，未删断言制造通过。
- 准入分为子分配冲突检查与发表时的保留检查：Acquire 可建立重叠 Residency，
  不把持有误作排他授权；派生、Open、Accord、Attach、Transfer 在子段检查之上核保留，
  豁免只认实际登记身份，不按 TaskId 放行。Forget 的全部可失败准备已移至首写前。
  相位快照先克隆出锁，再升级 Meta 弱引用，避免临时最后强引用在相位锁内析构重入。
- 九个派生回归与夹具按文件分开；Transfer 用例使用真正可迁移 Flat，
  验证旧父引用迁移子页 Busy、原 PTE 不变、子结清后同操作成功。共用夹具的初始化错误已修正。
  `target/refactor-sub-pole/final-health/qtest.log` 为 109 passed、1 个缺 initrd 哨兵失败；
  workspace dev/release、kernel tests check、pend 宿主 45 项通过，五个场景分别通过，
  现场 `target/refactor-sub-pole/final-scenes/<scene>/`。七条 release 与三条宿主 cfg 警告保留；
  一处新测试的 Copy 值 drop 警告随后删除无效调用，生产未变。
- **实跑发现的进展回归（下述修补后已闭合）**：四 Hart `memory-transfer-race` 返回 kernel=timeout，
  对端未能在在途窗口内登记。不是 TRACE_OUT 路径问题，也不是已知 scene/initrd 哨兵。
  Transfer 持 Unit 跨 execute/在途暂停点，peer 的登记因此被全局锁阻塞。
  修前现场封存 `target/refactor-sub-pole/race-before-fix/`，原 worker 定向返工：
  缩短全局锁段，但不能移走分区准入后重新留下派生 TOCTOU；不改 race 断言。
  独立 reviewer 收下该 blocker，旧静态批准不覆盖其活性。
- 最终修形：Transfer 的短 Unit 段只核两端准入并落带页窗口的在途登记，
  执行不持 Unit。复用来源既有 inflight 闸门，登记存续期间拒绝重叠发表，
  事务自身仍以 Moving/Reserved 槽保护承载；第二端登记失败撤第一端，执行失败同样撤两端。
  两端保留原 Hold 与共享几何，从 Hold 读取 source/retention，不复制坐标与授权状态。
- 在途键区分关系域与 Transfer 域，后者以高位标记加本域单调号，
  与关系号的非负有符号上界结构性分开。耗尽返回 OoM，不 panic、不回绕。
  第十例手工构造同低值的跨域键，证明双方清理互不误删、同笔两端窗口全部撤净；
  它是登记清理的顺序单元验证，不是分配器碰撞的红绿或跨 Hart 实证。
- 最终 `target/refactor-sub-pole/domain-final-health/qtest.log` 为 **110 passed、1 failed**，
  唯一失败仍为缺 initrd 的 scene 哨兵，退出 101，不称健康集全绿。
  workspace dev/release、kernel tests check、pend 宿主 45 项再次通过；
  最终 scoped fmt 与 diff check 通过，Pole 文件仅保留 HEAD 既有 window 折行差异。
  五场景分别通过，现场 `target/refactor-sub-pole/domain-final-scenes/<scene>/`；
  四 Hart race 的完整驱动退出 0，现场 `target/refactor-sub-pole/domain-final-race/`：
  153 events，两阶段 kernel=armed、peer waits/wakes 各 1，teardown press 2→1、
  peer PTE 1→0、resumed=1。修前 timeout 现场保留，不弱化任何 race 断言。
- 同一 DeepSeek reviewer 对每次真实增量定向复核，最终域、上界和两端 own 均无剩余 blocker；
  reviewer 未亲自执行构建/QEMU，运行证据归主线程。worker/reviewer 本阶段均冻结。
  生产已跟踪文件差异指纹为 `05a716b4cc3d4bc907dfff18d22880bed374b2a1`；
  该 git diff 指纹不包含未跟踪的新文件，补记新 geometry/derivation/lineage/independent 的 blob
  分别为 `7a7fd06d18a141390a6308f254458574d6aa643d`、
  `87aa92e027085f3fdfd047f2de7c78a4c594b9eb`、
  `3504db98a5183cac0f92b08af0d32acf801483e7`、
  `0ae416fbe0b1c4bffad80a70e540d0fcc52ca962`。
- 覆盖仍有限：Open/derive 与末份结清的真实跨 Hart 交错、派生资源的首次 Spawn、
  Forget 改指后的完整权限链及父资源关闭子图，不由十个顺序用例或 transfer race 自动证明。
  Task-retention RAII 不能当作跨任务死亡的 DMA 保留，未实现的 Driver/DMA 合同不记完成。

#### 派生生命周期增量（范围及首轮验收现场）

- 首次交付使用真实 staging 与 Spawn：失败保留 token、控制绑定及父段占用，
  成功消费 token 并移交控制；接收者不能借 View 新建来源授权。退出时 CPU 失效失败保留
  binding 与父段，恢复并完成回收后才允许同段派生，保留严格帧、表页与 husk 基线。
- 活资源的 Forget 在首写前检查新父存活、同资源及覆盖全部重新挂接授权的权限。
  合法改指保持后代 Grant 和派生资源身份；拒绝不修改授权树、历史索引或控制记录。
  `forget-before-fix/qtest.log` 曾真实得到 Ok 而非 Denied；修后对应回归通过。
- 显式父 Seal 从既有来源账遍历派生关系，处理无本地映射、已消费子 Meta 及失败 Open
  的预登记 binding。只关闭 CPU 关系，不终止接收任务、不退休外部 Space，也不把普通 Drop
  改成级联关闭。CPU 等待在 Unit 外，失效失败保留补关身份，以 Nest 正向 settled 事实摘账。
  `parent-close-before-fix/qtest.log` 曾错误成功而非 Busy；修后保留原失败与重试断言。
- 结清判据补入既有 inflight 义务，最后一笔 leave_publish 当场推进父槽结清，
  不新增完成旗标或持久清理队列。该新增用例是顺序登记测试，不是跨 Hart Transfer 实证。
- `parent-close-after-fix/qtest.log` 还记录了一次测试夹具清理失败：with_running 留下的
  Ready 任务尚未入调度队列。夹具在同一 Unit 内先入队再 slay；未删除调度或最终回收断言。
- 本节现场统一位于 `target/refactor-sub-pole-lifecycle/`。`final-health/qtest.log`
  为 **117 passed、1 failed**，唯一失败是缺 initrd 哨兵；新增七例全部通过，不称全绿。
  实现后已运行 `CARGO_INCREMENTAL=0 cargo check --workspace --offline`、
  `cargo check --workspace --release --offline` 和 `cargo check -p kernel --tests --offline`，
  三项通过；同样关闭增量编译的 pend 宿主测试以 x86_64-unknown-linux-gnu、offline、
  共用 `target/host-tests` 运行，45 项通过。release 六条既有未使用方法警告、
  宿主三条既有 race-scene cfg 警告保留。
- 首轮最终验收按 `TRACE_OUT=.../final-scenes/<scene> nu scripts/qtest.nu --package kernel --scene <scene>`
  顺序运行：accept、product 各 1 passed；**system-fault 退出 101**，
  `programs/src/harness/probe/hierarchy.rs:334:36` 的 `capability.unbind(exact, wait).unwrap()`
  得到 Unknown。完整现场为 `final-scenes/system-fault/scene-system-fault-qtest.log`；
  这不是缺 initrd 哨兵。当时停止并报告阻塞，原因随后按用户收尾要求定位并修复，见下节。
  该次命令未执行后两场景及四 Hart race；不能把后来通过的结果回填成该次通过。
- 主线程定向自查了 Forget 首写边界、父关闭的 Unit/等待边界、binding 身份核对及在途结清，
  scoped rustfmt 和 `git diff --check` 通过。本增量未获独立复审，不沿用上节 reviewer 批准。
  未动态验证派生/关闭的真实跨 Hart 交错、遍历工作表 OOM 或后代 reserved staging 分支。
- 检查点 HEAD 为 `4fe28fe35b7fe4785746d5f040be79ca2b7753f7`；生产已跟踪差异
  `git diff -- kernel/src crates programs/src | sha1sum` 为 `07bb575cbb8730a2d143f2d8afe83f4979cf7652`，
  不含未跟踪文件。新 close.rs、seal.rs、entry/call/pole.rs 的 blob 分别为
  `f50ffbcb63b63e9a76835c54fdf1db0f8253e724`、
  `440dc23cb17c58be5a2223faa59312dea8cfbd12`、
  `d78951683115ac4cc4f4908f214b8eab77939d2b`。生产代码未因本次场景失败继续修改。

#### 本轮收尾（已完成，限于上述范围）

- 用户要求收尾后，仅修复验收阻塞并补完原定检查，没有开启 Driver/DMA/ABI 开发或新 agent。
  根因是本轮 Forget 将新父活性检查套到已封闭资源的引用释放上：
  Operator 对已封闭 Hole 执行 Release Keep 时得到 Denied，被映射成解绑 Unknown。
- 修复规则区分授权与清理：活资源仍要求新父当前存活且覆盖所有重新挂接授权；
  已封闭资源允许在同资源关系中移除中间引用，不重建 Grant，不恢复后代活性。
  ONLY 拒绝、同资源及父节点存在性检查、首写前全部可失败准备保持不变。
- 新增宿主例 `forget_releases_sealed_references_without_reviving_descendants`：
  Narrow 新父后活资源 Forget 仍 Denied，封闭后 Forget 成功；后代保留同一 Grant、
  权限快照与查询身份但仍失活。`sealed-forget/before.log` 真实复现封闭后错误 Denied；
  `sealed-forget/after.log` 为全量 **46 passed**，不是删掉原断言或吞掉解绑错误。
- 修复后重新运行关闭增量编译、offline 的 workspace dev/release 与 kernel tests 三项检查，
  全部通过。`closeout-health/qtest.log` 为 **117 passed、1 failed**，确认唯一失败仍是
  scene 缺 initrd 哨兵，命令退出 101；七个生命周期回归保持通过。
- 以相同 `nu scripts/qtest.nu --package kernel --scene <scene>` 顺序重跑
  accept、product、system-fault、iommu-product、iommu-dma，五项均各 1 passed；
  现场 `closeout-scenes/<scene>/`。修前 system-fault 失败现场保留，不用重试覆盖它。
- 四 Hart race 使用 `QEMU_SMP=4 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true`、
  `--scene memory-transfer-race --feed-script programs/tests/memory-transfer/race.py -- --features semihosting,race-scene -- scene`；
  TRACE_OUT 使用项目 `trace/`，运行后封存 `closeout-race/`。完整命令退出 0：
  158 events，两阶段 kernel=armed、peer waits/wakes 各 1，press 2→1、peer PTE 1→0、
  resumed=1；保留原驱动断言，没有通过更改场景预期制造通过。
- 本轮主线程自查与 scoped rustfmt、diff check 完成，没有独立复审；
  首检查点 reviewer 批准不覆盖本增量，前述未动态覆盖项也不由场景通过自动证明。
  最终生产已跟踪差异 SHA1 为 `7fb4b43f8c421c94e26336bc22b9152eb5f6b8a8`，
  close.rs、seal.rs、entry/call/pole.rs 的 blob 与上节相同；Forget blob 为
  `e9f15848b3dee20df105c5f9df9689e803d59571`。上述验证完成时 HEAD 未变，尚未提交；
  随后按用户交付要求以本地检查点分支封存，不推送或合入。
  `closeout-code.sha256` 记录 kernel、crates、programs 下 1315 个已跟踪及非忽略未跟踪路径
  的内容 SHA256（删除项显式记 DELETED），覆盖新源码与测试；清单自身 SHA256 为
  `6ef0a3c9d675ef28be017875fca84df89e373b469295d7e4470b75b8414f7fe7`。

DMA 是独立问题：用户态设备管理与恢复流程先独立保留物理帧，
通过 IOMMU 和设备控制机制撤销访问，可靠确认硬件安全后才释放保留。
Nest 不认识设备，也不承接 DMA 恢复；CPU 失效完成不能替代硬件停止证明。
独立保留不能随使用任务死亡自动解除的要求不变，具体承载先核现有 Residency/Flat 及持有者生命周期。

其余待闭合合同仍包括实际分支范围与权限的提交复核、派生/使用的原子发布及失败回滚、
以及外部保留的确认/解除授权与初发失效域。解决这些问题不授权新增并行状态权威。
编号耗尽须终判，重复确认应幂等；Busy 只表达真实在途或未结工作。

本轮精确子 Pole 的首检查点已按本节范围验证；外部持有接口与独立 Driver 未实现，
原有 DMA 产品与探针场景的旧回归结果不是当前生命周期增量或 Driver 化完成证据。
当前生命周期增量已完成限定范围的收尾；不自行追加设计、实现、Driver/DMA 或 ABI 收尾，
也不擅自提交、推送或合入。

### 后续检查点：保留授权委托与恢复方装配

- 基线为 `7274c61b`，后续工作位于 `feature/external-retention`，不改写已封存提交。
  用户要求真实用户态可用，不接受只有内核测试能调用的内部核心；这不意味着必须新增系统调用。
- 用户随后明确：本轮不以恢复者也死亡为设计前提，不接受永久占用作为完成结果。
  指定恢复者持续承担结清责任；使用方死亡后，恢复者完成停止、撤销及同步，再解除持有。
  确认前保留只是等待结清，最终须证明帧、来源占用及持有记录净回收。
- 复核既有代码后，主线程与 DeepSeek 撤回新增 Register/Confirm、逐义务凭据及全局持有根的方案；
  它们未获批准，也未写生产或测试代码。普通 Acquire、IndependentHold、Release 语义保持。
  恢复者可在自己的引用表中用现有 Acquire 的 retention 授权取得真实持有，
  源任务死亡不清除恢复者的表；实际停止确认后用现有 Release 解除。
- `kernel/tests/health/retention/effects.rs` 已走上述两个真实 MemoryCall；
  `retention/mod.rs:60-120` 已验证源 Task/Pole/Pie 消失后七帧仍保留，
  停止确认超时不释放、恢复后真实净回收。该测试不是最终 Driver 装配完成证据。
- 当前真正缺口是生产授权及职责装配：引导只把保留授权交给 System，
  改前 require_retention 要求授权对象 owner 等于调用者，阻止了既有 Accord 子授权的使用。
  用户已批准第一笔小改动：保留授权可沿既有 Accord/Grant 链委托；
  使用者须实际持有活 Pie、内核铸 retention Nole 及 VEST，历史 owner 不再等于唯一获权使用者。
  Nole 归属及 Seal 规则、源范围/源权限检查、所有普通引用生命周期和 ABI 保持。
- 第一笔仅实现上述授权规则与真实调用回归：合法委托可用，伪造/越权/缺权限/撤销仍拒绝新增保留；
  已取得的持有不因授权撤销提前结束，恢复方完成安全确认后通过既有 Release 净回收。
  不新增类型、调用或持有总管；主线程统一构建、实跑，并对实际增量独立复核后单独记录、提交。
- 第二笔再确定实际恢复执行方及其授权交付点、迁移现有恢复流程，
  不能用测试 System 夹具代替最终 Driver 架构，也不默认把设备恢复职责放入 System/Hub。
- 本轮不新增恢复者自身故障的恢复框架、不改通用 Release/Forget 来冒充退出结清，
  不以提前释放制造零泄漏，不删除窗口授权、不重编号 ABI、不把设备责任搬入 Nest。
  第一笔委托已实施并验证；第二笔的结构与 IPC 已获用户批准，DeepSeek 正实施真实迁移。

#### 保留授权沿 Accord 委托（已验证并复核）

- 唯一生产行为变化在 `require_retention`：调用者表内活 Pie、VEST、
  内核铸 retention Nole 身份仍必须满足，不再把历史 owner 当唯一使用者。
  Nole 归属及 Seal、源 FETCH/窗口/形态、普通 IndependentHold 和 Release 全部保持。
- 新增 `retention_authorization_delegates_via_accord` 经真实 envcall dispatch 验证
  跨任务 Accord 子授权及 Acquire，且 Nole owner 仍是原方；非本表父/子 token、
  同 Mark 的自造 Nole、缺 VEST、实际 Narrow 掉 VEST、Revoke 及真实 Seal 均拒绝新增保留。
  重叠保留合法，释放只撤自己一份；源和铸发方退出后已取得的持有不早释，
  最后 Release 后来源占用归零、可再发表，七个已观测数据帧 Weak 全部失活。
  初发根 cap 和源授权的准备复用引导及既有夹具；不冒称已运行独立恢复 Driver。
- `traverse_pages` 与 `resource_registration` 两处旧 owner 拒绝断言按获批语义改为正证，
  同时补非本表 token、父 token 和撤销后的负证，新增引用与测试 Pie 逐份释放。
  不是删除伪造/越权断言或放宽源权限来制造通过。
- 现场统一位于 `target/retention-delegation/`。在相应正证上回放旧 owner 谓词，
  `owner-predicate-before/qtest.log` 为 115 passed、4 failed：
  三个委托正证真实 Denied，加一个缺 initrd 哨兵。
  恢复获批谓词后 `approved-predicate-health/qtest.log` 为 117 passed、2 failed；
  多出的失败是新测试把既有不合法的 Narrow(empty) 当作成功，不是生产判据失败。
- 只修测试为新授 FETCH|VEST 子授权后实际 Narrow 到 FETCH，不改内核 Narrow 规则。
  Seal 负证改用独立活来源及尚在表内的子授权，先成功再真实 Seal 后拒绝，
  不让已撤销 token 或已死来源掩盖目标判据。两处 `check_publish` 自检在提交段内，
  精确核 Busy/Ok；追加 `seen.len()==7`，排除空集回收假证。
- 最终 `health-final-count/qtest.log` 为 **118 passed、1 failed**，
  唯一失败仍是缺 initrd 的 scene 哨兵，qtest 原始退出 101，不称全绿。
  新例及两个更新后的旧例通过；全尾段真实 Seal、issuer doom 和七帧回收均已执行。
- 主线程关闭增量编译、offline 运行 workspace dev/release 与 kernel tests 三项 cargo check，
  全部通过。pend 宿主以 x86_64-unknown-linux-gnu、共用 `target/host-tests` 实跑 46 项通过，
  日志 `host.log`；宿主集合不替代内核入口判据的 QEMU 验证。
  release 六条既有未使用方法警告、宿主三条既有 race-scene cfg 警告保留。
- 五场景 accept、product、system-fault、iommu-product、iommu-dma 各自通过，
  现场 `scenes/<scene>/`。四 Hart race 使用 release debug assertions、
  `--feed-script programs/tests/memory-transfer/race.py -- --features semihosting,race-scene -- scene`，
  完整驱动退出 0、feed 结果 ok；现场 `race/` 共 158 events，
  两阶段 armed、wait/wake 各 1，press 2→1、peer PTE 1→0、resumed=1，原断言保持。
- 新 DeepSeek namer 核对唯一判据及职责命名，新 reviewer 独立静态复核实际增量无阻塞；
  reviewer 不运行构建/QEMU，运行证据归主线程。已修两项测试构造、文档漏改及七帧计数自检，
  新测试 scoped rustfmt 与 `git diff --check` 通过，不全树重排。
  owner 退出的 seal_owned 与在途 Acquire 的真实跨 Hart 交错未定向注入验证，
  退出封 Mail 未取提交段的形态本笔未改；不以 Transfer race 代替此项证明。
  新例自证七个数据帧与来源占用的回收，不将其扩称整套堆、表页账的新增净零验证。
- 六个变动源码/测试路径的内容清单为 `code.sha256`，含未跟踪新测试，
  清单 SHA256 为 `c63b57142208377d69f00b56f6693771f402b7e3688a9a274ee7e44fdcf81ad1`。
  本检查点在 `feature/external-retention` 本地提交封存，不推送或合入共享分支；
  不把委托可用宣称为生产 Driver 装配已完成。

#### IOMMU Driver 恢复迁移（已批准，实施中）

- 基线为 `3c5e5bb6`，父分支 `feature/driver-recovery`。用户已批准直接实施，
  不再对已确认范围内的机械接线、命名和内部签名逐项停等；只在真实合同阻塞时报告。
- System 创建 held Driver 与设备服务，服务实际 sire 为 Driver；定向交付 Retain 子授权
  （含 VEST）、Doom 及 IOMMU/ECAM/aperture 能力，不传其他进程 VA，不改通用供给准入。
  Driver 自己映射并持有 Pages、Holds 和停机权威；System 的硬件推进与保留账实际迁出。
- 关闭使用现有 IPC：Driver 完成完整停链、释放设备持有并成功关闭页资源后，
  才发送本次关闭的终端响应。System 核内核发送者身份及绑定通道，不解释硬件字段或复制计数账。
  先等响应，再 join/回收 Driver，最后才撤其能力及镜像等依赖；异常退出不算成功。
- BAR 落位与开启 Bus Master 分离；先核 MASTER 关闭，帧持有、恢复权限及 IOMMU 约束
  全部就绪后才开启。完整停止、复位/排空及回读链不削弱，失败保留并重试直到真实结清。
- 持有与停止机制留在 Driver 功能域，不新增内核调用、确认凭据、全局持有根或公共资源库。
  本笔 IOMMU 与端点恢复仍在同一 Driver，不宣称 PCI/IOMMU 已各自独立。
  故障注入用薄测试适配和镜像变体，不把景/路径码放进生产启动协议。
- 同一 DeepSeek worker 单一写入代码，父线程只维护本文件并统一构建、场景验证及提交。
  首个代码检查点已接入，三项构建检查、相关宿主与健康回归已实跑；
  首个 iommu-product 完整硬件链运行后在完成回执编码处崩溃，场景脚本判失败，尚未验收通过。
  回执布局、System 未结清门控及移交负证已交同一 worker 定向修正，并补协议回归；
  独立 reviewer 正核实际增量，不沿用前一检查点绿灯。
- 用户随后指出当前方案多处偏离既有架构，本迁移版暂停扩写与验收提交。
  待纠正的是专用 DeviceAssembly/raw Loader 生命周期、额外 bootstrap 能力交付旁路、
  缺少 Hub 发现/认领/绑定，以及 IOMMU manager 混入 PCI/设备恢复与固定 DMA 测试编排。
  内核管理写权不进入普通设备库存，不等于 Hub 无需参与发现、绑定和协调。
  已通知 worker 保存窄返工检查点后冻结；其后返回的修复 frontier
  `1de33c7b-8833-448c-bf93-e593f64badbe.14253894` 尚未接入，不把局部缺陷修复当架构已对齐。
  保留当前未提交迁移、失败现场和 worker 检查点，不回滚或删除；稳定基线 `3c5e5bb6` 不变。

#### Driver 迁移返工：既有链路核对与功能模型（已确认）

用户要求继续后的本步只核对既有代码与最小返工范围，不把冻结版重新作为实施基线。
保留授权修复 `3c5e5bb6` 及既有硬件停止判据不变；不接入旧 worker 的局部修复，
不删除冻结源码与运行证据。

| 独立效果 | 状态权威与复用点 | 失败责任 |
| --- | --- | --- |
| 构造、安装、启动 Driver | UnitFile/Deployment 选择；Assembly 调用 Loader，Control 登记真实实例 | 安装失败由现有作业回滚；不保留 DeviceAssembly 的第二份 Task/Team/阶段账 |
| 交付平台资源与保留授权 | Supply Binding 经现有安装链交付；提供方核调用者、目标及权限 | 交付失败由租约清理；Retain 精确带 VEST，不提高全部 Call 的权限 |
| 发现设备并绑定使用者 | PCI 提供发现事实；Hub 持设备及绑定事实，沿现有 Operator/Identity 授权 | 绑定失败不开放设备访问；原使用者死亡不是可重绑证明 |
| 建立并发布 DMA 访问 | Driver 执行硬件操作；现有资源引用持帧，先备保留、恢复权限及翻译约束，再开 MASTER | 未发布失败可回滚；已可能访问则先停机，不因任务退出释放最后持有 |
| 停止、隔离并确认完成 | Hub 协调；PCI、IOMMU、设备 Driver 分别执行本硬件操作，稳定恢复方持必要权限与引用 | 超时或确认失败保持隔离及持有，重试同一责任；不把日志或进程死亡作为完成 |
| 解除持有、重新绑定及退出 | 硬件责任方完成停止后释放引用；Hub 才允许重新绑定；Assembly/Control 才拆依赖 | 无有效完成结果不得正常完成退场；恢复后必须验证最终回收 |
| 编排工作负载、故障和断言 | Harness 发请求、控制故障时点并核结果 | 测试不代替 Driver 执行硬件恢复；生产路径不含 sector/sentinel/Path 场景状态 |

本次源码核对确认的接入点与缺口：

- `system/assembly/static.rs::hooks` 已有构造、准备、供给与退休挂点；
  `supply` 已消费 `program.supply()`。冻结版在 `app/install.rs` 另装 DeviceAssembly，
  在 `app/schedule.rs` 另跑设备生命周期，属于旁路，不是现有机制不足以构造程序。
- `Supplies::supply` 已按 Binding 获取租约并交付描述；`grant_call` 只给 FETCH 且丢弃接收方 token，
  不能原样用于 Retain。平台资源应作受限来源，不向普通库存开放 IOMMU/ECAM 写权。
- `Hub::claim_with` 目前在旧 sensor 失效时可直接再交付，`vacate` 也直接清格；
  因此仅改为调用 Hub::claim 不足以满足 DMA 安全。新增 DMA 绑定必须先完成恢复再可重绑，
  不能套用普通设备的探活即空闲规则，也不能通过公共库存绕过这一门。
- `control/lifecycle/schedule.rs` 当前先执行退休挂点，再 slay/join，随后释放供给；
  自然退出由 `unit/reap.rs` 提交同一 Ruin。停机完成门必须覆盖这条真实清理链，
  不能只放在 System 最终 shutdown 中，也不能过早撤掉恢复所需命名空间和能力。
- 冻结版 `iommu/manager.rs::settle` 同时验扇区、主动 doom 服务、写哨兵并再敲门铃；
  这些是 Harness 编排，不是 IOMMU 的生产推进逻辑。PCI 枚举/BAR 与端点协议也应各归硬件职责。

本步推荐按上述功能边界返工，不采用“保留 DeviceAssembly，仅补 Hub 调用”的局部方案。
下一关只确定这些效果如何落到既有模块、稳定恢复方与受限资源来源，不发明第二个总管。
稳定恢复方 β 自身死亡不在本轮扩展范围；这不允许把确认失败后的永久保留当正常完成。
PCI/IOMMU 故障所需独立恢复权限与 ONLY 资源交付的关系仍须在发布前明确，
不能在实现时暗中用共享映射或放宽权限解决。本文未批准新的协议、类型或签名。

#### Driver 迁移返工：结构方案（已确认）

用户继续后进入结构关。本关不新增生产类型或协议；先确定真实实例、资源与恢复责任的归属。

| 操作需要的事实 | 唯一保存位置 | 结构变化 |
| --- | --- | --- |
| 镜像选择、Task/Team、安装和清理进度 | 既有 Deployment、Control 实例及 Assembly 作业 | 删除冻结版 DeviceAssembly 与 app/device 专用推进；不另存镜像、身份和生命周期阶段 |
| 平台来源、子授权、交付和租约 | 既有 Supply Source/Resource/Binding | 注册受限资源定义，沿静态/动态安装交付；不新增 Retain 专用启动收据 |
| 总线、requester、BAR 分配及 MASTER | PCI Driver 的本地核心 | 独立 Task/Space；只它持 ECAM 控制映射，BAR 分配事实不复制到 Hub/IOMMU |
| 设备翻译、命令完成与必要帧引用 | IOMMU Driver 的本地核心及既有 Reference | 独立 Task/Space；只它持 IOMMU 控制映射，持有与映射关联，不把帧引用放进 Hub |
| 设备协议、队列与设备专用停止确认 | 设备 Driver 的本地核心 | 精确 BAR 能力及本地映射；IOMMU 不再执行 virtio reset，PCI 核心不解释 virtio 寄存器 |
| 设备登记、绑定对象、本次恢复进度 | 扩展既有 Hub Ledger 的对应设备绑定 | 不新增第二本设备账；普通设备与需 DMA 恢复的绑定显式区分，不能用一个可认领 bool 覆盖恢复中状态 |
| 退休请求与外部责任完成结果 | Assembly 作业中的退休进度；硬件事实仍归 Driver | 在撤命名空间、能力、供给和杀任务之前等待 Hub 协调完成；静态与动态清理入口都要覆盖 |
| 固定工作负载、故障时点及断言 | Harness | 移出 manager 的 Path、sector、sentinel、主动杀服务与重复敲门铃等编排 |

受限平台来源仍归可信装配方。准入使用已登记的真实任务及安装策略，不靠资源名、Mark
或自报角色认证；Retain 只给获准持有方 FETCH|VEST，其他 Call 不因此提升。
现有 Supply 核心不改成按 PCI/IOMMU 名称分派的设备业务。Driver 的接口通过既有
Operator/Identity 安装与发现，Hub 登记发现结果及授权绑定，不接收裸进程 VA。

硬件停链不能随模块拆分而改弱：当前 platform 同时执行 DDTP Off、IOTINVAL/IOFENCE、
清 MASTER、virtio reset/drain 和队列回读。拆开后各 Driver 保存自己的命令与等待状态，
Hub 只保存协调进度；停止结果必须对应同一绑定，未完成时不能释放该绑定的最后持有。
业务入口关闭与 CPU 访问撤销也属于恢复前置，不能让仍可写 BAR 的使用者在复位后重新启动。

本关有一个尚未由既有合同唯一决定的结构分叉，不能在实现中替用户缩小故障范围：

- 较小范围可把 PCI、IOMMU、设备控制 Driver 作为持续存活的控制侧，协作承担 β；
  使用方 α 是请求/工作实例。DMA 所需引用由控制侧取得，α 死亡不触及它们。
  各寄存器面保持单一控制方，不新增集中恢复总管；该范围不证明控制 Driver 自身死亡可恢复，
  不推荐用它替代已有端点服务死亡场景。
- 推荐保留实际设备执行方 α 可死亡的验收。按当前 virtio 停止判据，
  正常执行方之外需要持续存活的设备专用恢复控制实例，
  并在发布前明确其 BAR 权限、业务访问撤销及无在途判据。不能只留 IOMMU 引用就声称完成恢复，
  也不能把 virtio reset 偷塞回 IOMMU/PCI 核心。它只保存本设备的恢复状态与所需能力，
  不构造任务、不保存总设备账、不解释其他控制器；PCI 与 IOMMU 仍各自执行本域控制。
  本轮不扩展这些稳定恢复执行方自身死亡的框架。此分叉不是上项的机械接线，须先裁决。
- 不采用一个额外恢复进程包揽 ECAM、IOMMU 与全部设备协议的方案；那会重建冻结版总管。
  不采用“先 Open 再转授即可保证独占”的论证：现有 manager 恰好保留了这种早期映射，
  ONLY 的后续 Open 限制不等于旧映射失效，必须显式审定实际可写者与恢复顺序。

用户确认推荐方案：实际设备执行实例 α 可死亡，设备专用恢复实例 β 持续存活。
PCI、IOMMU 分别承担本域控制；不扩展稳定恢复执行方自身死亡的框架。

#### Driver 迁移返工：原语与顺序（已确认）

以下是用户态语义操作，不新增系统调用、内核确认凭据或恢复总管。
构造、安装、供给、身份和通用任务控制沿用既有原语，不再造一套启动/关闭协议。

| 所属核心 | 原语 | 独立效果与完成条件 |
| --- | --- | --- |
| Hub | `enroll` | 接收经授权 PCI 来源的发现结果，沿既有设备账登记；不接受裸 VA 或自报角色作为授权 |
| Hub | `claim` | 为具体执行实例绑定设备及恢复侧，签发本次不复用的绑定编号；恢复条件未备齐时不开放业务 |
| Hub | `close` | 关闭本次绑定的新请求与发布入口，开始恢复；重复请求继续同一责任，Fault 可覆盖 Normal，不反向恢复 |
| Hub | `inspect` | 查询同一绑定的协调进度或原始失败，不重发已在途硬件命令；断线不等于取消成功 |
| Hub | `unbind` | 只在本次责任全部结清后移除绑定、允许新认领；执行实例死亡不能直接走此步 |
| PCI | `place` | 复用 Endpoint 的 BAR 落位能力，只开放 MEMORY，不开启 MASTER |
| PCI | `enable` / `disable` | 分别设置及清除 MASTER 并回读；enable 受本次发布准入限制，disable 不等于端点无在途 |
| IOMMU | `acquire` | 用既有 Acquire/Retain 在本任务表中取得必要帧引用，准备本次设备映射；先预留元数据，成功后引用只有一个持有方 |
| IOMMU | `publish` | 使本次翻译可达并确认发布所需命令完成；先记可能可达再写硬件，部分失败必须走停止链 |
| IOMMU | `invalidate` | 撤销设备翻译并完成 IOTINVAL/IOFENCE；只证明本步骤，不释放帧，也不宣称端点排空 |
| IOMMU | `quiesce` | 在端点停止后关闭本轮队列并回读，确认相关命令/故障/页请求队列状态；保留原有错误判据 |
| IOMMU | `release` | 在完整停链完成后用既有 Release 解除本次引用，随后关闭本地页资源；任一步失败仍保留剩余责任 |
| 设备恢复 β | `attach` | 在发布前取得精确 BAR 恢复映射和设备能力布局，关联本次绑定及执行实例；不接收其他进程 VA |
| 设备恢复 β | `stop` | 业务 CPU 控制已切断后，执行本设备 reset/drain 并回读，确认本平台所支持的端点无在途条件 |
| 设备恢复 β | `detach` | 停链完成、执行侧授权已回收后关闭恢复映射和本次设备恢复状态；失败不能先报绑定可复用 |

`place`/`enable` 不合并；`invalidate`/`quiesce`/`release` 不合并。
这些分界分别承载“配置不授权发起”“撤翻译不等于排空”“停止成功不等于资源关闭成功”，
不是为了命名对称增加空操作。构造、交付和失败回滚仍使用既有 Assembly/Supply 操作。

正常发布顺序：

1. Assembly/Control 安装独立实例；Supply 定向交付所需来源及权限。
2. PCI 落位并核 MASTER 关闭。β attach，持有恢复映射后才向 α 交付精确 BAR 子授权；
   α 不获得 ECAM、IOMMU 写权或 BAR 再授出权。PCI 不为此持设备协议运行状态。
3. IOMMU acquire 必要帧引用，再 publish 翻译并等待完成；Hub 的本次绑定在各前置条件
   全部满足后才允许 PCI enable，随后放行业务。不得凭客户端转述的 ready 布尔值开 MASTER。

正常关闭与 α 死亡共用以下顺序：

1. Hub close，本次绑定立即拒绝新业务、重新发布和执行侧恢复运行请求。
2. Control 对 α 的整个执行域完成 CPU 停止；已有死亡状态仍需核实际停执行条件。
   复用既有 Debark/终止控制及其等待责任，不仅暂停一个线程；失败不得假装 BAR 已不可写。
   这一步允许停止或终止 α，但不能拆掉稳定恢复侧的任务、命名空间、能力和供给。
3. IOMMU invalidate → PCI disable → β stop → IOMMU quiesce。
   保留现有 DDTP Off、失效及 fence、清 MASTER、reset/drain、队列回读的硬件先后关系。
4. IOMMU release，回收 α 的 BAR 授权及映射，β detach；各方实际清理完成后 Hub 才 unbind。
   Assembly 随后完成剩余安装清理。稳定恢复侧自身退出也必须先结清其服务的所有绑定。

BAR 的权限选择是显式保留恢复映射、再给执行侧精确子授权；ONLY 不代表已有映射消失。
β 在正常业务期间不写控制寄存器，接管前必须完成第 2 步，避免两个实例并发改设备。
不以 Pie Revoke 返回成功单独证明 CPU 映射失效；现有 cull/withdraw 含异步清理责任。
下一关须将整个执行域的停止条件、BAR 回收结果和跨 Hart 等待落到准确签名及既有返回值。

所有跨 Driver 结果都核实际发送者、获授权入口及本次绑定编号；旧绑定的结果不能推进新绑定。
重试沿原绑定、原命令进度进行，不重置超时或再开 MASTER。持有超时不是最终完成；
恢复后必须验证引用、映射和来源占用实际回收。当前硬件停链保持单端点范围，
不把全局 DDTP/队列停止宣称为可并行服务多设备的逐设备隔离。

用户确认上述原语拆分、显式 BAR 恢复映射及关闭顺序，进入签名关。

#### Driver 迁移返工：签名与失败合同（已确认）

下表是认证后的核心方法签名，均隐含对应核心的 `&mut self`；只读 inspect 为 `&self`。
RPC 适配核授权入口与内核发送者，解码后调用核心；不在报文中接收可信 caller。
表内 `Fail` 指各所属 API 的失败域，不合并成一个跨所有控制器的大枚举。
异步操作复用 `schedule::Progress` 的 Pending/Done，不增加第二种进度包装。

必要的协议坐标：

- `DeviceId { pci: TaskId, requester: u16 }`：PCI 实例及其发现的 requester，不代表访问权。
- `BindingId { hub: TaskId, serial: u64 }`：Hub 签发的绑定身份，serial 不复用，耗尽终判 Full。
- `ClaimId(u64)`：调用方本次 claim 的重试编号，按真实调用者隔离，重试不另分配；
  这是交付请求的关联号，不是停止确认凭据。同编号同参数返回原 BindingId，不同参数 Conflict。
  已完成请求使用有界记录与拒绝旧编号的水位，不因回收重放而重建旧绑定。
- `Discovery { requester: u16, name: String, class: String, line: u32 }`：
  设备发现描述。PCI 来源取自内核发送者及获准登记入口，不能由该描述自报。
- `BindingStatus { state: BindingState, failure: Option<Failure> }`：
  BindingState 为 Preparing/Active/Closing/Closed；Failure 保存实际失败方、操作和原失败码。
  状态只属 Hub 的协调账，不复制 Driver 的寄存器值、队列游标或帧引用计数。

| 核心 | 签名 |
| --- | --- |
| Hub | `enroll(from: TaskId, device: Discovery) -> Result<DeviceId, Fail>` |
| Hub | `claim(from: TaskId, request: ClaimId, device: DeviceId, sensor: PieToken) -> Result<BindingId, Fail>` |
| Hub | `close(binding: BindingId, reason: Close) -> Result<Progress, Fail>` |
| Hub | `inspect(binding: BindingId) -> Result<BindingStatus, Fail>` |
| Hub | `unbind(binding: BindingId) -> Result<(), Fail>` |
| PCI | `place(binding: BindingId, device: DeviceId, bar: u8) -> Result<Endpoint, Fail>` |
| PCI | `enable(binding: BindingId) -> Result<Progress, Fail>` |
| PCI | `disable(binding: BindingId) -> Result<Progress, Fail>` |
| IOMMU | `acquire(binding: BindingId, source: AcquireSource, range: DeviceRange, access: Access) -> Result<Progress, Fail>` |
| IOMMU | `publish(binding: BindingId) -> Result<Progress, Fail>` |
| IOMMU | `invalidate(binding: BindingId) -> Result<Progress, Fail>` |
| IOMMU | `quiesce(binding: BindingId) -> Result<Progress, Fail>` |
| IOMMU | `release(binding: BindingId) -> Result<Progress, Fail>` |
| 设备恢复 β | `attach(binding: BindingId, execution: TaskId, bar: PieToken, pages: usize, caps: Caps) -> Result<Progress, Fail>` |
| 设备恢复 β | `stop(binding: BindingId) -> Result<Progress, Fail>` |
| 设备恢复 β | `detach(binding: BindingId) -> Result<Progress, Fail>` |

`Close`、`DeviceRange`、`Endpoint`、`Caps`、`AcquireSource`、`Access` 复用现有语义，
不另造同义类型；需要跨进程的纯数据契约进入所属 api，核心不依赖 System 私有模块。
Caps 的线上载荷沿用现有四项 capability 布局值，接收侧按实际 BAR 大小重建并校验，
不跨进程传 Dock/View/Pages 或本地地址。Endpoint 仍是 PCI 的落位结果，不成为第二份分配账。

各签名的关键前置及完成条件：

- enroll 对同一 PCI/requester 的一致重试幂等，不允许活绑定期间更换来源或设备描述。
  claim 的执行者为 from，核 sensor 确由它转授；恢复侧由可信安装及授权发现确定，
  不是调用者提交任意 recovery TaskId。创建 Preparing 记录后即可返回 BindingId，
  返回编号不是获准执行业务；只有 inspect 的 Active 才表示发布前置已满足。
- Hub close 不等待失联 α 的合作回复。unbind 只接受本次 Closed 记录；
  关闭、解绑的幂等结果须在同一请求命名空间中保留，旧编号绝不能匹配新绑定。
- PCI place 的 aperture 来自自身获授的来源，不接受客户任意地址窗口；
  device.pci 必须是本实例，绑定与 requester 必须一致。enable 仅接受获准协调入口，
  且只在本绑定仍可发布、恢复映射与 IOMMU 发布完成时执行；不接受客户的 ready 布尔值。
  重复 enable 不能使 Closing 绑定重新开放。
- IOMMU acquire 的 source 是 IOMMU 自己表内的授权/引用，不能直接使用另一进程的
  FlatReference。RPC 适配只接受经实际交付核验的本地来源，内部调用才可复用本地 Reference。
  DeviceRange 是 IOVA 字节区间，source 的 offset/count 是页坐标；
  区间必须页对齐、长度匹配且不溢出，Access 非空且只含所需读写位，不扩大来源权限。
  retention 来自本实例定向供给，不接受请求指定任意 retention token。
- 同一绑定 acquire 的同来源、同 IOVA、同权限重试复用原记录，不重复 Acquire；
  同区间参数冲突拒绝。publish 之前备齐控制表、队列及数据页的必要引用，
  请求中的数据页不能冒充整套 IOMMU 自身页资源；发布途中出错仍归该绑定的停止责任。
- β attach 的 bar 是 β 表内实际交付的精确 BAR token，Open 大小必须等于 pages×PAGE_SIZE，
  Caps 再按此大小验证；在首个可能失败的映射/再交付步骤前预留清理记录。
  α 子授权不给 VEST，β 的恢复映射必须在 α 子授权交付前取得。
  stop 仅接受本次绑定已经停 CPU、撤翻译并清 MASTER 的协调顺序，不能凭请求中的 stopped=true 接管。
- 所有 Pending 都保留句柄、命令身份和原截止时间；Err 保留原失败及未结责任，
  不通过普通 Drop 丢弃已取得的引用/映射。成功的子步骤不重复释放，失败的子步骤原地续办。
  传输超时只表示未得到结果；同一操作重试或 inspect，不能自动当成回滚成功。

CPU 停执行使用既有接口，但不能照搬现有静态 Debark 的完成口径：

```rust
env::unit::debark_team(team: TeamId) -> UnitResult<()>
env::unit::oust(team: TeamId) -> UnitResult<()>
```

活 Team 的 debark_team 返回 Ok 才表示整棵执行域暂停且无成员仍在 Hart；
Busy 必须等待后重试。现有静态 `lifecycle/debark.rs` 只调用 debark_task，不能用作这个证明，
应按 Control 已登记 Team 调整到与动态实例一致的整域口径。已 Doomed 的 Team 会拒绝 Debark，
不能将 Denied 当成已停；死亡路径以既有终止/退休流程最终 Oust 成功作为更强的完成条件，
仅在 Control 自己记录过成功后才清除 Team 身份。恢复侧不属于 α 的 Team 子树。

Control 的公共目标仍是获授权实例，不把 BindingId 或硬件步骤塞进通用任务命令。
已有关闭作业负责拒绝 α 的恢复运行与重新认领；正常恢复不得在该作业之外直接 Embark。
Hub 与 Assembly 的等待必须是可续办调度步骤，不能在 System 等 Hub 回复时阻塞 Hub 所需的
Control/身份/名称空间服务。失去任务观察权限或得到 Unknown 均不是 CPU 停止证明。

资源清理需修正现有 Pages 内部辅助方法，使关闭失败后仍可续办；这不是新增公共接口。
替代现有消耗整个对象的关闭方式，失败时句柄仍保存在该绑定内。
View::close 与 Reference::release 已是 &mut self；
按实际持有句柄逐项清除，只在对应步骤成功后移除所有权。
构造中途失败同样挂回该绑定的清理记录，不允许只返回字符串错误并遗失先取得的 Pole。
α 的 BAR 授权回收与 CPU 映射结清分别核结果；活 α 先整域暂停，随后终止并完成 Oust，
才能将其残余映射记为清理完成，不拿 Pie Revoke 成功代替 TLB/Space 退休。

实现验收保持：独立 Task/Space、越权与旧绑定拒绝、发布前/后失败、α 死亡、
停止确认暂时失败仍持有、恢复后最终净回收；额外覆盖多线程 α 的跨 Hart 接管门，
以及清理失败后同一对象可重试。用户澄清 Pages 的性质后要求继续，进入实施；
不将内部辅助方法的 Fail 占位符作为新增公共失败类型。

#### Driver 迁移返工：基础修正检查点（未完成整体迁移）

- 编辑前将冻结版已跟踪差异与未跟踪源码封存在
  `target/driver-recovery/realign-checkpoint/{tracked.patch,untracked.tar.gz}`，
  SHA256 分别为 `e85b75d108b1ae2caa48ee2c4c9275646c6c3c24e58fc6119586f829548cefa2`、
  `57054924d8bb28c53135cb1e4047c5464de3ca56b7648c94c24b5ad628519c94`。
  未接入旧 worker 的窄修复，未删除冻结源码或改动稳定提交 `3c5e5bb6`。
- 现有 Pages 不新增包装类型；取得 Pole、Reference、View 后立即保存句柄，
  由调用方先持有对象再执行 take，中途失败也能清理。关闭以 &mut self 逐项续办，
  只有成功后才移除对应句柄；Pole release 的 Busy 留给下帧，不在一次调用中无限重试。
  保留内部既有字符串失败信息，不增设公共 Fail。页数零值、乘法溢出及重复覆盖活资源拒绝。
- 配套调用方在构造失败后保留同一对象，在关闭失败后继续推进，不回失败回复后直接退出。
  已开始关闭的页不能再用于完成记录/MMIO证明；清理续办走既有 range 退出后的分支，
  不另加一份 closing 标志。冻结 manager/run 仅做这些调用兼容与责任保存，不认可其架构。
- 静态 Debark 改用 Control 已登记的 Team，Busy 保持 Pending；静态恢复也成对改为
  Embark Team，否则 Team 暂停后只恢复代表线程不能重新运行。缺 Team 不猜测成功，
  动态实例原有整域暂停口径不变。
- 新宿主集合 `programs/tests/iommu-pages` 直接编译生产 Pages，以独立夹具注入
  获取、映射、导出和逐项清理失败，共 12 项通过；验证失败期间所有权留存、
  清理顺序、不重复释放及恢复后夹具账归零。这是模拟环境证据，不是实际物理帧净回收证明。
  system-shape 19 项、retention 6 项通过；新增静态暂停/恢复检查是源码结构检查，
  不冒充多线程跨 Hart 实跑。
- 统一关闭增量编译、offline 运行 workspace dev/release 与 kernel tests 三项检查通过。
  首次 dev 检查发现 iommu-probe 的旧调用遗漏，修正后重跑通过，初次失败日志保留。
  release 六条既有 kernel 警告保留；旧迁移的 session unused_mut 已随调用文件修正。
- 使用 `nu scripts/qtest.nu --package kernel --scene <scene>` 实跑 accept、product、
  system-fault、iommu-probe、iommu-dma，五项通过；没有修改场景断言。
  完整健康集为 **118 passed、1 failed**，唯一失败是缺 initrd 的 scene 哨兵，原始退出 101。
  日志统一在 `target/driver-recovery/realign-foundation/`，宿主仍共用 `target/host-tests`。
- 主线程完成本增量差异自查、局部 rustfmt 与 diff check；未独立复审，
  未运行四 Hart race 或多线程执行方的接管定向场景。
  DeviceAssembly 替换、独立 PCI/设备恢复程序、Hub 安全绑定及真实退休门尚未接入；
  iommu-product 的已知旧迁移回执失败不由本检查点消除，不称整体迁移通过。
  本检查点未提交、推送或合入共享分支。

#### 常驻恢复与声明供给增量（仅供给路径已验证）

- 用户确认采用独立常驻恢复 Driver：α 的退出与一笔 DMA 责任结清不要求 β 退出。
  β 不在 α 的 Team 子树中；Hub 结清全部责任后才允许整机终止常驻任务。
  撤回为此增加通用 ControlConfig.close 的提议；现有 Resident 声明不能单独替代这一结清门。
- 撤掉未接入的 CapabilityResource 草稿，不新增能力后端。
  既有 CollectionResource 已能交付 Nole/Pole 及所需权限；本增量在现有可信引导来源中
  注册 `memory/retention` 集合，复用私有租约根、定向叶授权、描述帧及既有回收。
  普通资源清单不因此获得 Retain；只有可信装配选择的声明绑定才得到该集合。
  集合主 Pole 只给 FETCH，Retain 叶精确给 FETCH|VEST，不提高其他 Call 的交付权限。
- InitReceipt 保存新增来源分支，构造失败沿现有清理回滚；Supplies 的来源根字段改名为
  source_roots，统一保存清单及 Retain 来源，不新增另一份所有权账或永久保留根。
- 复用既有 probe-lease 程序增加真实装配验证，没有新增 bin：
  UnitFile Binding → Assembly 安装 → Supply 集合交付 → 接收者表内 Retain 子授权
  → 真实 Acquire/Traverse/Release。核历史 owner 仍为 System 且不等于接收者，
  两个真实 PA 非零、页对齐且不同；缺 VEST 的同对象子授权和伪造普通 Nole 均精确 Denied。
  引用及来源显式释放成功；此处不宣称完成整机物理帧净零计数证明。
- 首次 accept 的嵌入用例通过，但 wrapper 正确拒绝 probe-lease 未成功及缺少新证明：
  OnSpawn 让装配作业在消费供给描述前释放了描述通道。该消费者改用现有 Ready::Signal，
  消费完描述后才报 Ready。第二次因测试把历史 owner 错写为 kernel 0 而真实失败；
  改为核 System owner 并保留 owner 不等于接收者的断言，未删除断言或放宽内核判据。
  初次与第二次现场分别保留在 accept/、accept-ready/，后续通过另存 accept-final/。
- qtest accept wrapper 增加声明供给成功证明，不以嵌入 scene 的成功代替探针实际完成。
  最终三项 offline cargo check 通过；宿主 supply 23、system-shape 19、retention 6、
  iommu-pages 12 项共 60 项通过。accept、product、system-fault、iommu-probe、iommu-dma
  五场景通过；健康集仍为 118 passed、1 个缺 initrd 哨兵失败，原始退出 101。
  日志在 `target/driver-recovery/declared-retention/`，宿主共用 target/host-tests。
- 本增量未改变内核 ABI、Release 判据、Hub 绑定状态或 Control 的终止规则。
  真实 Driver 的声明交付替换、常驻装配及 Hub 结清门仍未接入；专用 DeviceAssembly、
  三个场景变体 bin 和旧 iommu-product 回执问题仍在，不能据此称整体迁移完成。
  尚未独立复审，未实跑跨 Hart 接管；未提交、推送或合入。

#### 实际 IOMMU 进程使用 Retain 租约（已验证，仍非整体架构交付）

- 将 Supplies 中唯一的租约获取逻辑抽为 acquire，既有 Service 描述交付与当前设备调用适配
  共用该逻辑；不新增 Source 后端、内核调用、公开入口或第二本租约账。
  IOMMU 声明绑定 `memory/retention`，当前装配适配调用同一绑定获取租约；
  Receiver 与 probe-lease 共用 driver/shared/retention 中的读取与核验。
- 撤掉私有启动收据的 Retain token 字段及对应直接 Accord。收据由九字降为八字，
  Driver 从自身表内收到的只读集合和 FETCH|VEST 叶授权取得 Retain。
  其余设备能力及生命周期仍走旧 DeviceAssembly，尚未宣称接回完整 Assembly 启动。
- 修复先前真实失败的完成回执：两字载荷按两字存储，解析拒绝未知 bool；
  System 只接受指定 Driver 的成功回复，Unsettled 或无回复退出不再正常 Done。
  Driver 也核启动/关闭消息的真实发送者。该修正不引入通用 ControlConfig.close。
- 移交负证改查 System 原始 token，并在 Driver 启动前精确核 HandedOver；
  不再查接收表 token，也不把任意错误打印成成功。意外可打开或其他错误均回滚并报告失败。
- 首个 fault 场景嵌入用例通过，但 wrapper 因缺失 Fault 关闭证明正确拒绝；
  旧 closing 标志已在杀执行方前置位，掩盖了之后真实发生的 Open→Closing(Fault)。
  日志改由既有 Holds 阶段变化决定，没有新添 printed 标志或删除场景断言。
  初次失败现场与后续通过现场分别保留，均在 target/driver-recovery/driver-retention/。
- 新增五项真实协议编解码宿主回归，以及供给单一路径的结构检查：
  supply 23、system-shape 20、retention 11、iommu-pages 12，共 66 项通过。
  三项 offline cargo check 通过。accept、product、system-fault、iommu-product、
  iommu-product-fault、iommu-product-start-before、iommu-product-start-after、
  iommu-probe、iommu-dma 九场景全部通过；均以完整 qtest wrapper 成功为准。
  健康集为 118 passed、1 个缺 initrd 哨兵失败，原始退出 101，不称全绿。
- 正常及故障 DMA 现场核不同 Task/Team、真实数据写回、完整停链回读、页关闭及成功回复；
  fault 现场先见 holds=1、Closing(Fault)、Pending，之后才真实确认释放。
  本增量未新增整机物理帧净零计数证明或跨 Hart 接管验证，未独立复审。
- Driver 仍为旧单轮进程，场景变体 bin、PCI/设备恢复混合、DeviceAssembly 及 Hub 缺口仍在。
  不把九场景通过称为常驻 β 或架构迁移完成；未接入冻结 worker 的检查点，
  未提交、推送或合入共享分支。

#### 新组主线实施（已接回第四轮，构建及安全阻塞未关闭）

- 用户明确同意启用新的 DeepSeek namer/worker/reviewer 组，不重开已确认的设计关。
  三者均使用 DeepSeek V4.1 Flash / max；reviewer 在首轮实际代码接回后启动，
  不以对旧原型的检查冒充新实现审查。
- 唯一代码 worker：
  `ksQQwKxhEIDfRmKsYfFDQ_ymxZLOANnh_JIBxBBnK9ozTelNwroye-tIjWUe`。
  只读 namer：
  `ksQQwKxhEIDfRmKsYfFDQ_ymxZLOANniD5IBxBBnK9ozTelNwroye-tIjWUe`。
  旧 worker 仍冻结，未合入旧窄修复快照。父线程只维护本计划、封存检查点并统一构建验证。
- 交付合同：正常 Assembly/Control 装配独立 PCI、IOMMU、设备恢复 Driver；
  独立常驻 β 不随 α 或单笔责任退出；Hub 接真实绑定与恢复协调，
  正常及超时退场均在全局 Slay 前结清；删除 DeviceAssembly、私有启动收据及三个测试 Driver bin，
  不把旧总管换个位置，也不只交一笔辅助或供给演示。
- 已明确禁止恢复 CapabilityResource、ControlConfig.close、额外内核调用、全局持有根或确认凭据；
  不扩展恢复者自身死亡框架，不放宽 Source 准入或最后回收判据。
  worker 不运行构建/QEMU或创建缓存，不提交推送；namer 只核实际接入点与命名，直接向 worker 传确认事实。
- namer 已完成只读核对并直接交 worker，未改代码、未运行构建或场景。
  已向 worker 补充覆盖边界：静态 retire hook 可作显式退役的前置门，但动态 Retire 当前在退出后触发，
  settling/owner 死亡路径可能更早 Slay；不能只加 retire hook，正常及强制全局退场也须先结清，
  并防止显式静态 Ruin 绕过保护。此核对不是新实现的 reviewer 验收。
- 分组前源码封存于 target/driver-recovery/mainline-group/：
  before-tracked.patch 的 SHA256 为
  `ee452243668ad98e75687b55a42b84605fa1132a798f22b6b09133a5a926cfaf`；
  before-untracked.tar.gz 为
  `4e04f9e061dd1cd8a388628b270e9d62d3eba5b54230b8fb011d378f8dff5cdc`。
  HEAD 仍为 3c5e5bb6，分支 feature/driver-recovery；这只是保存已有未提交现场，不是验收或提交新主线。
- 父验证仍以源码检查点为单位：三项 offline cargo check、实际宿主集合、
  九个既有场景及实际新增的 α 死亡/β 存活/确认失败保留/恢复回收用例；
  多线程跨 Hart 接管需独立证据。旧九场景与 66 宿主通过仅为分组基线，
  健康集旧缺 initrd 哨兵仍红，不预填新组结果。
- 已接回同一 worker 的第四轮固定版本
  `b8a8bde1-b105-4b16-90c7-506bea7ff758.14452610`：
  声明装配三个常驻 Driver，场景移入 Harness，α 改为动态实例，
  IOMMU 恢复并实际调用生产 Holds，未绑定时 DDTP 保持 Off。
  这些是源码改动，不是运行验收结论。
- 第一轮独立 reviewer：
  `ksQQwKxhEIDfRmKsYfFDQ_ymxZLOANtsWJIBxBBnK9ozTelNwroye-tIjWUe`。
  首审确认兄弟任务 join 错当已停、静态 α 误走动态实例面、DMA 认领无真实授权、
  删除生产持有账及失败回滚丢责任等阻塞，均交同一 worker 返工。
  后续只读核对确认 State::Dead 不等于全 Team 已退出，公开 bond 也不是设备授权。
  第四轮不可变源码已交同一 reviewer 复核，未取得最终批准。
- 父线程第四轮真实检查记录在 `target/driver-recovery/mainline-group/round4/`：
  `cargo check --workspace --offline` 退出 101，programs 有 37 个编译错误；
  `cargo check -p kernel --tests --offline` 通过。
  宿主 hub-api 3、system-shape 20、supply 23、iommu-queue 15、iommu-pages 12 项通过；
  retention 编译失败，未执行用例。完整错误已回送同一 worker，未删除旧持有安全断言。
  本组尚未运行 release 检查、QEMU 或健康集，不沿用分组前通过记录。
- 补充真实检查：`system-api` 的 x86_64 宿主 cargo check 通过；
  `crates/image` 同目标 cargo check 退出 101，四个错误与上述 catalog 旧名及静态数组寿命同源。
  日志仍在 round4。worker 随后遇到 OpenCode 提供方失败，同一线程有界恢复一次仍失败，
  未切换模型、未新开写入线程、未继续自动重试。
  失败前增量保留在原 worker 的固定版本
  `6e1b52f4-708a-4f04-8574-4832b779ec82.14465255`，
  尚未接回或验证，不计作已修复。
- 同一 reviewer 已完成第四轮不可变快照复核（静态，未运行构建或 QEMU）：
  动态 α 的 owner/sire 确为 Hub，内核整域操作权限成立；但 Control 在 Ruin 回信前已 Oust，
  Hub 之后再次 Oust 必得 Denied，使 StopCpu 无法完成。
  IOMMU 在 register 成功后的物理页列表分配及 ReferenceRange 构造失败路径，
  仅 forget 无法移除仍持引用的 scope，会使同号重试永久 Conflict。
  故障景的重绑探针由静态 controller 直接认领，与新的 owned_instance 准入冲突，必被拒绝。
  另确认公共 DMA 面的 Close/Settle 未核发送者，可被未授权调用方强关绑定。
  上述问题均未关闭，不把 B2/B3 的局部修复或宿主通过当最终批准。
  reviewer 未取得 kernel/tests 快照；父线程已实际运行 kernel tests check 通过，
  这仅证明该目标编译，不是健康集运行结果。
- 接回时 hub-api 导出块出现合并标记，父线程仅消解导出去重冲突；
  不代替 worker 写业务实现。BAR/ONLY 重绑及对偶清理、部分取得与未知结果责任、
  实际整域停权限、真实设备授权和陈旧 IOVA 请求负证仍须关闭。
  本轮未提交、未推送，HEAD 仍为 `3c5e5bb6`。

#### 父线程接手检查点（主线场景已通过，尚非最终验收）

- 用户在提供方失败后明确要求“由你接手”，后续继续实施由父线程单写。
  已接回失败 worker 的固定增量
  `6e1b52f4-708a-4f04-8574-4832b779ec82.14465255`；
  上节“尚未接回”“父线程不写业务实现”是当时的历史状态，不再是当前状态。
  未重启失败 worker、切换模型或创建第二个写入者。
  旧 reviewer 的结论仍只适用于它实际审过的第四轮快照；没有当前接手代码的独立批准。
- 生产路径改为声明装配的常驻 PCI、IOMMU、设备恢复 Driver，Hub 只记绑定并协调。
  旧 DeviceAssembly、System 私有设备启动链、单轮 DMA service 和测试专用 Driver bin 已删除；
  场景控制与变体在 Harness。普通 product 不声明不存在的 IOMMU 平台资源，
  IOMMU 产品景才选择这组 Driver；没有用另一条 Loader 启动旁路补装。
- Hub 的认领与整机结清使用同一本 Ledger：主人死亡只转 Closing，不空出设备；
  Close 必须来自绑定 α，Settle 必须来自 System，整机结清永久禁止新认领。
  收场游标只前进，不被重复 Close/死亡探测/Settle 重置。
  当前认领历史是硬上限 32 条，不淘汰旧编号，也不把它描述成可无限重绑的水位压缩。
- α 是真实动态实例，owner/sire 为 Hub。Hub 从 Control 的 StateInstance 查询真实 Task/Team，
  不再信构造方报文自报的 Team；对应关系先成功入账才允许 Embark，异 Team 重放冲突。
  Control 的 Said 因此从 19 字节扩为 27 字节，新增 TeamId 编解码；
  所有同镜像调用方一起重建，旧宽度答复必须拒绝，没有增加操作码或内核调用。
  Control 在委托 owner 仍活着时保留子团队回收收据，包括尚未 Claim 的实例。
- StopCpu 先 Slay 整个 Team，再等 Oust 成功确认；只 Join 主线程或只调用 Oust 都不够。
  其后依次撤翻译并 fence、清 MASTER、恢复方 reset/drain、关队列、解除引用与清绑定。
  PCI/恢复方保留有界的设备 BAR 基线映射，α 只拿本次精确子窗口且无 VEST；
  不让 β 与仍运行的 α 争 ONLY 写映射。正常、强制全局退场以及静态 Driver Ruin
  都增加结清前置门，不能在结清之前 Slay 这些提供方。
- IOMMU 真实恢复生产 Holds。引用取得前预留物理页列表，register 成功后立即登记部分取得责任；
  发布失败保留原记录，旧 serial 不能晚到取得或推进新绑定。
  二代绑定的真实旧 IOVA 提交曾击穿哨兵：根因是复用非叶页表时留有旧路径。
  已改为 DDTP 确认 Off 后清 DDT/各级表，填完新表并同步才开启 DDTP。
  每次最终 release 都实际回读 DDTP/队列/帧持有，包括从未 acquire 的责任，
  不用“没有登记”替代硬件已停证明。
- Harness 每代 α 都有同 Team、不会自行退出的辅助任务，控制器核它亲自转授的报活孔。
  故障景先真读，再让 α 主线程退出，等 Hub 整域停止与结清，构造第二代真实 α；
  第二代实际敲铃提交旧 IOVA 描述符，要求哨兵与状态不变，并读到对应 requester 的 IOMMU 故障。
  当前 QEMU requester 为 8，不再用固定设备号 0 作为判据。
- 最后回收增加实际物理分配基线核对：启动用户任务前记录
  `frame_occupied - block_occupied`，调度器回收并冲刷堆块后要求严格相等。
  这里计的是物理分配对象，不是页数；先预热永久 ASID 位图，避免把一次性内核元数据当泄漏。
  此检查发现 ToleAwait 在可能不返回的调度调用外仍持 Task/Pie：
  改为先退出资源查询闭包、释放强引用，再交调度器等待。
  Join/Fall/自停/中断调度入口同样先放下调用者强引用，不改 Source 准入、ONLY 或回收断言。
  内核 doom 四项是累计终止分支计数，不是残留资源数；
  DMA wrapper 的旧 doom=0 误判改用物理基线，memory-transfer-race 的既有 doom=0 断言仍保留。
- 当前统一验证目录：`target/driver-recovery/takeover/final/`。
  六项 offline cargo check 均退出 0：
  workspace dev/release、kernel --tests、programs --all-targets、
  system-api x86_64、crates/image x86_64。
  全部 34 组独立 programs 宿主配置共 311 tests passed、0 failed；
  扩大检查曾发现 capability-import、control-instance、device-api、interface-marks 四组漏迁，
  已补普通设备字段、真实 self_id 夹具及新增 DMA 发布项，保留旧标记值、碰撞与授权拒绝断言。
  新增回归覆盖可信 Team 查询、委托收据保留、结清后禁入、游标单调和历史满容量。
- 九场景均用实际命令
  `CARGO_INCREMENTAL=0 TRACE_OUT=<独立目录> nu scripts/qtest.nu --package kernel --scene <scene>`
  退出 0：accept、product、system-fault、iommu-probe、iommu-dma、
  iommu-product、iommu-product-fault、iommu-product-start-before、iommu-product-start-after。
  故障产品景最终物理分配为 85 → 85；第二代 fault queue 实读 faults=127、last_did=8。
  `nu-check scripts/qtest.nu` 与 `git diff --check` 通过，没有全树格式化。
- 四 Hart α 接管另跑：
  `CARGO_INCREMENTAL=0 QEMU_SMP=4 TRACE_OUT="$PWD/target/driver-recovery/takeover/final/multihart-fault" nu scripts/qtest.nu --package kernel --scene iommu-product-fault -- --features semihosting`
  wrapper 退出 0；保存 console 与 events.jsonl，并由
  `programs/tests/device-recovery/trace.py` 核对两代主/辅助任务在不同 Hart 的实际调度现场及完整 reap。
  第一代主 12/辅助 14，第二代主 15/辅助 18，两代均回收；
  全部 Hart 号改成 0 的负对照被拒绝。它不宣称证明连续同时执行的时间区间。
- 健康集命令
  `CARGO_INCREMENTAL=0 TRACE_OUT="$PWD/target/driver-recovery/takeover/final/health" nu scripts/qtest.nu --package kernel`
  实际退出 101：118 passed、1 failed；
  唯一失败仍为未给 initrd 的 scene 哨兵，不称全绿。
  四 Hart 内存转授竞争命令
  `CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true QEMU_SMP=4 TRACE_OUT="$PWD/trace" nu scripts/qtest.nu --package kernel --scene memory-transfer-race --feed-script programs/tests/memory-transfer/race.py -- --features semihosting,race-scene -- scene`
  退出 0，1 passed、118 filtered；
  这是另一项检查，不把它代替 α 接管现场或健康集。
- 尚未关闭：当前接手版本的独立审查；显式静态 Driver Ruin 与强制全局退出的专门在途 DMA 场景；
  故障确认失败后原责任保持的端到端注入；部分取得和回复未知的完整失败责任审计。
  其中 Launch 报文已消费后 Control 查询/Embark/引导交付失败仍缺持久重试，
  enrollment 与非法 BAR 形态的失败清理也未完整验证；
  不能用当前正常与 α 死亡场景通过推断这些边界已闭合。
  单文件多结构等 worker 遗留组织问题也未完成项目准则核对。
  未提交、未推送、未合入共享分支；HEAD 仍为 `3c5e5bb6`。

#### 启动责任与 BAR 部分取得修补（父线程续办）

- 用同一本 Hub Ledger 的 Execution 记录替换仅保存 Task/Team 的 Launched 记录；
  没有新增第二个总管。消费 Launch 前预留容量，重复任务不重设启动期限。
  Control 身份查询、创建引导孔、填数据、交付、收窄、Embark 各步成功后才前进，
  失败保留当前步、已得句柄及首个原始失败。
  引导数据现于 Embark 前准备完成；Embark 答复丢失时只重问同一实例，不重复构造或交引导孔。
- 启动超时、显式结清或主线程退出均转同一退休责任；已 Running 的实例不继承启动超时。
  整 Team Slay、Oust 确认、释放引导能力分开推进。
  Oust 成功后即使能力释放失败也不再次消费回收收据；
  未关闭绑定仍需要 CPU 停止确认时，已停 Execution 记录不提前淘汰。
  未经 Control 确认 Team 的请求不能认领 DMA 设备，Unknown/Denied 也不冒充整域停止证明。
- Settle 现在封闭新启动与新认领，并同时等待执行实例和 DMA 绑定的剩余责任；
  没有第一笔 DMA 认领的部分启动也不能从结清门漏过。
  答复 count 是饱和的未结清责任数，不再仅是绑定数；消费方仍以零为结清判据。
  执行记录容量为同时 256 条；Done 且无未关闭绑定引用时移除，
  与累计认领历史的 32 条硬上限是两件事。
- Control 客户端新增 inspect，StateInstance 的 Task 和状态码统一校验；
  team 仍拒绝零 Team，inspect 可表达可信登记中已退休且无 Team 的实例。
  本轮没有新增操作码、内核调用或进一步改变线上帧宽。
- 恢复方改由 Core 持有唯一 Dock，Device 不再复制 BAR token/base；
  先校验参数再开图，图一开成就立刻登记到 Core。
  实测大小不符且 Shut 再失败时，映射保留，后续 Stop/Detach 必须继续撤同一张图，
  不得先报完成或推进水位。子授权失败复用既有映射，同号异能力表或异执行方重放冲突；
  成功设备的基线图仍跨绑定保留，没有改变 Source/ONLY、β 先得图及 α 无 VEST 的限制。
- 新增 12 个生产 Execution 核心宿主回归：
  各启动边界失败、未知 Embark 结果、每个边界超时、停机失败/未确认、释放失败、
  无绑定主线程退出、部分启动与未知 Embark 下结清、容量及回收收据保留。
  新增 device-recovery 独立宿主套件 5 项，直接编译生产 Core、Dock、virtio 和协议，
  只替换内核资源边界：覆盖非法布局、撤图叠加失败、子授权失败、参数重放、
  真实 volatile 队列回读未静止时拒绝 Detach，以及来源/权限拒绝。
  这些是宿主故障注入，不冒充 Control/硬件故障的完整 QEMU 端到端注入。
- 当前验证目录 `target/driver-recovery/launch/final/`：
  六项 offline 构建检查全通过，35 套独立宿主配置合计 329 passed、0 failed；
  九个既有 QEMU 场景全部退出 0。
  四 Hart 故障产品景及结构化 trace 检查退出 0，两代主/辅助任务均完整 reap；
  故障产品景物理分配仍为 85 → 85，旧 IOVA 提交的故障队列记录 last_did=8。
  健康集重跑退出 101，118 passed、1 个缺 initrd 哨兵失败；
  四 Hart memory-transfer-race 按前节同一完整命令重跑退出 0。
- 尚余的实现与验收不是仅剩审查：
  enrollment 在创建类联盟、建立目录及发布门牌时的部分成功/回复未知责任；
  静态 Driver Ruin 和强制全局退出的在途 DMA 专景；
  硬件确认失败保持原责任的完整端到端注入；
  认领终态记录/防重放水位与 IOVA 编号上限的长期运行语义
  （目前历史累计 32 条，IOVA serial 另受小于 64 的限制）；
  项目结构准则核对及当前代码的独立审查。
  未提交、未推送、未合入共享分支，HEAD 仍为 `3c5e5bb6`。

#### 登记未知结果与有界认领历史（父线程续办）

- 经用户批准，Identity 的 Found 改为携带非零 FoundId；
  以真实调用者 Task 和请求编号定位同一次创建，创建结果随 CoalitionNode 保存在 Identity，
  不新增第二份结果账。相同请求重返原联盟，不再次分配，也不再次消耗创建配额；
  未绑定调用者拒绝，调用者当前主体已改变时返回 Changed，不把旧请求改归新主体。
  Found 仍使用原操作码和授权面，但请求由 9 字节变为 17 字节；
  不兼容旧匿名创建帧，客户端、服务、所有 Harness 调用点与编解码测试已一并迁移。
  没有新增内核调用或 Identity GC。
- Hub 在取得任何外部资源前预留设备与类责任。类记录保存 Found / Admit / Activate / Ready，
  创建成功即保存联盟号；设备记录以 Cell::Enrolling 保存唯一 Entry、已铸设备门及目录进度，
  Part 和 Bind 成功后才前进。回包未知时重用原创建编号、原目录操作编号和同一枚设备门；
  完整发布前不出现在设备查询、门牌窗口或认领入口，也不因“设备已在册”提前返回成功。
  重放同时核 PCI 来源、requester、名称、类别和线路，不允许已完成或部分登记被异参覆盖。
- 自查修正了 Operator 操作编号的另一处合同偏差：不再按 PCI requester 大小分配编号；
  设备行只追加，以预留顺序分配和推进目录发布。前一笔结果未知时不发布后一笔，
  避免新的较大编号使尚未送达的较小编号被 Operator 水位拒绝。
  宿主夹具同时检查单调编号，覆盖 requester 500 先于 requester 1 及 Part/Bind 回包丢失，
  不再假定外部目录接受任意乱序新编号。
- 拆开 Entry、Owner、Cell、ClaimRequest 与 CoalitionNode 的定义；
  没有引入第二个设备总管。普通设备与 DMA 设备仍共用 Hub Ledger。
  生产 Identity 核心与服务的既有宿主入口恢复为可独立运行的 manifest，
  19 项涵盖真实编解码/服务重放、权限、配额和分配失败；
  Hub 增加 7 项登记故障回归，覆盖每个边界失败、重复未知回复、同类登记、
  异参拒绝、容量、结清封闭以及目录编号顺序。
- 删除重复的 ClaimRecord 向量：请求编号直接归 Binding，防重放高水位归已有 Execution。
  新认领必须来自已核实 Team 且处于 Embark / Running 的执行记录；
  失败、Taken 或地址耗尽都不推进请求水位。已完成请求在保留期内返真实状态与原编号，
  不再一律答 Preparing。最近 32 笔结清结果按完成顺序保留，未结清记录绝不淘汰；
  淘汰后 Inspect 返回 Unknown，仍在执行的旧请求由高水位拒绝复活。
  Execution 退休后不再准入新认领，Done 且无开绑定时按既有规则移除，
  不另设随历史 Task 数量无限增长的水位表。
- IOVA 不循环复用。编号上界从任意的 64 改为正 Sv39 地址范围实际可容纳的窗口数：
  `(2^38 - IOVA_BASE) / IOVA_SPAN = 126976`，有效 serial 为 1..126975。
  新绑定建账前检查地址范围；真正耗尽返回 Full，既不回绕编号，也不取得帧引用后才失败。
  两种页表映射入口共用窗口校验，拒绝非规范地址、算术溢出、空窗和跨 2MiB 叶表的范围。
  现有 DDTP Off、清除各级旧路径、发布、invalidate/fence 和停链判据保持不变。
- 宿主遍历全部 126975 个可用绑定编号，核窗口唯一、终态仅保留 32 笔、
  旧请求不复活及耗尽无孤儿；另核未结清记录不被挤出、迟完成结果不会立即淘汰。
  绑定测试现在登记真实形状的 Execution，Settle 必须同时计入执行与绑定责任：
  绑定关闭但 Execution 未 Done 时仍非零，没有放宽结清判据。
  IOMMU 表回归覆盖 64、511、512、最后一窗与拒绝前不写 PTE。
  这些不是 126975 次真实动态 α 启停，也不是完整产品长期 soak。
- 增强真实 iommu-probe：使用 serial 64、511、512、126975 对应窗口，
  每次核硬件译回实际保留页、前一 IOVA 不再可译，以及 DDTP 和命令/故障队列确认停止。
  实测最后一窗为 `0x3fffe00000`，旧窗响应均为 `0x3401`；
  qtest 包装器要求四条对应地址的读数齐全。生产故障产品景仍核真实 DMA、旧 IOVA 拒绝与回收，
  不能用这个单域翻译探针代替多代 α 的产品端到端证明。
- 当前验证目录 `target/driver-recovery/longrun/final/`：
  六项 offline 构建检查全部退出 0；36 套独立宿主配置合计 362 passed、0 failed；
  九个既有 QEMU 场景全部退出 0。
  四 Hart 故障产品景及结构化 trace 检查退出 0，两代主/辅助任务均在不同 Hart 被观察到并完整 reap；
  这不是持续同时执行区间的证明。普通故障产品景与增强 IOMMU 探针的物理分配均为 85 → 85。
  健康集仍退出 101，118 passed、1 个缺 initrd 哨兵失败；
  四 Hart memory-transfer-race 按前节完整命令重跑退出 0。
  git diff --check 与 Nushell 脚本解析检查通过，没有全树格式重排。
- 尚余：静态 Driver Ruin 与强制全局退出的在途 DMA 专景，
  invalidate/fence、reset/drain、引用解除确认失败保持原责任并恢复的完整端到端注入，
  多代真实动态 α 的长期运行验证，以及剩余结构准则核对与当前最终代码的独立审查。
  β 自身死亡仍明确不在本轮合同内。
  本轮完成差异自查，未冒称独立审查批准；未提交、未推送、未合入共享分支，
  HEAD 仍为 `3c5e5bb6`。

#### System 通用入口与显式部署选择

- 经用户批准，删除 System 下四份完全相同的 IOMMU 场景入口及 `scenes.rs`：
  减少 4 个 Cargo bin、4 个 UnitFile 声明和 4 个 Package 登记。
  普通 product 与四个 IOMMU 产品部署共用 `system` 镜像；
  四份部署启动单和 Harness 工作负载、控制器保持不变，不搬入口壳到 Harness。
  本次没有改 Hub 回收、PCI/IOMMU/β 操作、内核调用或驱动协议。
- initrd 清单显式记录部署名和引导项下标，格式标识为 `SQD1`。
  写侧由同一个 `entry_at` 选择清单引导项和 capsule 的源 ELF；
  内核仍只读原有 8 字节 capsule 前言，部署选择由用户态读取。
  旧匿名清单不兼容，必须用当前 image 工具重建 initrd；
  不保留根据镜像存在性猜测部署的回退路径。
- 运行时按部署名查唯一声明，再核引导项的镜像名与特权级；
  未知部署、引导项不匹配、旧格式、非法头和缺失记录均拒绝。
  Catalog 在提供查询前检查全部记录，不因先找到一项便忽略后面的非法记录。
  没有新增运行时部署表或第二套场景管理状态。
- 新增 4 项清单编码测试、3 项实际 Catalog 测试及 3 项 image 回归，
  并迁移原有选景错误测试。覆盖所有部署的打包/选景往返、五景共用入口、
  其他入口镜像共存、错误特权、未知部署、旧头、截断和末尾非法记录。
  测试夹具不模拟 System 选景或 Catalog，只替代读取启动映射的 Accounts 边界。
- 验证目录 `target/driver-recovery/deployment/`：
  六项 offline 构建检查退出 0；env 宿主测试 38 passed（含 4 项新增），
  image 13 项单元测试与 3 项 Catalog 集成测试通过；
  36 套 programs 独立宿主配置仍为 362 passed、0 failed。
  九个 QEMU 命令使用既有 qtest 包装器：
  accept、product、system-fault、iommu-probe、iommu-dma、
  iommu-product、iommu-product-fault、iommu-product-start-before、
  iommu-product-start-after 全部退出 0，物理分配对象均为 85 → 85。
- 补充四 Hart、semihosting 故障产品景首跑退出 101：
  内核报告 `structured export incomplete`，后续 trace 缺少完整 reap 证据；
  即使控制台显示物理分配 85 → 85，也不算该次验收通过。
  原日志和不完整事件保存在 `multihart-fault/`。
  不改代码、参数或判据，同命令复跑退出 0，trace 核两代主/辅助任务完整 reap；
  主任务均在四个 Hart、辅助任务均在 Hart 1 和 2 被观察到。
  成功现场在 `multihart-fault-retry/`，不把复跑成功写成首跑全绿，
  导出失败的具体触发原因尚未定位，不能据此宣称四 Hart 导出稳定。
- 本轮完成差异自查；未运行默认 health、memory-transfer-race 或旧 identity-smoke 脚本，
  不沿用前一检查点的运行结果充当本轮新证据。
  当前最终代码的独立审查和前节列出的 DMA 故障专项仍待完成。
  未提交、未推送、未合入共享分支，HEAD 仍为 `3c5e5bb6`。

#### 多 Hart 结构化导出的争锁截断

- 将导出失败的布尔标记换为同一枚原子首因记录，区分 Encode、LockTimeout、
  Create、Write；第一个原因确定后不被后续失败覆盖，停机时输出首因，
  完整性判据仍要求本轮没有任何失败。没有新增第二份成功状态。
- 定位阶段七次四 Hart 故障产品景中六次通过，第七次退出 101，
  明确报告 `structured export failed: LockTimeout`。
  失败发生在全局宿主文件锁等待，不是本次创建、编码或写文件失败；
  原实现让每条正常事件也使用 10000 tick 的短争锁预算，
  一次超时就永久停止本轮导出。定位现场在
  `target/driver-recovery/export-diagnosis/run-7/`。
- 正常 trace 按同一把锁串行等到实际写入，不再把正常锁竞争当成丢失证据。
  等待中服从现有 stop::hush 停核机制；不另建队列、文件分片或后台总管。
  Panic 事件和崩溃报告仍使用原短预算，避免等在已停住的持锁 Hart 上；
  以时间差核超时，覆盖时钟回绕。创建失败时先放开宿主锁再打印错误。
  写 JSON 和换行仍在同一临界区，任一失败均锁存首因并拒绝验收；
  不能以“机器已退出”代替完整导出，也没有删除 reap、物理回收或完整性断言。
- 新增独立 `kernel/tests/export` 宿主入口，编译实际 export、事件词表与 Report。
  仅替代锁、时钟、停核和宿主 I/O 边界；一项合同测试覆盖超过旧预算的正常竞争、
  四线程 4000 条 JSONL 完整且不重不串、创建失败、换行写失败、首因不覆盖、
  panic 事件/报告持锁不前进及计时回绕。
  这是 1 项测试、4000 条记录，不报告成 4000 项测试；
  编码首因分支以直接置错验证，没有冒称注入了原生序列化失败。
  Panic 有界等待由宿主持锁夹具验证，尚未运行真实内核 panic 停核专景。
  首次夹具构建的模块可见性错误已修正，没有改变生产判据。
- 最终证据在 `target/driver-recovery/export-final/`：
  kernel tests 含 semihosting 的 debug/release offline 检查通过；
  新 export 合同测试通过；既有 trace 宿主测试普通/semihosting 两档各 2 项通过。
  四 Hart 故障产品景连续 10 次退出 0，10 次 trace 检查均通过，
  两代主/辅助任务实际完整 reap，物理分配对象均为 85 → 85。
  这是有限重复验证，不声称所有调度和宿主 I/O 环境下永远不会失败。
- 额外八 Hart 检查：QEMU 场景退出 0，JSONL 含最终 halt，
  物理分配对象 153 → 153；但辅助任务没有被 switch 事件记录到，
  既有 device-recovery/trace.py 因无跨 Hart 执行证据退出 1。
  原 checker 与断言均未修改，该项仍未通过，不用导出完整替代执行覆盖。
- 四 Hart memory-transfer-race 按原完整命令（含 debug assertions、
  semihosting,race-scene、feed-script 和 TRACE_OUT="$PWD/trace"）退出 0，
  成功导出另拷至 `race-exact/`。
  此前误把 TRACE_OUT 改成嵌套的 `export-final/race`，驱动按捕获目录的父目录
  寻找固定名 JSONL，因读错位置报告 peer 未登记；该次不是有效验证，也未隐去日志。
  没有为这个调用错误修改驱动或断言。
- 本轮未运行默认 health、九景全集或新的静态 Driver Ruin 专景；
  没有将上一检查点的结果计为本轮新结果。
  在途 DMA 静态 Driver Ruin、强制全局退出、原生确认失败注入和多代 α 长跑仍待完成。
  八 Hart 执行证据覆盖及当前最终代码的独立审查也未完成。
  已做本轮差异自查，未提交、未推送、未合入共享分支，HEAD 仍为 `3c5e5bb6`。

#### 真实在途 DMA 与静态 Driver Ruin

- 新增 `iommu-ruin` Harness 部署、控制器和入口。入口复用 System 的实际
  bootstrap、资源安装、启动、运行与退场计划；只将原运行循环提为 `supervise`，
  不复制第二套总管，不向生产入口加入选景分支。
  Harness 按真实控制器请求向测试调用任务交付既有 Control Ruin/State 能力，
  调用实际 `service("recovery").ruin(...)`，成功答复后查询必须为 Dead。
  没有扩大公开授权、增加内核调用、设备协议或 DMA 资源类型。
- 真实 virtio-blk 请求由独立只读 NBD 后端保留未回复；QEMU 启动探测读正常放行，
  只在 binding Active 后挂住一个偏移 0、长度 512 的 guest 读。
  doorbell 回报只是请求已提交的证据，不拿它单独证明设备请求仍在途。
  宿主必须实际观察 NBD held，并在 Ruin 停链期间确认 α 主/辅助任务均已 reap、
  静态恢复驱动尚未 reap、IOMMU 失效后仍持 6 个页帧引用。
  100 ms 窗口只观察未提前完成，不把固定等待本身当在途证明。
- 当前 QEMU 在清 MASTER 时就等待未完成块请求，比设备 reset 更早。
  外部后端的放行不依赖这条已阻塞的 QEMU 控制通道；
  放行前不得出现 MASTER 已关、reset 完成、队列关闭、引用释放或 Ruin 成功答复。
  放行后核 NBD 成功回复、实际保留内存中的扇区花纹与状态 0，
  再核完整停链、绑定 Closed、Driver Dead、完整 trace 与物理分配对象回到基线。
  挂起阶段的 trace 必须是本轮最终 trace 的前缀，不能用其他轮的 reap 补证。
- 排空退役不等于正常服务完成：本机此路径读缓冲和状态确实写回，
  但队列停用后的 used index/len 为 0/0。
  `delivery/` 曾错误套用正常完成的 used=1/len=513 断言，宿主验收据此报红；
  现按排空合同核真实数据与状态，记录 used/len，而不宣称正常请求已交付完成。
  既有正常读、故障重绑及旧 IOVA 负证判据均未放宽。
- 失败探索保存在 `target/driver-recovery/ruin/`：
  起初直接使用 UART boot 能力因权限及 HandedOver 失败，最终没有绕过 Supply 或放宽权限；
  blkdebug 暂停后的 QMP 放行也被同步 drain 阻塞，因此改用独立 NBD 后端。
  初版 NBD 误挂 QEMU 启动探测读，后端超时断线虽让原生收尾完成，
  宿主仍拒绝验收；这些 native pass 不是本专项成功。
  中间的 stdout 缓冲假设和 PTY 尝试均已撤回，未保留无效包装。
- 新增 18 项 Python 宿主测试：4 项真实 NBD socket 合同，
  14 项合成证据正反对照，拒绝启动探测冒充 DMA、断线冒充成功回复、
  缺辅助任务 reap、Driver 提前回收、停链乱序、设备错配、
  后端成功但原生 DMA 数据错误、帧引用/物理分配未回收和 trace 截断。
  合成记录与测试代码分开；这些不替代原生场景验证。

#### 等待入队覆盖 Slay 的竞争窗口

- 修复前 `verified/ruin-4/` 原生失败：辅助任务已 reap，α 主任务继续运行，
  Ruin 调用超时，控制器退出后 α 访问窗口触发页故障；最终物理分配回到基线，
  但该次仍明确失败。没有延长调用期限来覆盖它。
- 检查发现 `block_with_call` 在提交锁外检查 doom，另一 Hart 可以在检查后
  写入 Doomed，随后本任务取得锁并用 Waiting 覆盖终止状态。
  将终止检查与入队置于同一个既有提交边界，复用原锁，不新增终止标记或重试总管。
  函数移入同模块的 `wait/block.rs`，便于宿主直接编译实际实现。
- 新增 `kernel/tests/wait-block` 两项合同测试，只替代任务、提交锁、时钟、
  等待表与调度边界。在取得提交锁的边界确定性注入 Slay，
  必须直接返回 Quit，不得登记等待表、闹钟或切换任务；
  普通等待仍须在同一提交段内完成登记和发表。
  临时恢复旧锁顺序运行负对照，真实函数测试退出 101，
  明确报 `Waiting overwrote Slay`；恢复修复后两项通过。
  这是边界替身上的确定性交错，不冒称已在原生内核逐指令注入同一交错。
- 修复后的最终证据目录为 `target/driver-recovery/ruin/closure/`：
  四 Hart、semihosting 的 Ruin 原生场景连续 10 次退出 0，
  每次实际核挂起读、α 全 Team reap、Driver 延后 reap、成功后端回复与原生数据，
  物理分配对象均为 85 → 85。此前被用户中断的 20 次批次不计作完成。
- 九个既有 QEMU 场景全部退出 0；四 Hart 故障产品景的既有 trace checker 通过，
  两代主任务均在 Hart 0/1/2/3 被观察到，辅助任务分别在 Hart 2 和 Hart 1/3，
  主/辅助任务均完整 reap。这不是持续同时执行区间或八 Hart 覆盖的证明。
  默认健康集退出 101：118 passed、1 个缺 initrd 哨兵失败，不报告全绿。
  memory-transfer-race 使用前节原完整命令（含 debug assertions、semihosting,race-scene、
  feed-script、TRACE_OUT="$PWD/trace"）退出 0，成功现场另拷至 `race-exact/`。
- 修复后重新运行 36 套 programs 独立宿主配置：362 passed、0 failed；
  image 16 项、env 38 项、Python 18 项、等待合同 2 项、导出合同 1 项、
  trace 普通/semihosting 各 2 项均通过。
  workspace debug/release、kernel tests 含 semihosting 的 debug/release 检查通过；
  Nushell 解析与 git diff --check 通过，没有全树格式重排。
- 本轮完成差异自查，未冒称独立审查批准；未提交、未推送、未合入共享分支，
  HEAD 仍为 `3c5e5bb6`。尚余强制全局退出的在途 DMA、原生确认失败注入、
  多代真实 α 长跑、既有八 Hart 执行证据缺口及最终独立审查。
  β 自身死亡仍不在本轮合同内。

#### 强制全局退场、真实多代 α 与确认不可达

本节接续已推送的 `0117e513` 检查点，仍在 `feature/driver-recovery` 上实施。
α/β 的生产分域、生命周期和停链顺序不变；没有新增设备总管、系统调用或能力种类。

**强制全局退场**

- 新增 `iommu-forced` 部署，入口仍是生产 `system` 镜像。
  Harness 控制器提交真实读请求后持续存活，不能靠业务退出走正常退场。
  使用实际的 10 秒空闲期限，未修改生产超时。
- 复用独立 NBD 后端挂起真实请求。宿主等待空闲期限、α 整个 Team 回收、
  翻译失效和 PCI disable 意图；挂起窗口内 PCI/IOMMU/recovery/控制器均不得回收，
  不得完成端点停止、引用释放、绑定结清或全局退场。
  本场景宿主夹具的保护期限为 30 秒，不是生产期限的延长。
- 后端恢复后必须有实际扇区数据和成功 status、完整停链、所有静态服务回收、
  `system: frame Step(Idle)` 以及生产根任务的 self-doom 事件。
  生产强制路径没有改成正常成功返回；内核允许 self-doom 干净结束，
  因而本场景还必须通过独立的原生证据检查，不能只看 QEMU 退出码。
- 首轮宿主驱动误匹配 `system: frame Idle`，导致交互驱动未完成；
  真实文本是 `Step(Idle)`。失败留在 `target/driver-recovery/forced/first/`，
  未当作通过。修正后先连续通过五次，最终代码又连续通过五次。

**动态实例活动与调度容器竞争**

- 新增 `iommu-soak`：同一个常驻控制器创建 128 代真实 α，
  每代包含主任务和辅助任务，完成一次真实 DMA 读，
  交替正常关闭与主人死亡，共 64 + 64 次。
  下一代只在前一绑定结清后交窗，重置请求页，使用新构造号和新绑定号。
  128 代跨过 Hub 的 32 条关闭历史窗口，不是纯账本循环或单域翻译探针。
- 首轮只完成七代便被强制退场：System 的活动计数只看静态服务，
  动态 α 的回收没有更新空闲基线。该次原生退出 0，但没有完成场景；
  补上的严格 checker 对原现场退出 1，不能把它算成 soak 成功。
- 活动采样现在同时数静态服务和仍持有 Team 的动态实例。
  只在实际数量下降或首次采样时更新时间，持续存活本身不算进展；
  没有增加心跳状态，也没有延长 10 秒期限。
  实现落在 `system/app/activity.rs`，直接复用 Control 的现存实例记录。
  三项宿主测试编译真实实现；临时恢复旧静态计数后三项均失败、退出 101，
  恢复修复后三项通过。
- 辅助任务周期性执行自身的 park，提供真实 syscall 所在 Hart 的观测。
  它此前在独占空闲 Hart 上可能一直运行而没有 `switch` 事件。
  checker 仍要求主、辅助任务的实际执行 Hart 至少有一对不同，
  并要求两者完整 reap；辅助任务的自身 park 可作执行证据，
  远端 exit/reap 不可冒充执行。新增五项正反对照。
  这仍不是持续同时运行区间的证明。
- 周期性 park 的多代试跑暴露 `remove_from_starved` 断言：
  空闲核已经从本地或远端队列取出 Ready 任务，但尚未把它放入运行槽，
  另一 Hart 的 Slay 在所有就绪容器中都找不到该任务。
  `soak/activity-fixed/` 曾被并行试跑复用，日志不作为独立重复样本计数；
  后续验收全部使用唯一目录并串行运行 QEMU。
- 新的 `conductor/dispatch.rs` 用既有提交锁覆盖“本地取出或偷取 → 入运行槽”，
  `fetch` 及空闲等待中的所有取任务入口共用这一条规则。
  不新增中间任务状态，不取消 Slay 断言；提交锁不会跨越 WFI。
  三项宿主合同直接编译实际 dispatch，仅替代队列、入槽和提交边界。
  去掉外层提交锁的负对照退出 101，本地和偷取两项均报
  `Slay observed Ready outside its container`；恢复后全部通过。

**原生确认不可达与恢复重试**

- 三个 Harness 部署 `iommu-fail-invalidate`、`iommu-fail-reset`、
  `iommu-fail-release` 共用 `iommu-confirmation` 入口和同一控制器。
  生产 IOMMU、PCI、recovery 镜像不增加场景分支或替身管理者。
- `dma-confirmation-scene` 是默认关闭的内核测试 feature；
  hook 还受 `debug_assertions` 和 `testing()` 限制。
  只有无父 Team 的引导任务对自身孔发送的完整测试控制帧可配置它。
  被拒的请求必须匹配内核提供的发送者、实际孔 owner、操作码和绑定号；
  不是信任生产报文自报的身份。
- 注入点在已核授权、复制请求之后、交付之前，返回真实传输错误。
  对应 Hub 记录原始 `party/op/code=0` 并留在同一步。
  控制器先确认 `Closing` 与准确失败，再要求移除注入，
  随后同一绑定必须继续停链并变为 `Closed`，原始失败仍须可查询。
- 首次拒绝及解除注入之前，夹具从真实 IOMMU 任务的引用表读取并断言两份引用：
  表页引用和 DMA 引用。两份引用不是两页，也不是物理分配对象数。
  配合本绑定的六页取得记录、完整停链及最终分配基线检查，
  不用 Hub 的状态位代替实际引用持有。
  kernel trace 还须证明 α 两个任务早于拒绝时刻回收，
  五个静态服务晚于解除时刻回收。
- 最初夹具把帧缓冲上界 24 误当实际报文长度，并假定根任务号为 1；
  debug 场景的真实根号为 10，IOMMU/恢复步骤实际过线长度分别为 18/17。
  同时修正控制器请求读端的显式交付，并复用唯一 ACK 孔，避免重复发现歧义。
  初版失败现场保留在 `confirmation/first/`，不作为验收。
  四项宿主测试编译实际注入函数和两份真实协议，
  核调用者、owner、操作、绑定、短帧、引导配置权限与引用缺失负对照。
- 三个边界在最终代码上各连续通过三次四 Hart 原生运行。
  不开 feature 的负对照退出 1：即使原生整机能收尾，
  也因缺少注入证明而被 wrapper 拒绝，不能静默测试到正常路径。
- 这里证明的是**操作确认不可达时保留责任与资源，以及恢复后的重试**。
  没有模拟硬件成功确认，也没有在本轮原生测试中制造
  `Fence::Expired`、设备 reset 回读拒绝或内核 Reference Release 自身失败。
  后三者不能用这组三个传输拒绝场景冒充已覆盖。

**验收与交付边界**

- 最终批次位于 `target/driver-recovery/final/`，每次运行有独立目录、退出码、
  控制台和结构化现场。新的 Ruin/forced/soak/confirmation 证据场景共用
  qtest 的归档与 checker 调用规则；任一 checker 失败均使 wrapper 失败。
- 九个既有四 Hart 场景全部退出 0：accept、product、system-fault、
  iommu-probe、iommu-dma、iommu-product、iommu-product-fault、
  iommu-product-start-before、iommu-product-start-after。
  最终代码的 Ruin 五次、forced 五次、三个确认边界各三次均通过。
- 128 代四 Hart 场景通过：164910 ms，物理分配对象 85 → 85。
  最终八 Hart 场景通过：164741 ms，153 → 153。
  每代均核真实 DMA 数据、完整停链、不同 Hart 的主/辅助任务执行及完整回收；
  PCI/IOMMU/recovery/Hub 必须跨全部代存活。
  另有三次八 Hart 既有 fault 场景通过两代执行 checker。
  这些是有界的分钟级 128 代测试，不称作小时级或无限期稳定性证明。
- 宿主检查：programs 37 套独立 manifest、365 项通过；
  kernel 9 套独立 manifest、75 项通过；image 16 项、env 38 项、
  Python 39 项通过。所有宿主 Cargo 命令显式使用
  `--target x86_64-unknown-linux-gnu`。
  本轮曾漏传 image 的 host target，错误日志保留；
  改用实际 host 命令后通过，未修改源码来适配错误的 RISC-V std 构建。
- workspace debug/release、kernel tests 含 semihosting 的 debug/release 检查通过。
  新的关键生产函数及注入夹具做定点 rustfmt 检查，不宣称全树格式检查。
  `git diff --check` 和 Nushell 解析检查通过。
- 默认健康命令仍退出 101：118 passed、1 个缺 initrd 哨兵失败，不报告全绿。
  精确的 race 命令退出 0，证据另存 `final/race-exact/`：

  ```sh
  CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true \
  QEMU_SMP=4 TRACE_OUT="$PWD/trace" \
  nu scripts/qtest.nu --package kernel --scene memory-transfer-race \
    --feed-script programs/tests/memory-transfer/race.py \
    -- --features semihosting,race-scene -- scene
  ```

- 场景调用示例：

  ```sh
  CARGO_INCREMENTAL=0 QEMU_SMP=4 TRACE_OUT="$PWD/target/forced-check" \
  nu scripts/qtest.nu --package kernel --scene iommu-forced -- --features semihosting

  CARGO_INCREMENTAL=0 QEMU_SMP=8 TRACE_OUT="$PWD/target/soak-check" \
  nu scripts/qtest.nu --package kernel --scene iommu-soak \
    -- --features semihosting --timeout 300

  CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true \
  QEMU_SMP=4 TRACE_OUT="$PWD/target/confirmation-check" \
  nu scripts/qtest.nu --package kernel --scene iommu-fail-release \
    -- --features semihosting,dma-confirmation-scene -- scene
  ```

- 本轮进行差异自审，未冒称独立 reviewer 批准，未合入 `master`。
  尚未覆盖上列硬件内部/内存释放原语自身的原生故障注入及更长时间 soak；
  β 自身死亡恢复仍明确不在合同内。
- qtest 的统一归档入口落定后再次按真实命令复测 Ruin、forced、
  四 Hart 128 代 soak 和 release 确认失败场景，四项均退出 0；
  该次 soak 为 163636 ms、85 → 85。
  恢复动态计数修复后的三个宿主合同、两份 Nushell 脚本解析和差异空白检查也通过。
  现场位于 `final/runner-unified/`。

## Operator 通用条目与授权机制

- 提取通用的目录取得、资源绑定、授权、权限收窄、撤销和清理机制，不限定于 /unit/{team}/{task}。
- /unit/{team}/{task} 是当前产品可选择的实例布局；其他逻辑位置同样可以选择实例空间，不预设 public / bound / hole 子目录。
- Operator 是条目、绑定和授权事实的唯一拥有者。关联对象的模块保存并直接返回真实条目引用，不另算路径或维护另一份绑定事实。
- 条目引用明确所属 Operator；引用标识对象，授权能力限制操作，裸编号和路径均不代表权限。
- 同一父目录下的 part：缺失则创建；已有目录则返回同一条目且不重复产生创建事件；已有资源绑定则报类型冲突。
- 根目录 / 的修改权限只属于可信初始化指定的 System 任务，不向其他任务授予根写权限。System 可以创建子空间并向其他任务授予局部权限；子授权不能扩大范围、修改父目录或替换授权锚点。
- System 通过通用构造操作建立产品选择的 /service、/device、/identity、/unit，再关联逻辑键。程序从逻辑键解析实际条目，只持有局部权限。
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

## 此前实施与验收（提交 50ad14b1，2026-10-12）

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

## 本次 Namespace 调整（已实施并验收，2026-10-12）

此前实施与验收记录对应提交 50ad14b1。用户已明确要求派 Luna 实现以下调整；源码已采用新骨架，本节记录本次实际结果。

用户确认当前产品的顶层骨架为：

```text
/
├── service/
├── device/
├── identity/
└── unit/
```

提供构造布局的机制，由产品组合代码使用实际父 Entry 和构造权限创建目录，或接入已有目录，再按需关联逻辑键。上述名称不成为 Operator 内置分类；不在机制中写死骨架，不保留逐程序枚举的 NAMESPACE_ROUTES。内部层级由模块组合决定，本次顶层选择没有指定 driver、sys 或 team/task 等子目录。

associate(key, entry) 将调用方约定的逻辑键关联到实际 Entry；resolve(key) 返回该 Entry。逻辑键由共享契约定义，不由 Operator 自动产生，也不代表操作权限。已有 Entry 的构造调用可以直接组合，无须先定义逻辑键。关联写入仅允许 System；同键同条目可重放，不同条目冲突。普通任务不能修改根或全局关联，只能使用已授予的范围与操作权限。

Task 的声明指定逻辑位置、相对位置以及 scope/ops/permit 等限制；Task 在范围内自行创建和绑定接口，无须在 UnitFile 预列内部名字。接口消费者从解析出的实际 Entry 查找相对条目；Identity 导出继续验证实际条目、版本、主体和调用来源，不能由逻辑键替代这些检查。

布局构造复用现有 Operator 操作和 schedule 推进机制。创建回执与操作编号支持重试和清理，引用已有共享条目不记作本次创建。修改产品骨架集中在组合处，程序所依赖的逻辑契约保持稳定。本次按启动时构造布局设计，不增加运行中搬迁。

实际实现：

- 产品组合在 app/operator 中使用真实根权限逐步 part/associate，保留操作编号、待关联结果与游标。System 的通用构造机制不接收强制骨架表；删除 Shared、Base、Route、NAMESPACE_ROUTES 及本地关联镜像，解析只从 Operator 的唯一关联表读取。
- NamespaceKey 定义纯逻辑标识；四个产品逻辑键定义在共享纯契约中。Operator 注册同时核对原生 System 调用者和真实根授权入口，拒绝关联资源叶子、跨 Operator 条目和已删除目录；同键同存活条目幂等，不同条目冲突。未知键不回落到根。
- Unit 声明显式选择 key、Shared/Instance placement 和相对位置。生产服务管理自身 Subtree，特殊测试保留 Names 范围。Instance 使用 Control 的真实 team/task；无 team 在副作用前拒绝，同任务同逻辑位置复用实例根，共享 team 目录保留到 System 生命周期结束。
- 查询和订阅支持实际 Entry 下的相对路径；订阅允许未来发布的条目，仍验证存活目录、真实事件来源和精确前缀，再投影相对路径。所有业务客户端和测试夹具迁移到逻辑解析与相对条目。
- 驱动接口分别发布为 SERVICE 下 router/entry、rtc/entry、uart/rx 和 uart/tx；设备使用 DEVICE；Identity 接口使用 IDENTITY/api。RTC 入口契约保持纯模块，镜像打包不依赖运行时实现。System 发布者复用各自 API 的相对位置定义。
- 禁止任务删除授权边界本身；普通任务不能写根或注册关联。删除回复按实际状态解码，传输不确定返回 Unjudged 并保留回执，重放已删除条目返回 Unknown。身份导出保留真实绑定、版本、视图、Permit、身份子树和来源核验。

本次有效验证：

| 检查 | 本次结果 |
| --- | --- |
| programs 宿主回归 | 30 套，259 项通过 |
| namespace-layout / namespace-installation | 18/18、4/4；前者使用生产 Tree，后者验证生产 Assembly 的计划和回执，外围 System 是测试适配 |
| Image / Identity book | 10/10、16/16 通过 |
| accept QEMU | 所有必需探针及入口明确成功退出，10.44s；覆盖逻辑解析、实例声明、普通注册与根修改拒绝、边界删除拒绝、删除重放和相对订阅 |
| product 登录 QEMU | 交互脚本与入口成功，2.68s |
| product Shell QEMU | 镜像命令、管道、1 MiB、暂停/恢复/取消与前后台脚本成功，11.84s |
| system-fault QEMU | 入口明确成功，13.69s；包含真实共享父目录/兄弟项清理、双实例、身份来源及故障回收 |

首轮整机暴露的入口命名分叉、未来条目订阅、删除回复忽略及旧 Terminal 夹具路径均已修正。以上整机结果仅计修复后成功运行；未放宽原验收期限。共享父目录清理由生产 System 回执和真实 Operator RPC 验证，不以宿主 mock 的 retire 成功代替。

## Image release 警告清理（2026-10-12）

完整 `cargo image product release` 构建警告由47条降为0，image宿主工具自身release构建也无警告。删除未使用依赖、旧封装及冗余字段，测试辅助入口仅在测试配置中编译；共享声明投影和当前组合未使用的通用供给契约使用具体、局部的lint标注，没有关闭全局warnings。Image 10项及受影响的7套宿主回归共98项通过，差异检查通过。
