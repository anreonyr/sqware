# Flat / Nest：职责、迁移与验证

## 范围与执行基线

PR 1 已完成：完整物理来源模型及旧 CPU 映射路径的来源适配。
PR 2 已完成：Nest 的地址占用、翻译关系、失效与退休回收。
PR 3 已完成：Pole 直接持授权来源，普通来源消费者不再经过 keeper。
PR 4 已完成：迁移 facade 与空包装删除，失败责任和测试保真收口。
新 ABI、System 独立保留和设备服务仍属后续阶段。

执行基线为 `master`，`03eabbde92586a6d6cf398d9a7a0b20e17b099c5`。
开始验证时工作区干净。仓库内没有 `AGENT.md` 或 `AGENTS.md`。
既有 CPU 内存合同见 `docs/space-model.md`。

## 已确认的功能模型

### 物理来源

Flat 表达线性逻辑页来源，不含 VA、页表、Task 或授权账目。
来源按持有与回收合同区分：

- Managed：分配器管理的 RAM。未物化页按需取得清零的 Frame；
  同一逻辑页只有一个物化结果。
- Borrowed：外部保证存活的物理区间，包括内核镜像、trampoline 与 MMIO。
  Flat 不分配其数据页、不清零、不把底层存储归还 RAM 分配器。
  `BorrowedRegion` 的元数据引用不代替平台的存活保证。

不引入 eager / lazy 策略类型。物化时机由调用方组合决定，
不能成为来源分类或 `Allocation` 的策略字段。
`Allocation` 描述逻辑页数与物理分配约束；物理对齐不等于物理连续。
不支持的约束必须明确拒绝，不能接受后忽略。

FlatRange 是强持同一 Flat 的校验范围。子范围用相对偏移选择，
合成后仍使用原来源身份，不建立父范围链，不复制数据。
Pages 表达整个范围已完整解析且稳定持有：
Managed 必要时物化；Borrowed 校验并保留现有借用区间。
Pages 不限定为 Managed，不以裸 PA 数组代替持有。

### CPU 翻译与授权

Nest 的功能包括 CPU 翻译身份、VA 占用、翻译关系、页表操作和同步退休。
View 是来源范围与地址占用之间的翻译关系；
Reservation 是可独立存在的 VA 占用，guard 不需要来源或 View。
页表机制归 Nest，不保留 `Tree / Map / Mapping` 作为最终语义枢纽。

以下事实各有唯一权威，不能互相推导：

| 事实 | 权威 |
|---|---|
| 来源声明的逻辑范围 | Flat / FlatRange |
| 逻辑页是否已物化 | Flat 页槽 |
| PTE 是否实际存在 | CPU 页表 |
| View 的活动区间 | 翻译关系记录 |
| 关闭与同步是否完成 | 同步、关闭和退休状态 |

活动区间为空不等于关闭完成。同步失败必须保留必要物理持有、
VA 禁止复用状态、原身份与可重试清理责任。
PA 是地址信息，不是持有凭证或设备访问授权。
来源授权、目标域授权、撤销依赖与创建资格留在 entry / pend / work；
不放进 FlatRange。

## 原语与适配边界

| 操作 | 单一效果 |
|---|---|
| `Flat::acquire` | 建立 RAM 逻辑来源，不提前物化 |
| `Flat::from_frames` | 可信内部接入合法既有帧，不清零、不改变 PA 或内容 |
| `Flat::from_region` | 保留外部借用物理区间 |
| `Flat::range` | 按来源位置选择范围 |
| `FlatRange::range` | 按相对偏移选择子范围 |
| `FlatRange::materialize` | 取得完整、稳定的 Pages |

PR 1 的签名约束：

- `PageIndex` 与 `PageOffset` 分别承载来源页位置与范围内相对偏移，
  不提供隐式互转。`PageCount` 使用非零页数，零由构造拒绝；
  页数折字节、子范围合成与 PA 偏移均使用 checked 运算。
- `Allocation::new(pages, align)` 的 align 是每页物理起址的对齐下界，
  不承诺连续性。本阶段只接受 `PAGE_SIZE`；其他值以 `NotAligned`
  明确拒绝。旧 ABI 的零尺寸规范化仍在原适配入口执行。
- `Flat::range` 和 `whole` 从 `&Arc<Flat>` 构造强持范围；
  FlatRange 自身可以从 `&self` 继续选择范围。
- `from_frames` 借用输入的稀疏有序索引与 Frame 引用，内部 fallible
  预留后克隆持有。越界、重复或乱序索引以 `OutOfRange` 拒绝，
  失败不消费调用方的输入引用；同一物理 Frame 的合法别名不误判为双释放。
- 页槽物化与可信帧快照只按实际槽数量分配元数据；
  高页号不得要求为全部前缀分配空槽。
- Pages 的逐页物理地址读取使用相对 `PageOffset`，不增加裸 PA 数组。
  旧发布适配的 `materialized_frames` 只返回稀疏已物化 Frame 的强引用，
  不物化、不保证整个范围完整，也不向借用来源制造 Frame。
  该状态快照不代替 Pages 的完整范围保证；发布前必须先取得完整物化结果。

Clone 和长度查询不物化、不登记授权。内核持有依靠对象生命周期，
不为动词对称新增 Release 原语或隐式 Detach。
后续 ABI 的 Acquire / Release 是调用者资源表登记操作。

物化失败可保留已经成功的页槽，但不返回不完整 Pages。
来源构造失败不公布来源；实现仍须回收内部临时资源。
既有帧接入失败不能丢失调用方的合法帧持有。
逻辑页索引唯一与物理别名不同：共享 `Arc<Frame>` 不构成双重释放。
数据页、容器元数据与引用对象的分配失败都进入失败域。

