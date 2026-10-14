# Frame、Tree、Memory、Space 与 Pole

本记录描述 CPU 页表与内存资源的所有权和清理合同。IOMMU/设备页表没有实现。

## 资源关系

```text
Space ──值── Memory ──Arc── Tree ──私有区间索引──Arc── Map ──Arc── Frame
                                                     │
                                                     └──Weak── 所属 Tree

Pole ──单一 HeldMap 集合──Arc── 实际 Map（及授权、打开位置与关系）
Mapping ──Map Arc + VA Reservation
Memory ──Tree + VA allocator + 失败清理队列 + kind
Space ──Memory + 等待生命期
Task ──Space 中的栈映射、陷阱帧映射和 guard 预留
Team ──共享 Space 与构造/成员状态
```

`Frame` 用分配器提供的 `Box<UnsafeCell<[u8; PAGE_SIZE]>>` 持有一页物理内存。它的 `Arc` 是帧共享和释放的所有权依据。`BorrowedRegion` 表示调用者保证仍被保留的物理区间，创建它是 `unsafe` 操作；其元数据 `Arc` 不接管底层物理存储的所有权。

`Tree` 持有 ASID、页表几何规则和根页表状态。它创建、查询、拆分、保护和移除 `Map`。`kernel/src/core/memory/manager` 不引用 `work`；任务、团队与 Pole 的组合留在工作层。`Map` 仅保存所属 `Tree` 的 `Weak`、初始 VA 范围和映射状态；状态保存仍存活的子范围、所持 Frame、借用区间与 lazy 标记。Map 自身没有 AVL 左右链、高度、最大区间或回收链字段。区间查找用的 `IndexNode` 和 shootdown 期间保留的 Map/Frame/页表页都属于 Tree 内部状态，没有 `work::unit::space::Index` 外观层。

`Memory` 组合 `Arc<Tree>`、VA 分配器、延迟清理链和地址空间 kind；`Space` 以值持有 Memory 和等待生命期，不增加 `Arc<Memory>` 分配点，也不转发整套内存操作。清理链头是 `SpinLock<Option<Box<CleanupNode>>>`，没有单字段队列包装。普通区间在构造时接入；`attach_region` 配置已有边界，镜像装载后的 `advance_region` 单调抬高分配下界。内核 Memory 的 `RegionId::Resource` 范围为 `[mode.lower() + DRAM.base + DRAM.size, TRAP_STACK_BASE)`，内核 keeper 和 device 大区映射从此区域选址；`RegionId::Kernel` 限于 64 MiB 的 frame-context 窗口。Memory 执行地址范围、U 位和 supervisor 策略检查；目标团队选择、授权、构造租约及通知仍在工作层。`backend()` 仅在 debug 构建中供白盒核验使用，生产调用不借出 Tree。

`Reservation` 持有 Tree 弱身份、区域、起点、长度和 allocator 生成的 reservation id。Map 与该预留组成 `Mapping`。Memory 的选址、映射、派生与回收操作协调 VA 预留和 Map 生命周期，通过 reservation id 防止过期或错误的预留释放当前区间。凭证保留稳定的 Map 身份，不在索引移除后仅按可复用的地址重建身份。

Task 的 `TaskMemory` 持有栈和陷阱帧的 Mapping 及 guard Reservation。显式退休通过 `Memory::unmap` 完成；`TaskMemory::drop` 把尚存的 Mapping 交给 `Memory::dispose`，由 Memory 持有清理责任。借用回收失败后调用方仍持有凭证；接管回收失败后 Memory 保留清理记录，两者不可混成一律消费。`TaskIdent` 缓存陷阱帧的 `PhysAddr`，调度和陷阱路径直接读它；同一帧的内核 `VirtAddr` 只给健康面核验该页不在用户可访问的范围内。Team 持有共享 Space；任务发布与 Pole 私有映射交付仍由 Team 的构造状态控制。

## 普通内存使用面

申请和视图使用标准 `Layout`、逻辑 `Access` 和可选的指定地址，返回 Mapping 凭证。凭证公开地址与长度，内部映射身份和预留 generation 不向普通调用方暴露；它不提供可在当前地址空间直接解引用的指针。页表、ASID、PTE 位与回收记录由 Memory 管理。

