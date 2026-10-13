# Frame、Tree、Space 与 Pole

状态：最终结构记录（2026-10-13）。本记录描述当前工作区实现；CPU 页表与内存资源路径已迁移。IOMMU/设备页表没有实现。

## 资源关系

```text
Space ──Arc── Tree ──私有区间索引──Arc── Map ──Arc── Frame
                                        │
                                        └──Weak── 所属 Tree

Pole ──单一 HeldMap 集合──Arc── 实际 Map（及授权、打开位置与关系）
Mapping ──Map Arc + VA Reservation
Space ──Tree + VA allocator + shootdown 清理队列 + kind/life
Task ──Space 中的栈映射、陷阱帧映射和 guard 预留
Team ──共享 Space 与构造/成员状态
```

`Frame` 用分配器提供的 `Box<UnsafeCell<[u8; PAGE_SIZE]>>` 持有一页物理内存。它的 `Arc` 是帧共享和释放的所有权依据。`BorrowedRegion` 单独表示调用者保证仍被保留的物理区间，创建它是 `unsafe` 操作；它不冒充分配器拥有的 Frame。

`Tree` 持有 ASID、页表几何规则和根页表状态。它创建、查询、拆分、保护和移除 `Map`。`kernel/src/memory/manager` 不引用 `work`；任务、团队与 Pole 的组合留在工作层。`Map` 仅保存所属 `Tree` 的 `Weak`、初始 VA 范围和映射状态；状态保存仍存活的子范围、所持 Frame、借用区间与 lazy 标记。Map 自身没有 AVL 左右链、高度、最大区间或回收链字段。区间查找用的 `IndexNode` 和 shootdown 期间保留的 Map/Frame/页表页都属于 Tree 内部状态，没有 `work::unit::space::Index` 外观层。

`Space` 组合 `Arc<Tree>`、VA 分配器、延迟清理链、地址空间 kind 和等待生命期。清理链头直接是 `SpinLock<Option<Box<CleanupNode>>>`；没有单字段 `PendingCleanup` 包装。普通区间在构造时已接入；`attach_region` 用于配置已有边界，镜像装载后的 `advance_region` 单调抬高分配下界。内核 Space 另有 `RegionId::Resource`，范围为 `[mode.lower() + DRAM.base + DRAM.size, TRAP_STACK_BASE)`；内核 keeper 和 device 大区映射从此区域选址。Team 的 `RegionId::Kernel` 仍限于 64 MiB 的 frame-context 窗口。该分区让 256 MiB PCIe 映射不再受原 64 MiB 窗口限制，且没有恢复 `Window` 或 `Segment`。查询、物化和保护直接针对 `space.tree`。Space 仍执行用户地址范围、U 位和 supervisor 策略检查；这些访问策略不是 Tree 的薄转发。

`Reservation` 持有 Tree 弱身份、区域、起点、长度和 allocator 生成的 reservation id。Map 与该预留组成 `Mapping`。Space 的 reserve/allocate/map/derive/unmap 操作负责把完整 VA 预留与 Map 生命周期协同起来，并通过 reservation id 防止过期或错误的预留释放当前区间。

Task 的 `TaskMemory` 持有栈和陷阱帧的 Mapping 及 guard Reservation。显式退休通过 `Space.unmap` 完成；`TaskMemory::drop` 把尚存的 `Mapping` 交给 `Space.dispose`，由 Space 持有清理责任。`TaskIdent` 缓存陷阱帧的 `PhysAddr` 和 `KVirtAddr`，供调度和陷阱路径直接访问；这两个缓存对应同一个由 Mapping 保活的帧，不取代其所有权。Team 持有共享 Space；任务发布与 Pole 私有映射交付仍由 Team 的构造状态控制。

## Pole 与授权

Pole 的唯一映射记录集合是 `Vec<HeldMap>`。每项关联实际 `Arc<Map>`、可选 token/`Arc<Grant>`、可选的 `Weak<Space> + Reservation` 放置位置，以及 Input、Keeper、Authority、Open、View 或 Staged 关系。创建 Pole 时从完整、已物化的现有 Mapping 建立输入映射，并在内核 Tree 中派生 keeper 映射；打开时从仍存活的规范来源派生目标 Space 的 Map。Pole 不持有裸 Frame，也不维护单独的 `source_map` 字段。来源通过集合中的 live Keeper 或 live placement 选择。

`Grant` 原子保存权限位；Pie 与快照共享同一个 Grant。权限收窄用原子比较交换，失效将其清零；快照不会延长已失效授权。Pole 的操作 lease 序列化创建、打开、撤销和交付；同一时刻已有 lease 时相关操作以 Busy 失败。

ONLY 根资源的创建权通过 `CreationRegistry` 一次性交付。注册项以 `Weak<Map>` 关联候选映射，查询与 claim 使用 `Arc::ptr_eq` 的实际 Map 身份；registry 不用可复用的 VA 或 ASID 数值代替身份。准备阶段先分配节点，映射发布后再提交登记。claim lease 在构造失败时恢复可用，成功提交后消费该权利并移除登记项。

## 失效、锁序与错误