逐页 PA 读取通过 Pages 的可信内核访问边界，不另设隐式物化入口。
crate 可见性不代替后续 ABI 的运行时授权。
FlatError 是纯域错误，不依赖 wire；错误折叠只在适配边界完成。

## PR 1 的结构与迁移责任

公共坐标归 `memory/address/`：PhysAddr、VirtAddr、PageIndex、
PageOffset 和 PageCount。复用唯一的平台模式来源，不新增 VaBits 静态。
`address/virt.rs` 暂时仍读取 `manager::mode` 的唯一模式与几何；
这是 PR 1 的过渡依赖，不能把整个 address 模块报告为已脱离 manager。
Flat 使用的物理坐标与页坐标不依赖该虚拟地址模式；
PR 2 已将模式唯一来源移到 `machine/mode`，收口该临时依赖。
物理来源归 `memory/flat/`：Flat、FlatSource、Allocation、Frame、
FlatRange、Pages、BorrowedRegion、FlatError 与 allocator。
每型一文件，不新增 FlatState、Ram 或 PageSlot 包装。

旧 Map 的稳定状态直接持 FlatRange，不能把关联只放在可重建的 Mapping
收据里，也不建立全局 PA 或 Map memo 表。
同一 Map 重复查询取得同一 Flat；派生映射继承来源并合成偏移。

Flat 是唯一物化权威。迁移期旧 Frame 列表继续承担发布与退休所需的强持有，
不是第二套物化权威；失效完成前不得丢弃最后一份必要持有。
这种保留是迁移责任，不是最终 Nest 内再存一份来源页槽。
来源关联字段是本阶段真实新增状态，不能报告为字段净减少。

Flat 页槽只增不减。因此旧 Mapping / Map 或独立 FlatRange 仍持来源时，
部分或整段撤销 CPU 翻译不会立即归还该来源的数据页。
弱引用生命周期用例须同时证明翻译已撤、来源仍在时页仍在，
以及最后一份来源引用解除且同步完成后才回收；最终净占用仍须回到基线。
这项来源级保留是本轮明确的白盒生命周期变化，不能删除相关回收检查。

旧 `MapState.lazy` 暂作为旧 CPU 关系的迁移字段保留，
用于原缺页入口、派生触发和“立即发布关系缺叶即错误”的审计。
它不表示 Flat 是否物化，不能镜像到 Flat、Allocation 或新策略类型。
后续关系发布合同接管这些判断时，该字段随旧 Map 退出。
本阶段不以已物化槽或已发布帧的存在性替代完整发布期望的审计。

旧侧组合：

1. 验证原 Map 身份、活动范围与权限。
2. 取得强持的来源范围并物化所需页。
3. 在发布边界重新验证关系仍有效。
4. 发布 CPU PTE，按原合同完成失效同步或保留失败资源。

缺页只选择一页，不扩大为整个来源物化。
旧 Allocate 在返回前完整物化，保留提前物理 OOM；
旧匿名 Mmap 保持按需取得数据页。
派生路径保留原空洞形状与物化行为，不能先发布空洞再撤销。
CreationRegistry 在本轮仍绑定原 Map 身份，不提前改绑来源资格。
Pole 的 keeper、canonical_map 和普通授权合同留待对应阶段迁移。
共享是范围选择、Clone、Reserve、Attach 的组合，不新增 share / remap 总管。

## 基线验证记录

工具链：`rustc 1.100.0-nightly (a69a63265 2026-09-03)`、
`cargo 1.100.0-nightly (b2e9d5f9d 2026-09-02)`、
Nushell `0.116.1`、QEMU `11.1.2`。

以下是修改前实际执行的结果，不是最终实施结果：

| 命令 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 退出 1；183 个 Diff 块，81 个文件 |
| `nu scripts/qtest.nu --package kernel` | 46 passed，1 failed；退出 101 |
| `nu scripts/qtest.nu --package kernel --scene accept` | 1 passed，0 failed；退出 0 |
| `cargo test --manifest-path kernel/tests/frame-runs/Cargo.toml --target x86_64-unknown-linux-gnu --offline` | 9 passed |
| `cargo test -p env --target x86_64-unknown-linux-gnu --test memory --offline` | 2 passed |
| `cd crates/alloc-probe && cargo test --target x86_64-unknown-linux-gnu --offline --lib` | 6 passed |

无 scene 的失败是 `embedded::tests::scene`：
`test scene requires an initrd`。这是既有缺镜像哨兵；
不能把该轮称为全绿。accept 使用脚本规定的 release 镜像，
完成整机场景与探针成功退出检查。

格式检查是基线读数，不允许通过全树重排消除既有差异。
本轮没有执行格式写入。

健康用例 `pagetable_recycle` 验证 32 轮映射撤销后表节点回到基线，
且 `frame_occupied - block_occupied` 的净增为 0。
`pole_mapping_and_creation_authority` 验证生命周期结束后表节点净增为 0。
`kernel_memory_layout` 验证内核布局、guard、trampoline 与映射权限。
这些是实际通过的相对占用判据，不是整机绝对物理或 Resource VA 占用读数；
基线运行没有输出后两者的绝对值。

## 调用方清单与必须同步的消费者

在 kernel、env、execution、resource 和 programs 的源码中检索
`manager / Memory / Mapping / canonical_map / CreationRegistry /
Allocate / Deallocate / Mmap / Munmap / Mprotect`，命中 105 个文件。
检索大小写敏感、使用词边界，不含 tests；基线复现命令：

```sh
rg -l '\b(manager|Memory|Mapping|canonical_map|CreationRegistry|Allocate|Deallocate|Mmap|Munmap|Mprotect)\b' \
    kernel/src crates/execution/src crates/resource/src crates/env/src programs/src | wc -l
```