- `allocate` 申请已物化存储，初始权限由 Access 给出，权限上限为 R/W/X。物理帧保持既有的清零分配。
- `allocate_lazy` 保留地址与权限范围，按既有缺页或派生路径物化；它仍可能在元数据或清理节点分配时以 OOM 失败。
- `view` 从已有凭证建立共享存储的另一段地址映射，是可失败操作，不由 Clone 隐式分配。
- 三条 Layout 入口都在内部取整大小并满足首地址对齐；请求大小不必由使用者按页取整，超对齐不会新增大页机制。
- `protect_range` 先检查完整覆盖与权限上限，再按既有顺序逐段修改。当前权限可在上限内调整，上限本身只能收窄。

旧 wire 字节入口与 Layout 入口共用实现，保留不能表示为 Layout 的合法机器字尺寸及原失败码。Allocate 仍是 eager；无来源的 Mmap 仍是 lazy。Mmap 拒绝 W+X，Mprotect 仍允许 R/W/X 同时存在，不将两者的授权规则混成一条。

缺页入口只采集地址、PC 与错误种类，调用 `Memory::resolve_fault`；重试、权限核验和单页物化在 manager 内完成。普通读取和复制不隐式物化。启动静态映射、上下文 satp 和物理地址查询通过 crate 内的具体可信操作接入，不提供通用页表后门。IOMMU、设备页表、grow/shrink 和新的返回寄存器协议均不在此接口范围内。

## Pole 与授权

Pole 的唯一映射记录集合是 `Vec<HeldMap>`。每项关联实际 `Arc<Map>`、可选 token/`Arc<Grant>`、可选的 `Weak<Space> + Reservation` 放置位置，以及 Input、Keeper、Authority、Open、View 或 Staged 关系。创建 Pole 时从完整、已物化的现有 Mapping 建立输入映射，并在内核 Tree 中派生 keeper 映射；打开时从仍存活的规范来源派生目标 Space 的 Map。Pole 不持有裸 Frame，也不维护单独的 `source_map` 字段。来源通过集合中的 live Keeper 或 live placement 选择。

`Grant` 原子保存权限位；Pie 与快照共享同一个 Grant。权限收窄用原子比较交换，失效将其清零；快照不会延长已失效授权。Pole 的操作 lease 序列化创建、打开、撤销和交付；同一时刻已有 lease 时相关操作以 Busy 失败。

ONLY 根资源的创建权通过 `CreationRegistry` 一次性交付。注册项以 `Weak<Map>` 关联候选映射，查询与 claim 使用 `Arc::ptr_eq` 的实际 Map 身份；registry 不用可复用的 VA 或 ASID 数值代替身份。准备阶段先分配节点，映射发布后再提交登记。claim lease 在构造失败时恢复可用，成功提交后消费该权利并移除登记项。

## 失效、锁序与错误

Tree 修改页表后先完成本地及远端 TLB 失效，完成后才丢弃延迟保留的 Frame、TableNode 和 Map。每个 reservation generation 在 PTE 发布前由 `VaAllocator` 预留一个 `Box<CleanupNode>`，因此 `Memory::dispose` 接管已发布映射时不需要再分配清理队列节点。Tree 在准备 unmap 时仍可能因 OOM 返回错误；Memory 会把未完成的 cleanup node 放回链头，不丢失清理责任。失效状态为 `Clean / Dirty / Fencing / Failed`：进行中让并发操作等待；失败保留资源，重试进入 Fencing，成功后才清空记录并在锁外释放。关闭状态为 `Open / Closing / Closed`，只在失效完成后落为 Closed。永久失效失败时资源继续保留，不能宣称已经回收。`Memory::unmap` 先完成已有待失效操作，再检查 Map；空 Map 可幂等成功，并只回收传入 reservation 的旧 generation，不能释放后来占用相同 VA 的新 generation。销毁普通 Tree 时先撤掉其映射，完成 shootdown 后才回收 ASID；内核 Tree 返回 `Pinned` 并保留进程期根页表。Map 的弱 Tree 关联让树关闭或销毁后的旧 Map 无法操作新 Tree。