Tree 修改页表后先完成本地及远端 TLB 失效，完成后才丢弃延迟保留的 Frame、TreeNode 和 Map。每个 reservation generation 在 PTE 发布前由 `VaAllocator` 预留一个 `Box<CleanupNode>`；因此 `Space.dispose` 接管已发布映射时不需要再分配清理队列节点。Tree 在准备 unmap 时仍可能因 OOM 返回错误；Space 会把未完成的 cleanup node 放回链头，不丢失清理责任。普通进行中的 shootdown 会让并发操作等待完成；真正失败后另设 failed 状态，返回错误并保留 pending 标志和待释放资源，后续通过 `retry_invalidation` 重试。永久失效失败时资源继续保留，不能宣称这一路径已经回收。`Space.unmap` 先完成已有 pending fence，再检查 Map；空 Map 可幂等成功，并只回收传入 reservation 的旧 generation，不能释放后来占用相同 VA 的新 generation。销毁普通 Tree 时先撤掉其映射，完成 shootdown 后才回收 ASID；内核 Tree 返回 `Pinned` 并保留进程期根页表。Map 的弱 Tree 关联同时让树关闭或销毁后的旧 Map 无法操作新 Tree。

`lock::Level` 的递增顺序为 `Unit < Scheduler < Space < Tree < Map < TeamTasks < L3 < Roster < Asid < Frame < Block < Tally < Spare < UnitState`。新增锁必须按严格递增次序取得；debug 构建会检测递归获取、倒序和跟踪容量越界。

ASID shootdown 总是刷新本 hart。远端 kernel shootdown 依据 `ONLINE` 原子位图，用户 Tree shootdown 依据各 hart 当前 ASID lease。次级 hart 在启动入口发布 online 位（Release），shootdown 侧用 Acquire 读取；切换地址空间时 ASID lease 也以 Release 写、Acquire 读。这样 kernel 映射修改会覆盖已 online 的 hart，即使其当前 lease 不是该 kernel ASID。`mark_hart_started` 只记录启动请求成功的 hart 数，不替代 online 状态或 ASID lease。

保留的 `MapError` 覆盖物理内存不足、未对齐、已映射/未映射、无所属区域、借入映射不可加宽、DRAM 布局重叠、shootdown 失败和内核 Tree pinned。工作层在 envcall 边界保留其既有失败码映射；shootdown 未完成时返回错误不会丢弃映射资源或重用 reservation。

## ABI 可见变化

`UnsealArgs::Pole` 现在携带 `{ addr, size, shared }`，wire 参数仍为五个机器字，Pole variant 编码为 `[1, addr, size, shared, 0]`；旧编码 `[1, size, shared, 0, 0]` 的调用方须更新。Pole 创建要求指定一段已有、完整的映射。

`MemoryCall::Mmap` 的公开字段由 `backing: PieToken` 改名为 `pole: PieToken`，wire 槽位不变；Rust 调用方须使用新字段名。`Frame`、`Tree`、`Map`、`Space`、`Mapping` 与 `Reservation` 的实现接口也已按新所有权关系调整，它们不是稳定的用户 wire ABI。

## 结构字段计数

采用同一份词法扫描器读取 Git HEAD 与当前 `kernel/src` 中的所有 `.rs` 文件，包括当前未跟踪文件。扫描器剥除注释和字符串，跳过宏输入，识别具名 `struct Name { ... }`，并按括号、方括号、大括号和泛型括号的嵌套层级统计结构体顶层字段；enum、tuple struct 与 unit struct 不计。该口径在 HEAD 重算得到 125 个结构、400 个字段，与原基线一致。

| 范围 | Git HEAD | 当前工作区 | 净变化 |
|---|---:|---:|---:|
| 全部 `kernel/src` | 125 个 / 400 字段 | 135 个 / 429 字段 | +10 个 / +29 字段 |
| `memory/manager` 与 `work/unit/space` | 20 个 / 65 字段 | 22 个 / 74 字段 | +2 个 / +9 字段 |

字段迁移不代表字段净减少。表中计数包含真实新增状态、旧职责归位后的字段和仍保留的缓存；不能把职责迁移或删掉旧封装称为字段净节省。当前真实新增状态包括 reservation generation 身份、Map 子范围/lazy 与失效保留状态、Space cleanup 记录、allocator 中按 generation 预分配的 cleanup 节点，以及 Pole 的实际映射/授权关系。旧 Map 上的 AVL 索引与回收链字段已从 Map 移走，由 Tree 内部索引和 shootdown 延迟保留结构承担；旧 `Backing`/`Permit` 混合记录和重复 VA/物理地址包装已删除或由真实 `Frame`、`Map`、`Reservation` 关系取代。Task 陷阱帧物理/虚拟地址仍是供快路径使用的真实缓存。故字段总数如实净增 +29（manager+space 净增 +9），结构职责仍已归位，不能据此声称净减字段。

## 验证记录

最终检查针对当前工作区：

- workspace debug 与 release 编译检查通过。
- QEMU 内核健康检查 41 项通过。未带 initrd 的 `scene` 哨单独失败，报错为 `test scene requires an initrd`；这不是场景验收通过记录。
- release `accept` 与 `product` 场景各 1 项通过，包含实际镜像、自举、PCIe 资源区域及调用方迁移。
- gate 宿主测试 32 项、resource 宿主测试 16 项、env ABI 宿主测试 13 项通过。
- `git diff --check` 通过；manager 不引用工作层类型，旧 Backing、SpaceInner、空间 Window、独立 Index 门面及 with_flush/with_shootdown 路径已退出。

健康检查覆盖共享 Frame 的单侧撤销与最终释放、Map 稳定身份、部分保护/撤销、大小页拆分、AVL 与硬件页表一致性、lazy/guard、失效失败保留与重试、ASID 复用身份、任务逐点失败和首次 Spawn 提交，以及 Pole 创建失败、重复 Open、关闭后重开、授权收窄/撤销和禁止重铸。相关检查比较帧、页表分配及真实授权关联，确认被测试的成功及恢复失败路径无残留。永久 shootdown 失败依约保留资源；本记录不将其列为已经释放，也不宣称设备 DMA 或 IOMMU 页表已经实现。