相关功能入口分组如下，路径记录执行基线；实施后的归位见下节：

| 消费功能 | 主要位置 |
|---|---|
| wire 编码与包装 | `crates/env/src/abi/call/memory.rs`、`crates/execution/src/memory.rs`、`crates/resource/src/raw.rs` |
| CPU 核心 | `kernel/src/core/memory/manager/{memory,tree,table,addr,asid,mode}/` |
| 资源授权与来源捕获 | `kernel/src/core/work/mail/pole/`、`kernel/src/core/work/pend/allocation/` |
| 创建、栈与镜像 | `kernel/src/core/work/unit/{task,team,space}/`、`kernel/src/core/work/unit/capsule.rs` |
| syscall 与缺页 | `kernel/src/entry/call/{memory,pie,unit}.rs`、`kernel/src/entry/trap/fault.rs` |
| 启动与硬件上下文 | `kernel/src/entry/boot/`、`kernel/src/machine/{switcher,hart,console}/`、`kernel/src/machine/layout.rs` |
| 查询与诊断 | `kernel/src/entry/report/`、`kernel/src/core/work/record/trace/` |
| 用户侧布局 | `crates/execution/src/{boot,unit}/`、`programs/src/system/loader/` |

移动底层类型还必须同步 `crates/alloc-probe` 的 include 路径和模块垫片、
`kernel/tests/frame-runs` 的生产模块树，以及 `kernel/tests/health/` 的直接引用。
新增锁级须同步 `docs/space-model.md` 的锁序记录。

## PR 1 实施记录

`memory/address/` 和 `memory/flat/` 已建立；整个 allocator、
Frame、BorrowedRegion 与物理/虚拟地址类型已归位。
旧 manager 不保留这些类型的转发别名。直接引用与宿主 include 垫片同步更新。
旧 Map 的非 Optional FlatRange 关联已接入所有构造路径；
派生沿同一来源合成偏移，仍从源 Map 的实际发布记录复制活动页。

旧 Tree / Memory / Map / Mapping、Pole keeper / canonical_map、
CreationRegistry 的 Weak<Map> 身份和旧 wire 编码仍保留。
旧 borrowed / borrowed_offset 字段由来源关联取代；
旧 lazy 只承担本节前述的 CPU 关系迁移职责。
这些兼容入口不构成最终 Flat / Nest 结构。

新增 `flat_source_model` 健康用例覆盖来源取得、幂等物化、范围组合、
稀疏既有帧接入、借用区域、错误与溢出输入、数据页耗尽后的部分物化重试、
旧 Map 撤销后来源保留、派生来源与空洞不发布。
同一 Flat 的不同逻辑索引共享同一 Frame，以及别名页的派生、
部分撤销和最终释放也纳入测试。12 个来源场景分别核对净占用回到基线；
旧共享、退休和失效失败用例保留最终回收检查。

Pole 用例中，局部 `peer_map: Arc<Map>` 在授权撤销后仍持来源。
用例明确验证 PTE 已撤但页仍存活，再 drop 该最后持有并验证页回收；
不能把残留强引用数等于 1 当作最终回收判据。

主工作区实施后实际执行：

| 命令 | 结果 |
|---|---|
| `cargo check --workspace --offline` | 通过 |
| `cargo check --workspace --release --offline` | 通过 |
| `cargo check -p kernel --all-targets --offline` | 通过 |
| `cargo check -p kernel --all-targets --release --offline` | 通过 |
| `cargo check -p kernel --features semihosting --offline` | 通过 |
| `cargo check -p kernel --features semihosting --release --offline` | 通过 |
| `nu scripts/qtest.nu --package kernel` | 47 passed，1 failed；退出 101 |
| `nu scripts/qtest.nu --package kernel --scene accept` | 1 passed，0 failed；退出 0 |
| `cargo test --manifest-path kernel/tests/frame-runs/Cargo.toml --target x86_64-unknown-linux-gnu --offline` | 9 passed |
| `cargo test -p env --target x86_64-unknown-linux-gnu --test memory --offline` | 2 passed |
| `cd crates/alloc-probe && cargo test --target x86_64-unknown-linux-gnu --offline --no-default-features` | 库 6 passed，多线程 4 passed |
| `cargo fmt --all -- --check` | 退出 1；175 个 Diff 块，77 个文件 |
| `git diff --check` | 通过 |

健康轮唯一失败仍为缺 initrd 的 scene 哨兵，新 Flat 用例通过。
accept 是实际整机验证，不以健康轮替代。格式差异少于基线，
未执行全树格式写入，不宣称格式检查全绿。

固定 reviewer 对当前实现完成核心静态审查和末轮差异复核，无功能阻塞。
末轮版本校验为 `flat/range.rs: f8562db79eba`、
`manager/tree/mod.rs: a3a7280b246c`、`health/flat.rs: ce466041630f`。
签名、词族、PA 边界、旧 CPU 关系门控、锁序与失效保留均与确认合同一致。
审查是只读证据，不代替上表实际构建和测试。

同一 Flat 在多个 hart 同时物化尚未做专项动态测试；
分配器的宿主多线程测试不代替该场景。
数据页耗尽的部分物化与重试已测，`from_frames` 等元数据预留的
独立 OOM 路径尚未强制注入，不能把前者报告为全部失败域均已动态覆盖。
没有执行其它整机场景或真实设备试验，不宣称 IOMMU / DMA 硬件隔离已验证。

## PR 2 执行边界

输入是已验证的 PR 1 工作区，不把 Git HEAD 的未变化视为没有 PR 1 实施。
本阶段按功能、结构、原语、签名推进，以下合同已由实现与回归验证。