`machine::sync::Level` 的递增顺序为 `Unit < Scheduler < Space < Tree < Map < TeamTasks < L3 < Roster < Asid < Frame < Block < Tally < Spare < UnitState`。新增锁必须按严格递增次序取得；debug 构建会检测递归获取、倒序和跟踪容量越界。锁与其守卫分别定义，守卫作为锁的子模块访问私有字段。持锁集长度由 `Vec::len()` 给出，不另存一份计数；8 个槽是初始容量，不是不可扩展的上限。

ASID shootdown 总是刷新本 hart。远端 kernel shootdown 依据 `ONLINE` 原子位图，用户 Tree shootdown 依据各 hart 当前 ASID lease。次级 hart 在启动入口发布 online 位（Release），shootdown 侧用 Acquire 读取；切换地址空间时 ASID lease 也以 Release 写、Acquire 读。这样 kernel 映射修改会覆盖已 online 的 hart，即使其当前 lease 不是该 kernel ASID。

保留的 `MapError` 覆盖物理内存不足、未对齐、已映射/未映射、无所属区域、权限上限不可加宽、DRAM 布局重叠、shootdown 失败和内核 Tree pinned。普通接口保留既有失败码映射，shootdown 未完成对应 Busy，不丢弃映射资源或重用 reservation。

## 既有 wire 接口

`UnsealArgs::Pole` 携带 `{ addr, size, shared }`，wire 参数为五个机器字，Pole variant 编码为 `[1, addr, size, shared, 0]`。Pole 创建要求指定一段已有、完整的映射。

`MemoryCall::Mmap` 使用 `pole: PieToken`，wire 槽位保持不变。Memory 接口与内部类型归位不改变这些既有编码、错误码或诊断 schema；`Frame`、`Tree`、`Map`、`Space`、`Mapping` 与 `Reservation` 的 Rust 实现接口不是用户 wire ABI。

## 先前页表迁移的字段计数

下表保留先前 CPU 页表与资源路径迁移的测量，不代表后续类型归位或 Memory 接口迁移后的工作区读数。该次测量使用同一份词法扫描器读取迁移前后 `kernel/src` 的所有 `.rs` 文件，包括当时未跟踪文件。扫描器剥除注释和字符串，跳过宏输入，识别具名 `struct Name { ... }`，按括号、方括号、大括号和泛型括号的嵌套层级统计顶层字段；enum、tuple struct 与 unit struct 不计。

| 范围 | 迁移前 | 该次迁移后 | 净变化 |
|---|---:|---:|---:|
| 全部 `kernel/src` | 125 个 / 400 字段 | 135 个 / 429 字段 | +10 个 / +29 字段 |
| `memory/manager` 与 `work/unit/space` | 20 个 / 65 字段 | 22 个 / 74 字段 | +2 个 / +9 字段 |

字段迁移不代表字段净减少。表中计数包含真实新增状态、旧职责归位后的字段和仍保留的缓存；不能把职责迁移或删掉旧封装称为字段净节省。当前真实新增状态包括 reservation generation 身份、Map 子范围/lazy 与失效保留状态、Space cleanup 记录、allocator 中按 generation 预分配的 cleanup 节点，以及 Pole 的实际映射/授权关系。旧 Map 上的 AVL 索引与回收链字段已从 Map 移走，由 Tree 内部索引和 shootdown 延迟保留结构承担；旧 `Backing`/`Permit` 混合记录和重复 VA/物理地址包装已删除或由真实 `Frame`、`Map`、`Reservation` 关系取代。Task 陷阱帧物理/虚拟地址仍是供快路径使用的真实缓存。故字段总数如实净增 +29（manager+space 净增 +9），结构职责仍已归位，不能据此声称净减字段。

## 先前 CPU 页表迁移验收

下列为该次迁移的验收记录，不代替后续接口或内部原语改动的重新验证：

