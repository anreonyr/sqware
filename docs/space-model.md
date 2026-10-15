# Frame、Nest、View、Memory 与 Pole

本记录描述 CPU 页表与内存资源的所有权和清理合同。IOMMU/设备页表没有实现。

## 资源关系

```text
Space ──值── Memory ──Arc── Nest ──全局占用──Arc── View ──Arc── Frame
                              │                   │
                              ├──私有关系索引       ├──Weak── 所属 Nest
                              │                   │
                              ├──身份与同步退休     ├──强持── FlatRange ──Arc── Flat ──Arc── Frame
                              │                   │
                              └──Installed表页     └──活动区间、已发布记录与 U/G/A/D 属性

Pole ──单一 HeldMap 集合──Arc── 实际 View（及授权、打开位置与关系）
Mapping ──Arc<View>（兼容名）
Memory ──Arc<Nest>（外层薄组合）
Space ──Memory + 等待生命期
Task ──Space 中的栈映射、陷阱帧映射和 guard 预留
Team ──共享 Space 与构造/成员状态
```

`Frame` 与 `BorrowedRegion` 属于 `core/memory/flat`。前者用分配器提供的
`Box<UnsafeCell<[u8; PAGE_SIZE]>>` 持有一页物理内存，其 `Arc` 是帧共享和释放的
所有权依据；后者表示调用者保证仍被保留的物理区间，创建它是 `unsafe` 操作，其元数据
`Arc` 不接管底层物理存储的所有权。`Flat` 是线性逻辑页来源与唯一物化权威：`Managed`
的页槽按需物化、只增不减，同一逻辑页只有一个结果；`Borrowed` 的存储不分配、不清零、
不归还 RAM 分配器。`FlatRange` 是强持同一 `Flat` 的校验范围，子范围按相对偏移合成，
不复制数据；`Pages` 是某个范围的完整物化结果，PA 只从它的可信内核边界枚举。

`Nest` 持有一个地址空间的全部 CPU 状态：ASID 身份、页表几何规则与根页表、一份全局
VA 占用、关系索引，以及待失效与退休资源。`core/memory/nest` 不引用 `work`；任务、
团队与 Pole 的组合留在工作层。`View` 仅保存所属 `Nest` 的 `Weak`、真实 `Reservation`、
初始 VA 范围、整段共有的架构属性（U/G/A/D）与关系状态；状态保存仍存活的活动区间、
强持的 `FlatRange` 来源与已发布页的 `Frame`。来源是 `View` 稳定状态的一部分，不是可
重建的收据：同一 `View` 重复查询取得同一 `Flat`，派生关系继承来源并合成偏移。物化归
`Flat`，PTE 是发布状态的真值，发布记录承担强持与双向核验；两者不是两套物化权威。
`View` 自身没有 AVL 左右链、高度、最大区间或回收链字段。占用表的窗口只决定
`Lowest` / `Highest` 的动态选址范围，`Exact` 与它们共用同一份全局重叠检查，跨窗口的
固定跨度（高镜像、跳板、hart frame、trap stack）按同一份起点/终点算术登记。区间查找用的
`IndexNode` 和 shootdown 期间保留的 View/Frame/页表页都属于 Nest 内部状态，没有
`work::unit::space::Index` 外观层。

`Memory` 是外层薄组合：只持 `Arc<Nest>`，不另存 kind、VA 存储或清理队列，也不转发
整套内存操作。它不引入 `Arc<Memory>` 分配点。清理链头在 `NestState` 里，形状是
`SpinLock<Option<Box<CleanupNode>>>`，没有单字段队列包装；节点在发布前按 generation
预留下来，只用于“已经消费但还没收尾”的关系。普通区间在构造时接入；
`attach_region` 配置已有边界，镜像装载后的 `advance_region` 单调抬高分配下界。
内核 Nest 的 `RegionId::Resource` 范围为
`[mode.lower() + DRAM.base + DRAM.size, TRAP_STACK_BASE)`，供显式内核映射选址；
Pole 保留 RAM 或借用设备区间不占此窗口。
`RegionId::Kernel` 限于 64 MiB 的 frame-context 窗口；没有动态窗口的
启动固定跨度记在 `RegionId::Static` 下。Nest 执行地址范围、U 位和 supervisor 策略检查；
目标团队选择、授权、构造租约及通知仍在工作层。`backend()` 仅在 debug 构建中供白盒核验
使用，生产调用不借出 Nest。