- Nest 管理 CPU 翻译身份、页表、VA 占用、View 与同步退休责任。
  Flat 继续独立管理来源和物化，Nest 不建立另一份来源页槽。
- Reservation 可独立存在；Reserve 不建立数据翻译。
  Attach 消费对应占用与来源范围，建立长度匹配的 View。
  guard 由整体预留、分割、仅给 body Attach 的组合表达。
- 当前 CPU Access 与授权 ceiling 分开；授权不塞入 FlatRange 或 Placement。
  普通内部路径与可信启动布局使用明确的权限和 PTE policy。
- ViewRange 是操作选择，不是权限区间库存或所有权快照。
  每次操作验证所属 Nest、关系身份、活动与关闭状态；
  部分 Detach 不重排初始页坐标，不影响其它 View。
- Detach 同步失败必须继续保留必要物理持有、VA 禁止复用和原身份的重试责任。
  完整发布、关系活动、Closing、同步完成不互相代替。
  借入凭证的显式关闭失败时调用者仍持有它；接管凭证的清理入口失败时
  责任留在 Nest，不能把已消费的输入声称仍归调用者。
  发布前预留失败清理载荷，任务 Drop 接管已发布关系时不再分配该载荷。
- 地址占用与清理链当前使用 `Level::Space`，页表与关系使用
  `Level::Nest / View`。撤销的有界准备 / 提交按该递增顺序共持既有锁，
  全部可失败准备在开票前完成；远端同步和清理链推进只在锁外执行。
- 旧 Allocate、匿名 Mmap、Munmap、Mprotect、Open / Shut 的可观察合同保持。
  CreationRegistry 继续绑定稳定翻译关系身份，资格不在此阶段改绑 Flat。
  ONLY / VEST staging、构造域权限与 Spawn 原子消费保持。
- 静态 DRAM、镜像、trampoline、hart frame 和任务栈接入时保留
  kernel ASID 0、U/G/R/W/X policy、guard、指令同步与跨核失效。
  不能以占用或关系的空壳替代真实启动布局。
- 旧 Memory 兼容入口若仍存在，只能从外层组合 Nest 的原语，
  不能成为 Nest / Flat 的内部依赖，不建立 `nest/memory` 第二层总管。
  wire 尺寸规范化与错误折叠留在兼容 / entry 边界。

验收须分别证明 Reserve 与 Attach、跨 Nest 同源关系、局部改权与撤销、
关闭区间的旧选择拒绝、同步失败原身份重试、PA / VA 不提前复用、
guard 无来源，以及旧授权与 ABI 行为保持。

### 中间检查点：模式与 CPU 基础机制归位

唯一模式来源已从 `manager/mode` 移至 `machine/mode`；
VirtAddr 与其它消费者读取同一 MODE，不新增启动状态。
页表、ASID 及失效状态机的基础实现已归入 `nest/`。
worker 在此检查点实际运行构建、健康轮、accept 与相关宿主测试，
结果保持健康 47 通过加缺镜像哨兵 1 失败、accept 通过；
格式检查读数为 173 个 Diff 块。

这是历史中间检查点，不是最终结果。其后的实体实现已删除 manager 垫片，
完成纯域 NestError 与外层 wire 折叠分离，不保留这些反向依赖。
View 独占对应 Reservation；兼容 Mapping 不再持第二份占用凭证。

### PR 2 最终验证与审查

主工作区重新验证 workspace、kernel all-targets 和 semihosting 的
debug / release 构建均通过。真实锁序检测在公共 init 中启用，
新增自检明确验证 armed 与持锁数量；不能用旧未启用检测的数字替代。

| 实际检查 | 结果 |
|---|---|
| `nu scripts/qtest.nu --package kernel` | 49 例，48 passed，缺 initrd 的 scene 哨兵 1 failed；退出 101 |
| `nu scripts/qtest.nu --package kernel --scene accept` | 1 passed；退出 0 |
| frame-runs / Pend 显式宿主 target | 9 / 35 passed |
| Trace 默认 / semihosting 显式宿主 target | 各 2 passed |
| env `--tests` 显式宿主 target | 13 passed；signals 的既有 unused Wire import 警告仍在 |
| resource capability 独立 manifest 显式宿主 target | 16 passed |
| console-line 独立 manifest 显式宿主 target | 5 passed |
| alloc-probe all-targets 检查与无默认特性宿主测试 | 检查通过，库 6 + 多线程 4 passed |
| `git diff --check` | 通过 |
| `cargo fmt --all -- --check` | 退出 1；154 个 Diff 块，72 个文件，未全树重排 |

审查发现的兼容收据反向依赖、未消费 ViewRange 的旧接口、
临时锁跨作用域、健康轮未启用检测、核心反导入测试及撤销漏票均已修复。
准备失败回归验证原 PTE / 权限 / 活动与占用不变；
既不孤立退休票，也不 reclaim 仍 live 的地址。局部与整段选择均可重试并最终净零。

命名与行为 reviewer 对最终源码差异复核通过：
`nest/mod.rs: c47f60c99451`、`nest/table/mod.rs: f767194af899`、
`health/nest_failure.rs: 839b80c6abc9`。
多 hart 并发和其它元数据 OOM 路径仍未专项覆盖；
新 ABI、Pole 改持 Flat / 去 keeper、System / IOMMU / DMAC 仍属后续 PR。

## PR 3：来源消费者

PR 3 基线为 `f690fd054b3aa6a8402441b4163ffbc8e9b4a7ad`。
前面 PR 1 / PR 2 的 keeper 描述是各自检查点记录，本节描述当前实现。

### 功能、状态与失败责任