- workspace debug 与 release 编译检查通过。
- QEMU 内核健康检查 41 项通过。未带 initrd 的 `scene` 哨单独失败，报错为 `test scene requires an initrd`；这不是场景验收通过记录。
- release `accept` 与 `product` 场景各 1 项通过，包含实际镜像、自举、PCIe 资源区域及调用方迁移。
- gate 宿主测试 32 项、resource 宿主测试 16 项、env ABI 宿主测试 13 项通过。
- `git diff --check` 通过；manager 不引用工作层类型，旧 Backing、SpaceInner、空间 Window、独立 Index 门面及 with_flush/with_shootdown 路径已退出。

健康检查覆盖共享 Frame 的单侧撤销与最终释放、Map 稳定身份、部分保护/撤销、大小页拆分、AVL 与硬件页表一致性、lazy/guard、失效失败保留与重试、ASID 复用身份、任务逐点失败和首次 Spawn 提交，以及 Pole 创建失败、重复 Open、关闭后重开、授权收窄/撤销和禁止重铸。相关检查比较帧、页表分配及真实授权关联，确认被测试的成功及恢复失败路径无残留。永久 shootdown 失败依约保留资源；本记录不将其列为已经释放，也不宣称设备 DMA 或 IOMMU 页表已经实现。

## 当前内核组织

生产顶层为 `core / machine / entry`，不保留旧顶层兼容模块：

- `core/memory`：分配器、页表、映射、ASID 与失效后的资源回收。
- `core/work`：运行实体、通信、授权、调度、等待和记录。
- `machine`：机器信息、每核执行信息、时钟、上下文、同步和处理器停机机制。
- `entry`：启动装配、异常与调用接入、资源交付、报告和测试接入。

本仓模块写 `crate::core::…`，Rust 核心库写 `::core::…`。

| 契约 | 核心或机器事实 | 接入与组合 |
| --- | --- | --- |
| 翻译与上下文 | `Tree::satp` 编码；`machine/switcher` 安装寄存器 | Task 选择空间与 SPP；启动、探测、恢复保留写入时机 |
| 定时 | room 保存期限队列、选择预算；machine 编程硬件定时器 | 中断入口领取到期票据并唤醒 |
| 初始资源 | machine 提供设备树视图；room 持有中断通信对象 | entry 创建、登记、冻结和交付能力 |
| 记录与报告 | `core/work/record` 保存 Trace 与退出账 | `entry/report` 格式化、导出、报告与测试判决 |

Trace 的环与可选 sink 使用同一次发布。OOM 不发布环，也不留下阻止重试的 sink。
没有第二份事件队列。导出外层 `{h, when, kind}` 的 JSONL/fs 运行路径尚未独立做整机导出验收。

停机顺序保持为：各核发布到达 → 屏障完成 → 报告与 Halt 记录 → 调度资源清理 →
分配器 flush → 测试判决或系统复位。已有 hook 集合保存清理顺序，等待报告使用独立直接
函数指针，不通过数组位置派发事件。

`machine/image` 仅定义链接器边界及启动栈；共享虚拟地址窗口仍在 `machine/layout`。
两者不全量再导出同名事实。地址类型保留在 memory。

内核内用例位于 `kernel/tests/health`。白盒用例通过受控路径保持原逻辑子模块身份，
没有为迁移测试公开生产字段；登记仍由 `kernel/tests/embedded.rs` 完成。
Pend 与 Trace 的宿主套件通过 `#[path]` 编译生产实现，运行时必须显式指定宿主 target。
`crates/alloc-probe` 被 workspace 排除，但直接 include 生产分配器，须从该 crate 目录
单独验证，让其 CFLAGS 与串行测试配置生效。

### 顶层迁移验收记录

- workspace 生产目标 debug/release、kernel all-targets debug/release 通过。
- kernel 的 semihosting feature debug/release 编译通过；未将此记作整机导出验证。
- QEMU 健康检查 41 项通过；无 initrd 的 scene 哨兵失败单独记录。
- release accept/product 场景各 1 项通过。
- Pend 宿主 34 项通过，包含新增的只读交付与 staging 观察回归。
- Trace 宿主默认/semihosting 各 1 项通过：首次分配失败不发布，重试后环与 sink 一致，
  再次初始化返回 AlreadyInit。