`Reservation` 持有 Nest 弱身份、区域、起点、长度和占用表生成的 generation。`View` 与
该预留组成一段已落地的关系。选址、挂载、改权、撤销与回收操作协调 VA 占用和 `View`
生命周期，通过 generation 防止过期或错误的预留释放当前区间。凭证保留稳定的 `View`
身份，不在索引移除后仅按可复用的地址重建身份。

Task 的 `TaskMemory` 持有栈和陷阱帧的 Mapping（即 `Arc<View>`）及 guard Reservation。
显式退休通过 `Memory::unmap` 完成；`TaskMemory::drop` 把尚存的 Mapping 交给
`Memory::dispose`，由 Nest 持有清理责任。借用回收失败后调用方仍持有凭证；接管回收失败后
Nest 保留清理记录，两者不可混成一律消费。`TaskIdent` 缓存陷阱帧的 `PhysAddr`，调度和
陷阱路径直接读它；同一帧的内核 `VirtAddr` 只给健康面核验该页不在用户可访问的范围内。
Team 持有共享 Space；任务发布与 Pole 私有映射交付仍由 Team 的构造状态控制。


## 普通内存使用面

申请和视图使用标准 `Layout`、逻辑 `Access` 和可选的指定地址，返回 Mapping 凭证。凭证公开地址与长度，内部关系身份和预留 generation 不向普通调用方暴露；它不提供可在当前地址空间直接解引用的指针。页表、ASID、PTE 位与回收记录由 Nest 管理。

- `allocate` 申请已物化存储，初始权限由 Access 给出，权限上限为 R/W/X。物理帧保持既有的清零分配。
- `allocate_lazy` 保留地址与权限范围，按既有缺页或派生路径物化；它仍可能在元数据或清理节点分配时以 OOM 失败。
- `map_range` 从已持有的 FlatRange 选址并挂载，长度由来源决定；普通生产消费者不再从
  活跃 View 推导来源。`view` 只保留为 PR 4 待迁移的形状测试桥。
- Layout 入口在内部取整大小并满足首地址对齐；请求大小不必由使用者按页取整，超对齐不会新增大页机制。
- `protect_range` 先检查完整覆盖与权限上限，再按既有顺序逐段 `set_access`。当前权限可在上限内调整，上限本身只能收窄。

旧 wire 字节入口与 Layout 入口共用实现，保留不能表示为 Layout 的合法机器字尺寸及原失败码。Allocate 仍是 eager；无来源的 Mmap 仍是 lazy。Mmap 拒绝 W+X，Mprotect 仍允许 R/W/X 同时存在，不将两者的授权规则混成一条。

缺页入口只采集地址、PC 与错误种类，调用 `Nest::resolve_fault`；重试、权限核验和单页物化在
Nest 内完成：一页先进 `Flat` 物化，再在本次关系上发布。普通读取和复制不隐式物化。
启动静态映射、上下文 satp 和物理地址查询通过 crate 内的具体可信操作接入，不提供通用页表
后门。IOMMU、设备页表、grow/shrink 和新的返回寄存器协议均不在此接口范围内。

关系操作只有 `reserve` / `attach` / `set_access` / `detach`，以及 Nest 自身的 `close` /
`retry_invalidation`。`attach` 在给定活动形状内只发布当前可用的页：`Managed` 已物化的页
发布、未物化的留给缺页，`Borrowed` 的活动页全部发布；旧 `Allocate` 先物化再登记，返回前
PTE 齐全，旧匿名 `Mmap` 不预物化。`set_access` 只在上限内改当前逻辑权限，`narrow` 另外
收窄上限，两者都不动架构属性。普通挂载与 crate 内的活动形状挂载共用一个私有安装/提交：
可失败的元数据、几何与清理载荷都先备齐，提交阶段不再失败。

## Pole 与授权

Pole 直接持有唯一 `FlatRange` 来源，长度从来源派生。`Vec<Authority>` 关联 token 与
`Arc<Grant>`；`Vec<HeldMap>` 只记录实际 `Arc<View>`、token、`Weak<Space>` 放置以及
Input、Open、View 或 Staged 关系。创建时保留完整已物化 Mapping 的来源，Input
仍承担原有交接与关闭责任，但不再派生 kernel keeper。全部 View 关闭不解除合法来源；
Open 与 Mmap 从授权来源窗口显式建立新关系，不用 canonical View 寻找物理来源。
放置项不另存 Reservation，占用仍归 View。