- `PoleMeta.source` 是唯一授权来源范围；来源长度从该范围派生，不再镜像存储。
  `Authority` 只关联 token 与 Grant；`HeldMap` 只记录实际 View、token、放置与关系种类。
  keeper 与 canonical 来源选择退出生产调用图；来源保留不占内核 Resource VA。
- `owned()` 只读 Managed 的整段物化状态，不分配、不物化、不用 Arc 数量判授权。
  Borrowed 不算 owned：旧 Mmap(Pole) 仍拒绝设备区间，合法 Pie Open 仍可显式挂载。
- 首份 Grant 关联 Input；提交前创建失败不接管 Input 的关闭责任，
  提交后 Input、Open、共享 View 与 Staged 继续受对应 Grant 撤销约束。
  Shut 失败保留原关系供重试；消费式清理失败由 Nest 接管，来源与 VA 禁复用责任不丢失。
- `map_range` 接收已有 FlatRange，再选址并挂载；普通挂载和显式活动形状挂载最终仍归
  Nest 的同一提交规则。惰性来源先建立再占位，缺页仍只物化并发布一页。
  来源页槽、关系活动形状与 PTE 发布状态仍是独立事实。
- 启动、Task 栈、trap frame、loader、内核临时访问与 TLS 调用链保留旧 ABI。
  不为已经正确的 Flat/Nest 组合机械新增入口或重写行为。

授权集合和事件 ring 各用 `Level::L3`，不同时持有；View 集合不嵌套持有授权锁。
开启锁序检测的健康轮包含相应创建、打开、收窄、关闭、撤销和 staging 路径。

### 行为证据

新增设备区域消费与 wire Mmap(Pole) 健康用例，覆盖：

- 创建 RAM Pole 前后 kernel Nest 的 View、表页与数据页账不增加；
  三个创建失败检查点保持 Input、创建资格与原 PTE，最终来源仍可回收。
- 全部 CPU View 关闭后仍从合法来源重新挂载并得到原 PA。
  同一 token 的多处 Open 经 Shut、Revoke 或根 Release 后全部撤下。
  撤销后的局部强引用仍可保留来源，最后引用消失后才回收。
- MMIO 接入不清零、不取得 RAM 数据帧、不占 kernel VA；
  Mmap 拒绝同一借用来源，而 Open 合法且内容保持。
- wire 检查无 STORE 拒写、跨构造域无 VEST 拒绝、有 VEST 成功；
  偏移非页对齐与越界仍返回原 NotAligned，ONLY/staging/Spawn 原子消费保持。
- 原 ABI 的一页 Allocate → Deallocate 后帧减块读数与 TABLE_LIVE 回到基线，
  同址同长第二次 Deallocate 返回 Denied。这不是实际 TLS 线程退出测试。

### 实际验证与限制

父工作区执行了以下检查；reviewer 另行独立运行 all-targets 编译和健康轮，
同样得到 50 项通过与缺 initrd 哨兵失败。

| 实际检查 | 结果 |
|---|---|
| workspace dev/release、kernel all-targets dev/release、semihosting dev/release | 通过 |
| programs all-targets | 通过 |
| `nu scripts/qtest.nu --package kernel` | 51 例，50 passed；缺 initrd 的 scene 1 failed，退出 101 |
| `nu scripts/qtest.nu --package kernel --scene accept` | 1 passed |
| `nu scripts/qtest.nu --package kernel --scene product` | 1 passed |
| `nu scripts/qtest.nu --package kernel --scene system-fault` | 退出 101，与 PR 3 前的 Loader Denied / 超时签名相同 |
| frame-runs / Pend / Trace 默认与 semihosting / console-line 宿主测试 | 9 / 35 / 2 + 2 / 5 passed |
| env / resource capability 宿主测试 | 13 / 16 passed；env 的既有 unused Wire 警告仍在 |
| alloc-probe 宿主 all-targets、无默认特性测试 | 检查通过，库 6 + 多线程 4 passed |
| `git diff --check`，使用从 HEAD 初始化的独立索引 | 通过 |
| `cargo fmt --all -- --check` | 退出 1，146 个 Diff 块；PR 3 基线 154，未全树重排 |

`system-fault` 的首个失败仍为 `programs/src/harness/probe/loader.rs:361`
返回 Denied(6)，随后 `:306` 与 `:190` 超时及服务级联；不是缺镜像哨兵，
不能称整机故障场景通过。product 的本轮证据是默认 EOF 退出场景，
不是登录或 Shell 交互脚本验收。

closure 线程有显式 TLS 释放路径，但未动态验证线程净零；
ELF bootstrap TLS 当前随 Team/Space 关闭回收，不声称按 Task 净零。
bootstrap TLS 出口和包装失败责任留 PR 5 的运行时 ABI 迁移收口。
其它元数据 OOM 和多 hart 并发没有专项强制覆盖。

### PR 4 必须接走的迁移责任

`Memory::view` / `view_record` / `derive_into` / `project` 等仅为形状与副作用测试桥，
PR 4 迁移等价测试后删除，不能以只有测试调用为由删掉保洞断言。
`attach_range` / `attach_range_shaped` / `map_range` 是唯一临时来源适配入口；
facade 删除时由调用方直接组合来源准备、Nest reserve 与 attach。

旧的 `map_frames`、`map_lazy`、`map_borrowed`、`attach_static`、
`map_boot_frames`、`project` 仍有“先取得无 Drop 的 Reservation，
随后来源或元数据准备以 `?` 失败”的占用责任缺口；PR 4 必须在迁移调用者时统一解决，
不能靠给 Reservation 加隐式释放或新通用 owner 掩盖。
本轮 `allocate_lazy_core` 已改为先建立来源再占位。
创建资格仍由 pend 的真实 View 身份维护，旧 wire 编码没有改变。
以上各条已由下文 PR 4 节收口（形状桥等价测试、六处占用交还、facade 与 `Mapping` 删除）；
本节其余描述是 PR 3 检查点记录。