- resource 宿主 16 项、env ABI 宿主 13 项通过。
- alloc-probe 的 all-targets 宿主编译、默认 6 项、plain 10 项与正式多核入口 4 项通过。
- `core/machine` 对 entry 的直接路径引用均为 0；顶层声明由 11 个降为 3 个。

workspace `--all-targets` 会把 stream 的宿主 std 测试编译到裸机目标，在迁移前即失败；
生产 workspace 与 kernel 测试目标分别检查，不修改源码掩盖这个目标口径差异。

### Memory 接口与内部原语验收（接口批次）

- workspace debug/release、kernel all-targets debug/release，以及 semihosting 两档编译检查通过。
- `nu scripts/qtest.nu --package kernel`：45 项健康用例通过；未带 initrd 的 scene 单独报预期失败，整次命令退出码为 101。
- `--scene accept`、`--scene product` 各 1 项通过；没有放宽镜像、成功退出或探针判据。
- Pend 宿主 34 项、Trace 默认/semihosting 各 1 项、env 13 项、capability 16 项通过。
- frame-runs 宿主夹具直接编译生产叶文件，9 项通过，包含分段的权限边界、页号缺口、物理缺口与零分配重播。
- 真实树用例补充跨空洞派生、相邻页不同权限、确定物理不连续帧及释放后的计数核对。
- alloc-probe 从其自身目录验证：all-targets 宿主检查、默认 6 项、plain 10 项、多线程入口 4 项通过。
- Layout 用例覆盖字节取整、零大小、指定地址、超对齐、最低空档复用、错误 owner、失败回收与共享视图；eager、lazy、view 三条路径都实际核对超对齐。
- 普通内存调用入口不包含 PTE、物理帧、页大小或地址桶的组装；生产 Memory backend 借用为 0。

manager 的 37 个具名类型和 sync 的 18 个具名类型已各自独立定义，关联或对偶不作为同文件豁免。`Memory` 的子模块保存凭证、回收记录及 `VaAllocator`；后者的子模块保存地址区域与预约槽。`TableNode` 定义在 `table/mod.rs`，其物理 `PageTable` 保存在 `table/page.rs`。`MapError` 属于 manager 共同词汇，不从 table 借出；本地 TLB 刷新与跨核指令同步由 ASID 机制实现，module 入口只声明与导出。Tree 与 Map 仅向 crate 内提供机制接口，普通使用面保持 Memory、Layout、Access 与 Mapping。类型数量和字段归组不冒充持久状态净减。其他内核对象仍按下述边界单独验收。

顶层迁移本身没有完成全树一型一文件。后续归位保持类型与自身实现同处，子目录表达
实际组成或算法；相互关联不作为同文件豁免。manager 的普通使用接口采用 Layout、
Access 和 Mapping 凭证，生产调用不再使用 `backend()`。
普通数据类型的双向引用不自动构成协议倒置，不能为了画 DAG 复制事实或拆断清理责任。
字段搬迁不计作字段净减少。

### 深层组成与规则归位

以下按 `kernel/src/**/*.rs` 中手写的具名 struct/enum/union 声明计数，包括宏输入中的声明，
不包括宏展开生成项。迁移前取 `193af277`，迁移后取本轮源码；同一类型的 impl 不另计。

| 范围 | 具名声明：前 → 后 | 多型文件：前 → 后 |
| --- | ---: | ---: |
| core | 155 → 151 | 25 → 0 |
| entry | 29 → 21 | 6 → 0 |
| machine | 32 → 31 | 5 → 0 |
| 合计 | 216 → 203 | 36 → 0 |

文件数为 170 → 277。声明减少包括测试 Buffer 迁出生产目录，不能换算成运行期状态或内存
节省；结构验收同时要求规则归实际拥有者，不能只凭多型文件清零宣称设计完成。

`kernel/src` 的物理布局统一为：有子模块的模块把本体放在 `foo/mod.rs`，没有子模块的叶子
保留 `foo.rs`。44 处同层 `foo.rs + foo/` 组合已全部退出；模块路径、声明数和实现不变，
不新增同名类型文件或转发层。此规则的本轮范围是 kernel，其他 crate 与 programs 未统一。