`owned()` 只读 Managed 的完整物化状态，Borrowed 不算自有 RAM；该查询不物化、不分配。
因此既有 Mmap(Pole) 的 RAM 限制不变；设备区域仍以借用来源经 Pie Open 建立普通 CPU View，
不清零、不按 RAM 回收，也不为单纯持有占内核 VA。

`Grant` 原子保存权限位；Pie 与快照共享同一个 Grant。权限收窄用原子比较交换，失效将其清零；快照不会延长已失效授权。Pole 的操作 lease 序列化创建、打开、撤销和交付；同一时刻已有 lease 时相关操作以 Busy 失败。

ONLY 根资源的创建权通过 `CreationRegistry` 一次性交付。注册项以 `Weak<View>` 关联候选关系，查询与 claim 使用 `Arc::ptr_eq` 的实际关系身份；registry 不用可复用的 VA 或 ASID 数值代替身份。准备阶段先分配节点，关系发布后再提交登记。claim lease 在构造失败时恢复可用，成功提交后消费该权利并移除登记项。

## 失效、锁序与错误

`Nest` 修改页表后先完成本地及远端 TLB 失效，完成后才丢弃延迟保留的 Frame、TableNode 和
`View`。每个 reservation generation 在 PTE 发布前由占用表预留一个 `Box<CleanupNode>`，
因此 `Memory::dispose` 接管已发布映射时不需要再分配清理队列节点。Nest 在准备撤销时仍可能
因 OOM 返回错误，未完成的 cleanup node 留在链头，不丢失清理责任。失效状态为
`Clean / Dirty / Fencing / Failed`：进行中让并发操作等待；失败保留资源，重试进入 Fencing，
成功后才清空记录并在锁外释放。关闭状态为 `Open / Closing / Closed`，只在失效完成后落为
Closed。永久失效失败时资源继续保留，不能宣称已经回收。已发布的挂载/撤销在同步失败时保留
必要的 Frame / 表页 / View / 来源、退休票与“VA 禁复用”，沿用原身份重试；改权的同步失败
只保留实际已改的权限与活动关系，不进入 Closing。`Memory::unmap` 先结算已有待失效操作，
再按活动库存逐段撤下；没有活动区间时可幂等交还占用，并只回收传入 reservation 的旧
generation，不能释放后来占用相同 VA 的新 generation。销毁普通 Nest 时先撤掉其关系，
完成 shootdown 后才回收 ASID；内核 Nest 返回 `Pinned` 并保留进程期根页表。`View` 的弱
Nest 关联让树关闭或销毁后的旧 `View` 无法操作新 Nest。

`machine::sync::Level` 的递增顺序为
`Unit < Scheduler < Space < Nest < View < Flat < TeamTasks < L3 < Roster < Asid < Frame < Block < Tally < Spare < UnitState`。
全局 VA 占用取 `Level::Space`，在页表/关系锁之前；`Flat` 是页面来源槽的锁，位于 `View`
之后、`Frame` 之前：`View` 在自己的锁下取到物化页并发布 PTE，`Flat` 从不回调 `Nest` /
`View` / `Pole`。索引锁内不回取低级地址锁，退休资源在锁释放后才 drop。新增锁必须按严格
递增次序取得；debug 构建会检测递归获取、倒序和跟踪容量越界。锁与其守卫分别定义，守卫
作为锁的子模块访问私有字段。持锁集长度由 `Vec::len()` 给出，不另存一份计数；8 个槽是初始
容量，不是不可扩展的上限。

Pole 的授权集合与事件 ring 各用 `Level::L3`，两者不共持；
授权集合与实际 View 集合分别准备和访问，不嵌套取得同级锁。

ASID shootdown 总是刷新本 hart。远端 kernel shootdown 依据 `ONLINE` 原子位图，用户地址空间 shootdown 依据各 hart 当前 ASID lease。次级 hart 在启动入口发布 online 位（Release），shootdown 侧用 Acquire 读取；切换地址空间时 ASID lease 也以 Release 写、Acquire 读。这样 kernel 映射修改会覆盖已 online 的 hart，即使其当前 lease 不是该 kernel ASID。