最终命名与行为 spot 均通过；行为 reviewer 独立复跑健康轮、差异与格式检查，
得到相同的 50 + 哨兵 1、差异干净和 146 个格式块。
封存 SHA256 前缀：`pole/mod.rs 2ecb4e32605f`、
`space/memory.rs 40df188cdeed`、`health/pole.rs 7e46c3c1aad9`、
`health/syscall.rs 5f8377b36923`。

## PR 4：删除迁移层，验证组合闭合

PR 4 基线为 `b6bba234cf87d50bc7378fb561730a8eb065fbce`（PR 1 / 2 / 3 已提交推送）。
本节描述当前实现；PR 1–3 各节的 `Memory` / `Mapping` / keeper 描述是各自检查点记录。

### 功能、状态与失败责任归位

| 层 | 位置 | 独立效果 | 状态权威 | 失败责任 |
|---|---|---|---|---|
| 物理来源 | `memory/flat/` | 逻辑页来源与物化 | 页槽与物化结果 | 来源构造失败不公布来源；物化失败保留已成功槽 |
| CPU 翻译 | `memory/nest/` | VA 占用、PTE、View、同步退休 | 占用表 generation / PTE / View 活动库存 | 未发布失败退回占用，已发布失败进清理链 |
| 团队地址空间 | `work/unit/space/space.rs` | 持 `Arc<Nest>` + `Arc<Life>` | `Nest` 与 `Life` | 随 Space 生命周期 |
| 空间构造 | `space/builder.rs` | 建 `Nest`、复制跳板 | — | 构造失败不公布 Space |
| 子系统组合 | `task/builder.rs`、`capsule.rs`、`entry/boot/*`、`pole/*`、`entry/call/memory.rs` | 来源准备 → 选址 → 挂载 | 各自局部 | 先完成全部可失败准备再 `reserve`；确需先占位的失败显式交还本次仍 live 的占用 |

`Space` 的最终表面是 `{ nest: Arc<Nest>, life }` 加 `kind` / `life` 两个稳定属性，
以及 5 个用户访问组合：`user_range` / `copy_in` / `copy_out` / `validate_access` /
`instruction_byte`。它们都是“用户可达范围 + 本 Nest 真实翻译”的组合，不是裸转发；
`Space` 上没有任何一对一转发到 `Nest` 的方法，也没有字节区间到映射的动词。

### 删、迁、留（按职责）

- 删：`core/work/unit/space/memory.rs`（整个 `Memory` facade）与 `space/mapping.rs`（单 Arc 空壳）。
  形状 / 派生桥 `view` / `view_record` / `derive_into` / `project` / `whole_or_range` /
  `selection` 与临时适配 `attach_range` / `attach_range_shaped` / `map_range` 一并退出；
  生产调用点直接组合来源准备、`Nest::reserve` 与 `Nest::attach`。
- 留真权威：`kind` / `asid` / `satp` / `root_ppn` / `physical_at` / `audit` /
  `resolve_fault` / `advance_region` / `split_reservation` / `release_reservation` /
  `reserve` / `set_access` / `detach` / `dispose` / `views_in` 全部直呼 `Space::nest`；
  借出式 `unmap` 落成 `Nest::detach(view.whole())`，消费式 `dispose` 落成 `Nest::dispose(view)`，两者语义不混。
- 留 entry 边界：`basic_bytes` / `exact_bytes` / `fixed_range` / `place` /
  `unmap_bytes`（旧 VA + 字节口径多关系撤销）/ `protect`（旧 `Mprotect` 口径）与
  `From<NestError> for MemoryFail` / `From<FlatError> for MemoryFail` 都在
  `entry/call/memory.rs`；`view_at`（按 VA + 跨度取候选，最终以 `Arc<View>` 身份校验）在
  `entry/call/pie.rs`。
- 留调用者：`seed_trampoline` 在 `space/builder.rs`；启动静态挂载的
  `attach_exact` / `map_borrowed` 在 `entry/boot/mod.rs`，调用者直接 `attach_shaped`，
  不再保留 `attach_held` 转发；`reserve_at` 在 `capsule.rs`；
  `narrow_view` 在 `pole/mod.rs`（`open` 复用与 Pole `narrow` 共用一份）。
- `TaskMemory` 持 `Option<Arc<View>>` 与 `Option<Reservation>`；`HeldMap` 与 `PoleMeta` 持真实
  `Arc<View>`；`Team::cancel_staging` 按 `item.view.whole()` 的真实身份撤下，不再从 View 反推 VA 再扫其他 View。

### 六处“先占位、后失败丢 VA”的迁后形态

统一规则：能失败的准备（来源、帧表、元数据）全部移到 `reserve` 之前；
`reserve` 与 `attach` 之间不再有可失败步骤；占用来是调用方交进真在手上时，
本步自己的失败显式 `release_reservation` 那一笔仍 live 的占用。
不给 `Reservation` 加 `Drop`，也不新增 owner / guard / PreparedView 类型。

