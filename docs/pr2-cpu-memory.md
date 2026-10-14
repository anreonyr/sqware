# PR 2：CPU 地址空间组交接

本文件只带当前合同与执行入口，不带设计讨论历史。
每个 PR 使用独立的 namer / worker / reviewer 组；组内持续协作。
旧组封存检查点后才启动新 worker，不允许两个 worker 并行写同一阶段。

## 范围与完成检查点

PR 1 已完成：`memory/address/`、`memory/flat/`、稀疏页槽、
Managed / Borrowed 来源、FlatRange / Pages 与旧 Map 的来源关联。
来源级保留、别名和最终回收已有健康用例。

PR 2 已完成 Nest / Reservation / View，以及 CPU 页表、身份、
VA 占用和同步退休。新 ABI、Pole 改持 Flat、删除 keeper、
System 外部保留和 IOMMU / DMAC 属于后续 PR。

当前检查点：

- `manager` 与转发 / 影子副本已删除，CPU 权威归 Nest，来源权威仍归 Flat。
- MODE 唯一来源在 `machine/mode`；核心不依赖兼容收据或测试文件。
- 占用为 `Occupancy / Window / Slot`，使用全局有序表与 last-inclusive；
  旧 Memory 兼容 reserve 保留窗口门，新 Exact 支持窗外真实跨度。
- View 单持 Reservation 与 FlatRange；外层 Memory 只持 Arc<Nest>，
  外层 Mapping 只持 Arc<View>。wire 折叠留在线外。
- 真实锁序检测开启的健康轮为 48 passed + 缺 initrd 哨兵 1 failed；
  accept 通过。构建与宿主结果见 `docs/memory-flat-nest.md`。
- 同 PR 的最终命名与行为复核通过。最后两处修订由主控完成：
  类型化物化 / 发布选择，以及准备全部成功后原子开票与撤销。

## 从功能确定边界

| 功能 | 唯一权威 |
|---|---|
| 来源物化与数据页持有 | Flat；Nest 不保存另一套来源页槽 |
| VA 占用、选址、分割、退休票 | 一份全局占用存储 |
| 来源与占用的翻译关系、活动区间和逻辑权限 | View |
| 实际 PTE、表页与遍历 | CPU 页表 |
| 待失效、退休资源与失败重试 | Nest 的同步责任 |
| ASID 取得、安全复用及地址空间特权政策 | CPU 身份与平台机制 |
| 跨来源派生、启动布局、旧 wire 适配 | 外层组合，不是 Nest 总管方法 |

先确定功能、状态权威与依赖，再命名。独立功能不自动要求新类型或新锁。
每型一文件；不新增 PreparedView、ViewRecord、ViewHandle 或综合请求袋。

## 已确认的核心接口边界

- Nest 操作自身状态：`reserve`、`attach`、`set_access`、`detach`，
  自身 `close` / `retry_invalidation`。
- View 提供 `whole` / `range` 选择；选择带真实关系身份与初始页坐标，
  不按可复用 VA 重建所有者。
- Reservation 独立存在、可分割；View 接管其对应占用。
  兼容 Mapping 只持 `Arc<View>`，不能再持第二份 Reservation。
- Attach 校验所属 Nest、generation、等长和重叠。Managed 安装活动形状内
  当前可用的页，未物化部分可由缺页补齐；Borrowed 活动页全部安装。
  旧提前物化入口返回前必须具有完整 PTE。
- 普通 Attach 与兼容活动形状共用内部建立 / 提交路径。
  内部形状入口只接 FlatRange 与明确活动区间，不接源 View / 源 Nest，
  不接可伪造的 PA + Frame 投影袋。
- 缺页只物化一页，再发布本关系翻译。物化只属于 Flat；
  私有发布机制不成为新的公开生命周期调用。
- 权限 ceiling 与 current 分开，均为逻辑 Access；U/G/A/D 为独立
  架构属性。改权不改属性，也不从来源种类推特权。
- 上限收窄可保留可信内部入口；跨源派生与固定布局安装不挂在 Nest 上。
  不保留 `install_projected`、`install_fixed` 或跨 Nest share / remap 总管。

## 不可降低的行为合同

1. 来源物化、实际 PTE、关系活动、关闭 / 同步完成互不推导。
   页表是 PTE 真值，发布记录承担强持与双向核验。