保留的 `NestError`（旧名 `MapError`，变体与线上码未变）覆盖物理内存不足、未对齐、
已映射/未映射、无所属区域、权限上限不可加宽、DRAM 布局重叠、shootdown 失败和内核
Nest pinned。物理来源自己的失败码是 `flat::FlatError`（不足、未对齐、越界），不依赖 wire；
它只在 `NestError` 处折成地址空间失败码（`OutOfRange` 折为 `NoRegion`）。`Flat` 不认识
线上编码；`From<NestError> for MemoryFail` 属于外层组合，不放在纯域错误文件里。普通接口
保留既有失败码映射，shootdown 未完成对应 Busy，不丢弃映射资源或重用 reservation。

## 既有 wire 接口

`UnsealArgs::Pole` 携带 `{ addr, size, shared }`，wire 参数为五个机器字，Pole variant 编码为 `[1, addr, size, shared, 0]`。Pole 创建要求指定一段已有、完整的映射。

`MemoryCall::Mmap` 使用 `pole: PieToken`，wire 槽位保持不变。Memory 接口与内部类型归位不改变这些既有编码、错误码或诊断 schema；`Frame`、`Nest`、`View`、`Space`、`Mapping` 与 `Reservation` 的 Rust 实现接口不是用户 wire ABI。

## 先前页表迁移的字段计数

下表保留先前 CPU 页表与资源路径迁移的测量，不代表后续类型归位或 Memory 接口迁移后的工作区读数。该次测量使用同一份词法扫描器读取迁移前后 `kernel/src` 的所有 `.rs` 文件，包括当时未跟踪文件。扫描器剥除注释和字符串，跳过宏输入，识别具名 `struct Name { ... }`，按括号、方括号、大括号和泛型括号的嵌套层级统计顶层字段；enum、tuple struct 与 unit struct 不计。

| 范围 | 迁移前 | 该次迁移后 | 净变化 |
|---|---:|---:|---:|
| 全部 `kernel/src` | 125 个 / 400 字段 | 135 个 / 429 字段 | +10 个 / +29 字段 |
| `memory/manager` 与 `work/unit/space` | 20 个 / 65 字段 | 22 个 / 74 字段 | +2 个 / +9 字段 |

字段迁移不代表字段净减少。表中计数包含真实新增状态、旧职责归位后的字段和仍保留的缓存；不能把职责迁移或删掉旧封装称为字段净节省。当前真实新增状态包括 reservation generation 身份、Map 子范围/lazy 与失效保留状态、Space cleanup 记录、allocator 中按 generation 预分配的 cleanup 节点，以及 Pole 的实际映射/授权关系。旧 Map 上的 AVL 索引与回收链字段已从 Map 移走，由 Tree 内部索引和 shootdown 延迟保留结构承担；旧 `Backing`/`Permit` 混合记录和重复 VA/物理地址包装已删除或由真实 `Frame`、`Map`、`Reservation` 关系取代。Task 陷阱帧物理/虚拟地址仍是供快路径使用的真实缓存。故字段总数如实净增 +29（manager+space 净增 +9），结构职责仍已归位，不能据此声称净减字段。表中 `Tree` / `Map` / `manager` 是该次测量当时的名字；PR 2 把它们归位为 `Nest` / `View` / `nest`（见下一节）。

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

- `core/memory`：公共坐标（`address`）、物理来源与分配器（`flat`）、CPU 地址空间的全部状态（`nest`：身份、页表、全局占用、关系、失效与回收）。
- `core/work`：运行实体、通信、授权、调度、等待和记录。
- `machine`：机器信息、每核执行信息、时钟、上下文、同步和处理器停机机制。
- `entry`：启动装配、异常与调用接入、资源交付、报告和测试接入。

本仓模块写 `crate::core::…`，Rust 核心库写 `::core::…`。

| 契约 | 核心或机器事实 | 接入与组合 |
| --- | --- | --- |
| 翻译与上下文 | `Nest::satp` 编码；`machine/switcher` 安装寄存器 | Task 选择空间与 SPP；启动、探测、恢复保留写入时机 |
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

内核内用例位于 `kernel/tests/health`。白盒用例通过受控路径（`Memory::backend()`）取得裸
`Nest`，没有为迁移测试公开生产字段；登记仍由 `kernel/tests/embedded.rs` 完成。
Pend 与 Trace 的宿主套件通过 `#[path]` 编译生产实现，运行时必须显式指定宿主 target。
`kernel/tests/frame-runs` 同样按 `include!` 夹入生产的活动区间与帧分段三份源文件，
只补叶子依赖，不镜像分段算法。`crates/alloc-probe` 被 workspace 排除，但直接 include
生产分配器，须从该 crate 目录单独验证，让其 CFLAGS 与串行测试配置生效。

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