| 旧函数 | 迁后位置 | 新形态 |
|---|---|---|
| `map_frames` | `capsule.rs`、`task/builder.rs`、`entry/boot/{mod,stack,unit}.rs` | `Flat::from_frames` + `try_reserve_exact` 全部前移到 `reserve` 之前 |
| `map_lazy` | `capsule.rs` bss 段 | `Flat::acquire` 前移 |
| `map_borrowed` | `capsule.rs` 只读段、`entry/boot/mod.rs` 镜像 / 账本视图 | `BorrowedRegion::new` + `Flat::from_region` 前移 |
| `attach_static` | `entry/boot/unit.rs` 低 identity 段 | 五段的借用来源与两处活动形状在整段 identity 登记之前备齐 |
| `map_boot_frames` | `entry/boot/unit.rs` hart frame、`entry/boot/stack.rs` trap stack | 先 `Flat::from_frames`，再 `attach_exact`（活动形状也先于登记） |
| `project`（只有测试调用） | `kernel/tests/health/support.rs::derive` | 测试侧显式组合：源活动片段先在来源关系上物化发布，再按请求窗口与权限 `attach_shaped` |

其中 Task 栈与 trap frame 原本在 `split_reservation` 之后才能知道 body 起点，
本轮把帧表与来源整体前移到 `allocate_high` / `split_reservation` 之前，
所以两条 `release_reservation(body)` 回滚分支随之消失。
占位模式确实存在的地方只有三处，全部显式交还：
`support::attach_frames` / `support::map_lazy`（旧 `map_frames` / `map_lazy` 的调用形状：
占用已经交进）；`entry/boot/unit.rs` 低 identity（必须先占位再抬高窗口下界，
`advance_region` 失败显式交还）；同一循环内 `split_reservation` / 挂载失败时
交还**尚未附着**的 tail（已交给 Nest 的 piece 按 Nest 责任，不双放）。
`space/builder.rs::seed_trampoline` 与 `entry/boot/mod.rs::attach_exact`
把活动形状与来源都排在登记占用之前。

### 本轮改到的核心合同

本轮修复两类失败责任错误：
（a）`Reservation` 没有 `Drop`，而安装路径的提前失败（属性位、`try_reserve`、
`View::new` 构造校验、`Arc` 分配）在收下凭证之后发生，失败时占用永不再回；
（b）`commit` 里发布记录容量这一批准备落在 PTE 写入与 `pending_views.push` 之后，
它失败时 PTE 已成型，而 `settle_failed_install` 按索引里有没有判“未发布”
去交还仍带 PTE 的跨度。受影响调用者就是本轮迁后的这一批：`capsule` 三段、
Task 栈与外壳页、启动静态布局、`PoleMeta::open`、entry 的
`Allocate` / `Mmap` / `Mprotect`，以及健康面夹具。

1. `nest/mod.rs::commit`：`view.state.published` 的容量预留移到**任何 PTE 写入、
   `pending_views.push` 与索引接入之前**（与 `publish` / `withdraw` 同一种顺序），
   并新增精确 debug hook `fail_publish_prepare`（只在这一个准备块内生效）。
   准备之后到 `index.insert` 之间再也没有可失败准备。
2. `nest/view/mod.rs::View::new`：构造失败即按占用身份退回，并直接返回 `Arc<Self>`，
   把 `Arc` 分配失败也纳入同一条责任。
3. `nest/mod.rs::attach` / `attach_shaped`：入口 `check_owner`（不是本 Nest 的占用 ⇒
   `NoRegion`），且输入已按值消费，失败一律还给它的真属主：
   `give_back_unpublished` 是“归还方向 = 属主”的那一个原语，
   `abandon` = “按属主交还 + 交出错误”（与调用方主动退的 `release_reservation` 成对），
   `settle_failed_install` 的未发布分支与 `View::new` 的失败路径都走同一方向；
   已发布失败仍进清理链。`give_back` / `give_back_span` 只是“退还”原语。
4. `nest/view/range.rs`：只被已删派生桥调用的 `active_fragments` 随桥退出；
   `ViewRange::source()` **保留**（核心的 `Nest::materialize_active` 仍在用）。
5. 两处纯查询可见性放宽，无语义改动：`ActiveRange::intersection` 到 `pub(crate)`
   （健康面的派生等价组合直接调真实交集算术，不另写一份 max/min），
   `Reservation::id` 到 `pub(crate)`（用例断言“同址重登记拿到新 generation”）。

生产里每一个调用者都把“自己的 Nest 建出的 `Reservation`”交给同一个 Nest，所以第 3 条
不是今日可达的故障路径，而是把已经分裂的失败责任（四个阶段里三个按 target、
一个按属主）统一成一个方向；测试用 A/B 两个 Nest 在同一 VA 上各自持合法凭证来钉住它。

### 测试保真：健康面无第二份线外 policy

`kernel/tests/health/support.rs` 不再复制线外算法：尺寸取整（`basic_bytes`）、
选址（`place`）、窗口内固定登记（`reserve_at`）、按字节改权 / 撤销（`protect` /
`unmap_bytes`）与按 VA 取候选（`view_at`）全部调 `entry::call::memory` /
`entry::call::pie` 的真实入口，夹具只做 `&Space → &Space::nest` 适配；
为此把这些 helper 与 `pub(in crate::entry) mod memory` 放宽到 `entry` 父域，
不宽到 `core/work`、不下沉 core、不新增 test-only 反依赖。
派生等价组合的片段交集也不再另写一份 max/min：直接用 `ActiveRange::intersection`，
夹具只负责选窗口与收集。只有确定仅测试的原语组合保留在夹具里
（`frames_source` / `attach_frames` / `map_frames` / `map_lazy` / `allocate` /
`allocate_lazy` / `attach_static` / `derive`）。
健康面模块也改为 `health/memory.rs`（原 `manager_api.rs`），
四个用例名为 `memory_placement` / `memory_view_identity` /
`memory_access_wire` / `memory_source_sharing`。

### 实际验证

工具链与基线相同（`rustc a69a63265` / `cargo b2e9d5f9d`）。主工作区实际执行：