- `room::conductor` 持有每 hart 的运行槽、饥饿队列和身份位，执行本地调度；
  `room::scheduler` 持有跨 hart 的放置、名册与唤醒协调。没有 `scheduler/core` 兼容层。
- 工作存活计数属于 `unit::tally`；各核停机到达、屏障与注入的清理钩子属于 `room::halt`。
  钩子次序仍是报告、调度资源清理、分配器 flush、判决。
- 提交锁属于 `work` 共同父，`Level::Unit` 的既有值与锁序不变。Task 的身份、内存、
  状态轴和构造过程、Team 的成员、收据、暂存与构造租约分别归其真实组成。
  状态转换、链载荷是否合法、团队终止判定由状态拥有者实现。
- 调用返回控制只有 `Resume / Switch / Exit` 一套词汇，失败码只有一个 A0 写入口；
  一值、两值与三值的成功回复保留各自 wire 形状，不引入 Pair/Triple 回包框架。
- `Backtrace` 自己持有帧集合，walk 直接产生该结果；scan 与链遍历不另存一份末帧缓存。
  回溯展示只读取 Backtrace，不反向依赖 Scene。零填充十六进制与裸地址标签保持各自格式。
- 分配器保留真实空闲链、诊断表和锁豁免。Frame 的空闲幂用 `Option<u8>` 表示，
  不另存 `free` 标志或写入无人读取的已分配幂；其每项仍为两字节，不宣称节省台面内存。
  Pump 无读的长度计数退出；block/hybrid 的大小上界取同一处定义。
  alloc-probe 直接编译共享 Link，不保留镜像实现。
- 创建权的唯一状态是 `AVAILABLE / CLAIMED / CONSUMED`。CreationLease 不再另存
  committed 标志，取消只能把 CLAIMED 改回 AVAILABLE；消费后的 Drop 不会恢复创建权。
  PreparedAllocation 直接拥有预分配节点，不表示永远不可达的空节点。
  Pend 宿主夹具使用生产能力账本，而不是字段同形的另一个 Pend。
- Site 建立规则只有一个实现；Alarm 等待共用一个具体动作。不同的 signal、wake、
  wipe 与空间清理协议仍分别保留，取消票据和放锁后唤醒的顺序不改变。
- Mail 的写入预约与读取守卫各自持有提交、取消和返还规则；PoleOperation 持有操作位的
  CAS 与 Drop，HeldMap 持有关系与放置判据。可重试关闭与接管清理仍是不同的所有权操作。
  Tole 的 Cell 判定相同 key 的第一次 forward 与最后一次 unforward，保留双侧账本及回滚。

真实删减与纯迁移分别记录：无读取的启动计数、Pump 长度、重复末帧缓存与创建租约标志
属于删减；字段换文件、身份词汇内联、状态归组不计作持久数据净减。暂存、收据、
Map 身份、Task 的上下文缓存与失败清理责任仍然存在。

`system-fault` 在深层批次验证中出现 Loader 超时。用 `git archive 193af277` 的独立源码
运行同一命令也失败，签名相同：`loader.rs:361` 的 `Err(6)`、`:306` 的等待超时和
`:190` 的 IPC 超时。此项未验收通过；它与不带 initrd 时故意失败的 scene 哨兵不同。
没有修改程序、超时门槛或验收断言来使其变绿。

本轮最终验证：

- workspace debug/release、kernel all-targets debug/release、semihosting 两档编译通过。
- `nu scripts/qtest.nu --package kernel`：46 项健康用例通过；无 initrd 的 scene 哨兵失败，
  命令退出 101。`--scene accept`、`--scene product` 各 1 项通过。
- Pend 宿主 35 项，Trace 默认/semihosting 各 2 项，frame-runs 9 项，console-line 5 项通过。
  Trace 的新增断言核对 19 种事件 payload 的序列化形状。
- env ABI 13 项、capability 16 项通过；env 的既有 unused Wire import 警告未处理。
- alloc-probe 从其自身目录运行 all-targets 宿主检查、默认 6 项、plain 10 项和多核 4 项通过。
- `system-fault` 未通过，基线对照如上；未做 semihosting 外层 JSONL/fs 的整机导出验收。