（以下归位描述中的 `manager` / `Tree` / `Map` / `VaAllocator` / `MapError` 是该批次当时的名字；
PR 2 后来把它们归位为 `nest` / `Nest` / `View` / 占用表 / `NestError`，见「PR 2 归位验收」。）

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

### PR 2 归位验收

本轮把 CPU 地址空间从旧 `manager` / `Tree` / `Map` 归位为 `core/memory/nest` 的
`Nest` / `View`，并给出四个关系原语与自己的同步退休。实际源码布局：

```text
core/memory/nest/
  mod.rs                Nest：身份、根页表、占用、关系索引、失效与退休
  state.rs              NestState：待失效、退休资源、待结算的占用票
  error.rs              NestError（纯域；From<FlatError> 在此，MemoryFail 折在外层）
  address/              Placement / PlacementAt / RegionId / Occupancy / Window / Slot
  identity/             Asid、shootdown、本地刷新与回收
  table/                TableNode / PageTable / PageTableEntry / PteFlags
  synchronize/          Invalidation / ClosePhase
  index/                关系索引（AVL 根 + 节点）
  view/                 View / ActiveRange / ViewRange（Whole 标记与 Pages 坐标）/
                        Access / 帧分段
  reservation.rs        Reservation（带 generation）
  cleanup.rs            清理链上的一环：一笔已消费、待收尾的关系
core/work/unit/space/memory.rs
                        Memory：只持 Arc<Nest> 的外层薄组合
core/work/unit/space/mapping.rs
                        Mapping：线外兼容名，只持 Arc<View>；核心不认识它
kernel/tests/health/nest.rs / nest_failure.rs / depend.rs
                        原语、失效失败与锁序自检用例
```

- `Nest` 提供 `reserve` / `attach` / `set_access` / `detach` 与自身 `close` /
  `retry_invalidation`；改权、撤销和物化 / 发布组合收 `ViewRange`（`Whole` 是操作时解析的标记，
  `Pages` 是创建时定下的页坐标，选到已结算的页就失败，不静默过滤）。
  `detach` 逐段走“退休该段占用 → 撤 PTE → 同步 → 只回收这一段”，失败时票与
  退休资源都留着、VA 禁复用，沿用同一身份重试才结算；部分撤下成功就只交还那一段。
  普通挂载与活动形状挂载共用一个私有安装/提交，备齐可失败项之后才写 PTE。
  撤销也先准备全部可失败项，再在地址 → Nest → View 的递增临界区内
  开票并提交 PTE / 活动 / 索引 / 退休责任；锁外完成远端同步。
  准备失败时原 live 占用与 PTE 不变，没有无人接管的票，也不提前 reclaim。
  没有 `PreparedView` / `ViewRecord` / `Handle`，也没有可伪造的 PA + Frame 投影输入：
  内部形状入口只接真实 occupancy、`FlatRange` 和明确的活动区间。
- 占用表一份全局有序表：`Lowest` / `Highest` 共用窗口选址，`Exact` 支持窗外真实跨度，
  三者共用同一份重叠检查；端点按最后一个字节（含）预测，因此
  `TRAMPOLINE = 0xffff_ffff_ffff_f000`、长度 4096、最后字节 `usize::MAX` 的跨度
  可登记/切分/退休/回收，也不用 `[0, max)` 固定桶。`Static` 只是地址记账政策：
  它没有动态窗口，跨度只按 `Exact` 登记，`Occupancy::attach` 不给它建窗。
- 物化只有一条原语：`FlatRange::materialize`。`Nest::publish` 只发布已物化 / 借用的
  页；缺页与派生编排走组合入口（`Flat` 物化一页或源活动片段，再回到本关系发布），
  它不充当第二套物化权威。PTE 真值在页表，发布记录只负责强持与双向核验。
- 权限拆成逻辑 `Access`（RWX）与整段架构属性（U/G/A/D）：`set_access` / `narrow`
  不改属性；改权同步失败保留已改的权限与活动关系，只有关才进 Closing。
- 启动布局：低 identity 已在 `Normal` 内整体登记，按段 `split` 后逐个挂载；
  `advance_region` 只抬高选址下界；高镜像、跳板、hart frame、trap stack 按 `Exact`
  登记真实占用（`RegionId::Static`）；用户 Nest 的跳板随该 Nest 回收。

本轮实际命令与结果（全部在本工作区重跑；无专项多 hart 并发与完整分配器元数据 OOM 注入）：