2. 派生形状来自源活动区间与请求窗口的交。源保留的空洞页不得重发布；
   保留只物化源活动片段的旧副作用，不先 Attach 全窗再 Detach 空洞。
3. 改权同步失败保留实际已改权限与活动关系，不进入 Closing。
   真实撤销失败保留已撤 PTE、退休持有、VA 禁止复用与原身份重试责任。
   关闭一个 View 不关闭 Nest，活动空不等于已完成关闭。
4. generation / ticket 精确匹配，旧凭证不得释放同址新占用。
   同一 Nest 的静态与动态占用统一检查全局重叠，区域只过滤动态选址。
5. 占用使用 checked last-inclusive，支持顶页
   `TRAMPOLINE = 0xfffffffffffff000`、长度 4096、最后字节 `usize::MAX`。
   不用 `[0, max)` 固定桶，不截断溢出或穿越非规范 VA 空洞。
6. boot 低 DRAM identity 已有整体 Normal 预留，复用并分割；
   高镜像、跳板、hart frame、trap stack 登记实际 Exact 跨度。
   低 identity 不在动态窗口外；原 G/U 与 guard 布局必须保持。
7. 发布前预留清理载荷。借入关闭失败时调用者持凭证；
   消费式清理失败时责任归 Nest，不能丢失已消费输入或在 Drop 接管时再分配。
   退休 Frame / 表页 / 关系只在同步完成后锁外释放，ASID 回收先失效。
8. 锁序维持地址占用在页表 / 关系之前，Flat 只向分配器取锁；
   不在持高层锁时回取低层占用锁，不为概念分组随意增加嵌套。
9. CreationRegistry 仍绑定真实关系 Weak 身份，单次 claim / consume 不变。
   不改绑 Flat；ONLY / VEST staging、构造域、Spawn 与旧 ABI 保持。

## 归位与验证

最终 `manager` 转发和 Nest 对旧 manager 的反向依赖须退出。
兼容 Memory 如保留，只是外层 `Arc<Nest>` 组合，无 kind、VA 存储、
cleanup 的第二权威；core 不反向引用工作层。纯域 NestError 与 wire 折叠分离。
同步所有生产引用与 frame-runs / pend / trace / alloc-probe include 夹具，
不镜像生产算法。详细既有行为见 `docs/space-model.md`。

最终执行 workspace / kernel debug、release、all-targets、semihosting 检查，
`nu scripts/qtest.nu --package kernel` 与 `--scene accept`，
显式宿主 target 的 frame-runs / env memory / pend / trace 与受影响的 alloc-probe。
新增 Reserve / Attach、guard、跨 Nest 同源、局部改权 / 撤销、
旧选择拒绝、关闭重试、发布失败责任、权限同步失败、
顶页 / 全局重叠及旧派生 / 授权 / 资格回归用例。

保留所有安全与最终净零判据。不全树格式写入，不把缺镜像哨兵称为全绿。
未动态覆盖的并发与元数据 OOM 路径如实记录。

## 失败责任与验证边界

撤销事务先完成可能等待的同步，再按现锁级取得
地址占用 → Nest → View，保护准备和提交。
所有可失败的元数据、索引和页表几何准备在开退休票之前完成；
开票后以非失败 apply 将票、PTE、活动区间、索引和退休责任一起提交。
释放这些锁之后才等远端失效和结票。没有暂态票留在占用表却无人认领，
也不用 reclaim 在准备失败时错误释放仍然 live 的 VA。

注入测试分别覆盖中间 Pages 与 Whole 的准备失败：原 PTE / 权限 / 活动不变，
显式 retry 后同址 reserve 仍拒绝；同一选择再次撤销可完成且 VA 可再用，
所有 Frame 的最后引用释放后回收、表页净占用为零。

物化 / 发布选择只解析 active，不能拿撤销所用的 owed
（活动 ∪ 未结算退休）重新发布已撤页。Pages 必须整窗活动；
Whole 跳过已结算的洞。新增测试确认洞页不能物化发布、Whole 不补洞。

其它分配器元数据 OOM 和多 hart 并发仍未专项强制覆盖。
Team 暂存表锁外换表目前靠实际调用者的 operation lease 串行化；
将来扩展调用者时须继续保持该前提，不把它当作无条件的并发接口。