| 实际检查 | 结果 |
|---|---|
| workspace dev/release、kernel all-targets dev/release、semihosting dev/release | 通过，无警告 |
| programs all-targets | 通过 |
| `nu scripts/qtest.nu --package kernel` | 51 例，50 passed；缺 initrd 的 `scene` 1 failed，退出 101 |
| `--scene accept` | 1 passed |
| `--scene product` | 默认 EOF 场景 1 passed |
| `--scene system-fault` | 退出 101，签名与基线相同：`loader.rs:361` Denied(6) → `:306` / `:190` 超时级联 |
| frame-runs / Pend / Trace 默认与 semihosting / console-line | 9 / 35 / 2 + 2 / 5 passed |
| env 宿主（显式宿主 target） | 13 passed；既有 unused Wire 警告仍在 |
| resource capability（独立 manifest） | 16 passed |
| alloc-probe 库 / 无默认特性宿主测试 / all-targets 检查 | 6 / 6 + 4 / 通过 |
| `git diff --check` | 通过 |
| `cargo fmt --all -- --check` | 退出 1，60 个 Diff 块；只格式化本轮触碰的文件，未全树重排 |

健康轮与 accept 是实际结果；`system-fault` 仍是真实失败，未改超时门槛、未删断言。

本轮新增的失败注入证据都在 `nest_invalidation_failure` 这一例内（子函数形式，保持 51 例不变）：

- 轴 1（尚未发布）：`fail_publish_prepare` 让发布记录容量准备失败 ⇒ `OutOfMemory`，
  `probe` / `translate` 均空、`pending_views` 空、关系数与表节点数不变、无待办失效；
  同址 `reserve` **成功**（未被“拒复用”），且输入 `FlatRange` 仍持原 Frame 与内容，
  同一段来源重新 `attach` 后拿到同一 PA 与同一 `Arc<Frame>`。
- 轴 2（已发布）：保留原有 shootdown 注入 ⇒ 旧 VA 拒复用直到 `retry_invalidation`，
  来源 / Frame / 清理链仍持有（旧断言未放宽）。
- 属主方向：A / B 两个 Nest 在同一 VA 上各自持合法凭证，把 B 的凭证交给 A ⇒
  `NoRegion`；A 自己的占用不受影响，B 同址可再 `reserve` 且拿到**新 generation**。
  非架构属性位（自己的凭证）与构造跨度不等（`View::new` 拒绝）也都退回自己的表。

F3 的几处“先占位后失败”路径（`advance_region` / `split_reservation` / loop 中段挂载失败
与未附着 tail 的交还）是代码里显式写的，但本轮**没有**为 boot 加注入 hook；
它们由已经单独测过的原语（`nest.rs` 的切分 / 登记、`nest_failure.rs` 的安装失败）
真实 accept 覆盖正常启动；不声称这些 boot 失败分支已注入验证。

### 保留的判据与残留

迁到直接组合的等价测试保留了原断言：保洞（被撤页不被派生重发布）、
源活动片段先物化再回源关系发布的旧副作用、两边 Frame 身份（`Arc::ptr_eq`）、
局部 `current` 权限与拷贝拒、派生不继承源页 current、来源在源树关闭后仍持帧、
每个用例与整轮的帧 / 表净零。`task::cancellation` 仍实际走 `cancel_staging`：
新写法比旧 `unmap_range` 更强（活动已撤但票未结算的一段也会按同身份继续退休），
旧写法“该范围没有任何活动关系就报 `NoRegion`”那条现在不可达。

仍未做：多 hart 并发物化 / 撤销、分配器元数据 OOM 的强制注入、
`system-fault` 的 Loader 首次构造失败根因；新 ABI / TLS 包装、来源授权身份（`FlatReference` / `ViewId`）、
System 独立保留、IOMMU / DMAC 均属后续阶段，本轮未动。
健康面的 `health/memory.rs`（原 `manager_api.rs`）与四个用例名（`memory_placement` /
`memory_view_identity` / `memory_access_wire` / `memory_source_sharing`）已同步改名，
不再指向已删的 manager 模块；`Mapping` 已在 PR 4 随 `space/mapping.rs` 删除，
借出句柄直接就是 `Arc<View>`。

### 集成后复核

隔离工作区的复原不能代替合回后的检查。本轮合回曾使非目标
`entry/call/resources.rs` 内容错置、`entry/call/tole.rs` 带冲突标记，
并多改 `health/spare.rs` 的格式。父工作区只将这三个文件恢复为已提交基线，
再对它们及同期涉及的八个非目标文件逐字节核对，十一份均与 HEAD 一致。
核心、来源调用者与失败用例的封存指纹没有改变。

恢复后父工作区独立重跑上表编译矩阵、健康轮、accept、product、system-fault
以及列明的宿主套件，读数与表内一致；差异检查干净，源码无冲突标记，
格式检查仍退出 1、60 个 Diff 块。这里的结果来自集成后的实际文件，
不是把 worker 隔离树的通过结果当作主工作区证据。

最终命名 / 层次与行为 spot 均通过；行为 reviewer 另行独立编译通过，
按实现核对了容量准备顺序、两轴失败证据、属主回收、启动尾段和测试保真。
SHA256 前缀：`nest/mod.rs 0fac4616cd88`、`nest/view/mod.rs eba3e61f4140`、
`entry/call/memory.rs fd56c0dbd30d`、`health/support.rs 52edc4ac0def`、
`health/nest_failure.rs af8b2c1efe57`、`health/memory.rs 8f6586036621`。
记录级限制仍包括：boot 常量经预校验后的 Exact 构造没有专项失败注入；
内存用例中只对合法输入自算尺寸的期望值不代替 wire 溢出验证。