- workspace debug/release、`kernel --all-targets` debug/release、semihosting 两档编译通过；
  已处理的警告为 0。
- **锁序检测本轮才真正启用**：`init_depend` 从启动路径移到公共 `init`
  （分配器就绪之后、被验证调用之前），并新增 `depend::armed` 自检用例；
  启用后冒出的两处真实次序问题按原位修掉（跨关系审计不套着关系锁比较快照；
  团队暂存表的扩容 / 合并 / 释放全部移到状态锁外），没有关检测、没有吞 panic、
  没有拿无级别的锁绕开。
- `nu scripts/qtest.nu --package kernel`：48 项健康用例通过（含新增的锁序自检）；
  无 initrd 的 `scene` 哨兵单独失败（预期），命令退出 101。`--scene accept` 1 项通过。
- Pend 宿主 35 项、Trace 默认/semihosting 各 2 项、frame-runs 9 项、console-line 5 项通过。
- env 宿主 2+4+3+4 项、resource capability 16 项通过。
- alloc-probe 从其自身目录：all-targets 宿主检查、默认 6 项、plain 10 项、多核 4 项通过。
- `git diff --check` 通过；`cargo fmt --all -- --check` 的 Diff 块由 173 降到 154
  （只格式化了本轮新建/重写的文件，未全树重排）。
- 新增/改写的健康用例覆盖：登记不动页表、guard 切分占用、顶页 Exact 与跨窗口全局重叠、
  已撤选择/关闭空间拒新关系、改权同步失败保持活动、内核 Nest `Pinned`、大叶分裂、
  非连续帧按页翻译、缺页逐页发布、跨 Nest 同源物理别名与逐侧撤销、空洞派生不重发布、
  发布失败回滚责任、失效失败后 VA 禁复用与原身份重试。旧的净零与“最后一份来源释放才回收”
  断言未放宽。
  另有中间 Pages / Whole 的撤销准备失败注入：原活动与 PTE 不变、
  retry 不释放 live VA、原选择可正常再次撤销且最后回到净零；
  物化选择拒绝关闭页、Whole 只处理活动片段不重装洞页。

未随本轮闭环（保持原状）：`system-fault` 的 Loader 超时按上文仍未验收；
`--scene product` 未在本轮重跑（本轮只跑了任务要求的 `accept`）；
semihosting 外层 JSONL/fs 仍未做整机导出验收。

## PR 3 来源消费者验收

Pole 直接持 FlatRange；Grant 授权、活跃 View 与 Input 关闭责任分别记录，
不再为 RAM 持有或设备借用创建 keeper。普通 Open / Mmap 只消费授权来源，
原 wire 编码、错误码、ONLY / VEST、构造域与 Spawn 原子消费不变。
来源状态判定不隐式物化，也不把 MMIO 放宽成旧 Mmap 可以接受的自有 RAM。

主工作区检查：

- workspace、kernel all-targets、semihosting 的 dev / release 以及 programs all-targets 通过。
- 健康轮 51 例：50 passed、缺 initrd 的 scene 哨兵 1 failed，退出 101；
  锁序自检与新增 Pole 来源 / MMIO wire 用例实际通过。独立 reviewer 得到相同结果。
- accept、product 默认退出场景各 1 passed；system-fault 仍在 Loader 首次构造返回
  Denied(6)，继而超时、服务级联，签名与 PR 3 前相同，未宣称通过或修改超时门槛。
- 宿主 frame-runs 9、Pend 35、Trace 默认 / semihosting 各 2、console-line 5、
  env 13、resource capability 16、alloc-probe 库 6 + 多线程 4 通过；
  alloc-probe all-targets 检查通过。env 的既有 unused Wire 警告仍在。
- 新增断言包括不额外占 kernel VA / 表页、Shut / Revoke / 根 Release 关闭多处 View、
  全 View 关闭后来源可重挂、借用内容不变与不回 RAM、权限和偏移错误码；
  一页 Allocate / Deallocate 帧表净零与重复 Deallocate 拒绝。
- `git diff --check` 通过；格式检查 146 个 Diff 块，基线 154，只处理本轮修改范围，
  没有全树格式化。

详细失败责任与剩余迁移入口见 `docs/memory-flat-nest.md` 的 PR 3 节。
Memory facade / 形状测试桥及六处来源准备失败的占用交还责任必须由 PR 4 收口；
新 ABI / TLS 包装、System 独立保留和真实 IOMMU / DMAC 尚未实施。
