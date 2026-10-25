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

## PR 5：迁移 ABI、来源授权与 Rust 包装（实现、验证与审查）

本节是**未提交的迁移检查点**：新五操作及生产包装已经有实现，
旧五变体、dispatch、VA 字节适配与 `CreationRegistry` 已删除。
健康面和宿主 Pend 的等价测试已迁移，固定版本已通过独立功能/生命周期与命名/层次审查；
验证与审查均不替代远端提交核对。

### 用户新指令（覆盖原兼容条款）

**不保留旧 `MemoryCall`**：`Allocate` / `Deallocate` / `Mmap` / `Munmap` / `Mprotect`
五个变体与其 dispatch 分支、旧 VA 来源桥、旧创建资格桥全部最终退出；
所有生产包装 / heap / TLS / loader / resource / 测试迁到新五操作。
`MemoryCall` 只承载 `Acquire` / `Release` / `Attach` / `Detach` / `SetAccess`，
仍用**显式槽 5–9**；旧 0–4 明确解码为 Unknown / 拒绝，**不重排、不 alias**。
因此没有“旧字节规范化”生产 helper，历史 ABI 固定用例改成“旧槽拒、新槽往返”。

### 已落地的线协议（现行）

- 5 `Acquire { source: AcquireSource, retention: PieToken } -> FlatReference`；
  6 `Release { reference: FlatReference } -> ()`；
  7 `Attach { range: VirtAddr, team, placement: VirtAddr, access, control: ViewControl } -> (ViewId, VirtAddr)`；
  8 `Detach { team, view: ViewId, selection: ViewSelection } -> ()`；
  9 `SetAccess { team, view, selection, access } -> ()`。
- `Attach` 五字：`range` 与 `placement` 分别是**指向定长描述的只读用户指针**，
  各从**调用者**的地址空间读一次快照；`control` 单独一格。不再把控制塞进 Placement，
  也不动 A0–A5 宽度。
- `ViewControl::{Caller, SpaceMembers}`：逐映射的显式选择。`SpaceMembers` 只允许
  **来源与目标域授权都已通过**的 `Attach` 发起者选择，只绑定这一条目标 Space 控制记录；
  它不是“同 Space 自动全控”，不给源所有权/资格/独立 Hold 任何授权，
  也不给其它 View 加权限。kernel 不代选（`team != 0` 不自动变 `SpaceMembers`）。
- `ReferenceRange`（`WORDS = 3`）与 `Placement`（`WORDS = 3`）各有自己的定长 codec；
  前者用 [`Wire`] 的同一份布局（`encode` / `decode` 都走 `pack` / `unpack`，不重写第二份）。
  两者实现 `Field`，`WIDTH = 24`，字节口径固定为 LE；内核按这份宽度快照，不另写长度。
- 类型：`AcquireSource`、`Allocation`、`Placement`、`PlacementAt`、`ReferenceRange`、
  `ViewControl`、`ViewSelection`（一型一文件），`FlatReference` / `ViewId` 在 wire 中立域各自成文件，
  `MemoryFail` 独立 `memory/error.rs`。
- 宿主证据：`cargo test -p env --target x86_64-unknown-linux-gnu --offline` 23 过
  （旧槽 0–4 对零载荷、全高位及旧 Mmap 载荷均为 `BadSlot`，新槽 5–9 逐一往返、
  `ViewControl` 两支与非法值、`Placement` 往返与
  零/非 2 的幂/高位对齐构造不 panic、范围 checked、固定字节布局与 signed 身份域）。

### 已落地的内核功能（真实入口，已跑）

- `SourceAuthorization` 已接入编译模块（`pend/allocation/source_authorization.rs`）：
  一份来源授权 = 范围 + 上限 + 一次性创建资格 + 私有/根 Grant 相位；
  它与引用表、创建资格、事务同处工作层。
- per-Task **引用表**（`hold_table.rs`）：句柄号**全局单调**、永不复用、
  超符号上界运行时明确失败；表自己按 id 找——跨 Task 的同数字不成立。
- `Acquire` / `Release` 已实现（`entry/call/memory.rs`）：
  新 `Allocation` 建来源持有 + 一份未消费资格；`Reference` 子范围 clone **共享**同一份
  授权记录（同一来源/上限/撤销依赖），不复制资格；`Release` 只摘本表那一格，
  不 Detach、不分配，授权失效也不妨碍清自己的句柄。
- `retention != NONE` 的独立保留授权只认**内核铸的** Nole 真对象
  （`NoleMeta::mint_retention()`；`try_new` 造的是可自造的普通 Nole，不算），
  还要求调用者持有它自己那枚活的 Pie。
- Task 尽头：`cull::doom` 清空本任务引用表（引用结清、未消费资格结束）；
  关系不由这张表拆，留给来源 Grant / Pole / Nest 的清理链。
- `ViewRegistry` 有发布前占位的 `claim`、不分配的 `commit` 与 `cancel`；
  `Attach` / `Detach` / `SetAccess` 已从真实 trap 路径执行过。
- Pole 的创建入口改收完整来源引用，不再从 VA 导出创建资格；
  `resource::raw::pole` 已使用 Acquire → Unseal → Release。

### 交接与退出清理

- foreign 安装已接回 VEST、readonly、ONLY、exclusive、整窗、无活别名与 staging 判据。
  ONLY 的成功 Spawn 原子消费发送者 Pie，移交映射 controller；scope/ceiling 不扩大。
- 交付范围限于已建立的 CPU 映射与控制权，不铸接收者的新来源能力或独立保留。
  `SourceAuthorization` 永久保留 Root/range=None，资格保持已消费，旧引用不能恢复私有归属。
- Spawn 先用短 gate 原子暂停新发布；在途未空即 Busy。publish 期间不持 gate，
  失败仅在原 Grant 仍活时恢复发布，成功永久关闭旧发布权。
- Task 的反向弱索引只定位其 Caller 控制记录，不承担地址或来源权威。
  死亡清理在表锁外消费 View；SpaceMembers 不因 issuer 退出被误清。
  退休判据为 Nest 的实际 Reservation generation；同步未结清不交还 VA。
- Rust `Reference` / `View` 是显式 release/close 的控制记录，失败保留本地句柄；
  映射归 Space，临时 Rust View 的 drop 不隐式 Detach。堆与 TLS 使用预留元数据；
  TLS 本地记录在 native 终态后回收，不把私有控制权交给另一个 Task。
- 原三条宿主资格判据已迁为来源身份绑定、失败回退、已消费不可复原，
  另覆盖独立保留不延长资格及身份耗尽不回绕；Pend 宿主共 37 passed。

### 实际验证与限制

| 检查 | 结果 |
|---|---|
| workspace build/check dev + release | 通过；dev 无告警，release 的 kernel 有 7 项 unused 诊断/保留入口告警 |
| kernel all-targets dev/release，默认与 semihosting | 通过 |
| programs all-targets dev/release | 通过 |
| `nu scripts/qtest.nu --package kernel` | 58 passed + 缺 initrd 的 scene 哨兵 1 failed，退出 101 |
| accept / product / system-fault | 各 1 passed；去掉临时诊断与 eager 实验后分别 10.29s / 5.33s / 13.30s |
| env / Pend / frame-runs / trace / console-line | 23 / 37 / 9 / 2+2 / 5 passed |
| execution heap / unit，resource capability | 6 / 1 / 16 passed |
| alloc-probe | 默认宿主 C 编译失败；加 `CFLAGS=-std=gnu17` 后串行库 6 passed；无默认特性串行 6+4 passed；all-targets 检查通过 |
| `cargo fmt --all -- --check` | 退出 1，58 块既有格式债，未全树重排 |
| `git diff --check` | 通过 |

alloc-probe 的默认并行库测试另有 4 passed / 2 failed（共享台面的预算/守恒判据相互干扰）；
串行与 C 标准覆盖是不同检查，不把默认命令失败写成通过。未改动该探针或 vendor C。
workspace 的裸机全树 `--all-targets` 也不能代替分包矩阵：`stream` 的 std 测试不能编到
`riscv64gc-unknown-none-elf`；上表使用实际支持的 workspace 与分包构建方式。

本轮真实入口覆盖：五个 Pole 失败检查点；四个准备失败（不为无分配 Release 伪造 OOM）；
双子 Grant 撤一留一及真实 cull 遇在途 Busy；普通/独立保留的 Frame 生命周期差异；
live View 在场时最后普通引用释放后资格永久结束；同 Space 控制不授来源权；
Caller 退出与共享映射存活。仍不声称已验证真实 IOMMU/DMAC、硬件隔离、多 hart 所有交错。

### 并发堆暴露的 ASID 返回遗漏

`system-fault` 入口先执行真实 Talc 并发堆验收。迁移到惰性来源后，成功缺页路径的提前
return 跳过了 ASID 租约恢复：hart 已回用户态，记录却仍为 kernel，后续 shootdown 可能漏掉它，
导致不同分配之间的数据污染。trap 返回统一经 `resume` 恢复目标 ASID；并发堆及完整
system-fault 场景随后通过。预触碰/eager 与 PTE fence 的诊断实验均未保留，期限和断言未放宽。
另外修正两处手工启动 closure 的真实类型为 trampoline 所要求的 `FnOnce()`，不再错转
`FnOnce(usize)` 的 vtable。

### 来源授权的内部口径

- **真实范围归谁**：授权记录只在**私有阶段**强持整段来源（`range: Option<FlatRange>`）；
  交给根 Grant 时 `adopt` 把范围**转出**，此后真实范围在 Pole 手里。
- **普通引用不永久 pin 来源**：`Hold` 只记相对授权的页坐标，用得着时向授权解析
  （`Hold::resolve`）：私有阶段解自己的，已接管就向 Pole 要并要求那枚 Grant 仍活。
  撤销后普通句柄既不能挂载，也不再强持物理源；**只有带独立保留的那一份**
  在取得时把这段范围解下来自己强持，因而能跨 Driver 撤销继续持同一批帧。
- **资格结束是显式语义**：授权记录上有一个普通引用的成员计数（不是 `Arc` 强计数），
  最后一份普通引用解除时**显式消费**那份未消费的一次性资格。接上 View 登记后
  仍有人持着授权记录，资格该结束还是结束；独立保留不计入普通引用成员。
- 私有来源记录保存真实 owner；`SpaceMembers` 控制权不能换成来源权。
  `Acquire(View)` 必须另核私有 owner 或调用者持有的活分支 Grant，
  失效或已移除 token 返回失败，不通过 `expect` 触发内核 panic。
- `Acquire(Reference)` 的切片必须落在输入 Hold 自己的窗口内，而非整份来源内；
  `create_pole` 拒绝子窗口、Grant 分支和独立保留输入。
- **保留 cap 命名**：`NoleMeta::retains()` / `mint_retention()`（不再用 `detached`，
  那是 `Detach` 的映射语义）；铸造厂是内核的真入口，把 cap 实际发给
  System / IOMMU 属 PR6。
- `Phase` 单独成文件（一型一文件）。
- 上述新增边界已有实现与 release 编译证据，仍待迁完健康面后的真实入口反例验收。

### 运行时元数据与宿主证据

- Heap 显式选择 `SpaceMembers`，TLS 使用 `Caller`；身份登记使用静态预留槽，
  表满在 Acquire / Attach 前失败，不递归调用同一 GlobalAlloc。
- 映射责任分为在用、欠清理、在途关闭、已结清；在途槽不能被复用。
  引用的待释放责任另记 owner，只由该 Task 重试；两笔责任都结清后才腾槽。
- 扫描只主张仍为 Owed 的项，按地址归还的查找与主张同锁完成；
  旧扫描不能撤下已复用的新 Live 映射，失败回写也不恢复已释放的引用。
- `cargo test --manifest-path crates/execution/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline`
  最近实跑 **6 passed**：原 pooling/alignment/realloc/failure/cross-task-free 断言保留，
  新增表满/在途、Detach 失败重试与引用 owner、Live-only pending、陈旧扫描与回写交错。
  夹具使用真实 System 分配区，并按宿主线程区分引用表 owner，不释放伪造地址。
- 这些证明的是运行时状态协议，不代替真实内核授权、TLS 退出和多 hart 撤销验收。
- 旧五操作删除前的父工作树 `accept` 已实跑 1 passed（10.39s）。
  删除后健康面尚未编译通过，不能把那次场景结果当最终树验收。

### 下一步必须钉住的事实（已有合同的检查点）

- `AcquireSource::View` / `Acquire(Source{Pole})`：来自子 Pie 的引用必须保**该子
  Grant/token 的撤销边与上限**，登记也登在该子 token 下——不能因为同 Flat、
  共享根资格就统一用 `SourceAuthorization` 里的根 token。否则子 Pie revoke 扫子 token
  时找不到新 View，成逃逸。根资格 / 私有→根的转移事实可以共享；不同 Grant 分支的
  授权不是同一事实，不可偷合。需要正反测试：同源下根与两个子 Grant，撤一支只关该支。
- 接 View 后的资格测试：仍有 live View 时最后一份普通引用释放后，
  重 Acquire / View 数据仍可用，但资格不得复活。

### 发布与关闭的线性化点（不照抄“PTE 后再读一次 alive”）
反例：`Attach` 写 PTE → 读 Grant 仍活 → `Revoke` 关闭 Grant 并扫描已登记关系
（本 View 还没登记）并成功返回 → `Attach` 再插入登记 ⇒ 撤销后逃逸一个 View。

因此：

1. `Attach` 在**写任何 PTE 之前**，在目标 Space 的控制记录里插入一条 *in-flight* 条目
  （携带本次占用的 generation、来源授权与精确 ViewId），容量先备齐，使这一步之后不再失败。
2. `enter_publish(id, branch, access)` 在短 gate 下验证当下分支权限并登记在途；
   `cull` 在移出 Pie / invalidate Grant 之前经 `begin_withdraw` 取同一 gate。
   在途未空则返回 Busy；不能先报告撤销完成，再漏掉迟来的发布。
3. gate 不跨 `Nest::attach`、detach 或远端同步。映射在锁外建立，提交后才解除在途；
   失败先保留/交还对应的占用与清理责任，再结清。
4. 分支 record 容量由已有 `PoleOperation` 租约串行；Space 的注册槽实际占位，
   不靠单独 `Vec::try_reserve` 假装并发提交不会分配。
5. 这些是当前实现规则，真实 Revoke / Seal / 在途 Attach 的交错和创建交接仍需整体验收，
   手动调用 gate 的健康例不替代真实撤销链测试。

### 独立审查记录

- namer 与 reviewer 均逐一核对 11 个生产 pin，基于集成后的真实未提交文件实审，
  不拿旧隔离树或 worker 的测试读数代替检查；两者结论均无实现阻塞项。
- 嵌套 `execution/tests/unit/target/` 已补 `/target/` 忽略，提交仅暂存明确的源码范围，
  不以 `git add -A` 收入产物。
- ONLY 的 A′ 语义已明确：成功永不恢复旧发布权；发送者 root token 已摘除并失效，
  其后真实 Acquire 返回 Denied。交付页继续可用，controller 属于接收者，
  但不授予来源再派生或创建资格。健康面另断言 Root/range=None、不恢复发布与资格。
- 坐标查询不按根 permission 统一过滤，操作点单独核分支授权；
  否则会误伤同源下独立的子 Grant。
- 非阻塞跟踪：描述符失败词汇细分、引用成员计数失衡防护、失效 Weak 压缩、
  旧 Pie/Mail 其它编号源的界限统一、少量诊断查询及保留 cap 工厂在 release 未使用。
  不用饱和减法掩盖计数违例，不将清理错误改成静默 `let _ = release`。
- `fresh caller reference cleanup` 的 expect 是对“刚取得、同 Task、无分配 Release”
  不变量的显式检查，不是错误重试；正常失败路径会清理这份引用，
  意外违反不变量时用户任务 fail-fast，由内核 Task 退出清理，不能表述成内核 panic。

### 本轮一并带上的基线修复

`programs/src/harness/probe/loader.rs::image_bytes` 的入口与 `PT_LOAD` VA
从 `0x4000` 改成与 `crates/execution/link.ld` 一致的 `IMAGE_BASE = 0x1_0000`
（低于 `IMAGE_BASE` 的真实失败是 `MemoryFail::NoRegion`，不是 IPC 超时；
修 fixture 后 system-fault 真实场景通过）。本项只改该夹具的三行事实。

## PR 6：System 独立保留与故障隔离路径（实现、验证与限制）

基线 `master = 75ef237326461a0a29dce1191240de82b623057b`（PR5 已推 `origin/master`）。
本轮不动旧 ABI：`MemoryCall` 仍是槽 5–9，`Acquire{source, retention}` 的 `retention`
就是 PR5 那枚**内核铸的**独立保留授权。

### 冻结合同

- System 在自己那张 pend 表里持**经过独立授权**的 `FlatReference`；原服务 / Driver /
  Pole / Pie 怎么销都不撤这一份。**没有**另建 Detached Registry、没有新 RPC、
  没有第二份设备状态。
- 保留必须在**硬件可达地址/配置发布之前**登记；记录设备范围、控制者、隔离权限、
  `FlatReference`、登记状态；不存旧 IOVA / 传输进度，不进 `Flat`、不进 UnitFile。
- 正常关闭：可信 IOMMU 同步确认后结清；故障：System 自己隔离后整批结清。
  登记 / 发布 / 关闭在同一段范围上串行，关闭一开始就禁新登记与新发布。
- 暂时失败**保留记录并重试清理**，不恢复旧业务；隔离能力不以那一头还活着为前提
  （不向已死的 DMAC 发停止请求）。
- 设备服务自然退出**先离线**；独立持有不随普通 supply / identity 清理提前结清；
  无关 Task 的资源不全冻。

### 内核侧（交付那一条路）

- `env::ledger::name::Call` 增一格 `Retain`（class 1 / item 2）：授权
  `MemoryCall::Acquire` 的独立保留形式，不新增调用号。
- `NoleMeta::mint_retention` 改为可失败（不再在引导期 `expect`）。
- 新增 `entry::boot::resources::retention::grant(target)`：在**引导装配那一处**为收件任务
  铸这一枚真对象（owner 就是它），并作为账目行交给账本。交付是**推**的——没有请求面，
  于是没有“第一个来问的就给它”这条路；`retention::ENTRIES = 1` 明写进账本尺寸。
  任务号在 `hold()` 后才存在，因此这一项在普通 `Frozen::grant` 后铸造；
  `ledger::write` 对全部账目统一检查 `Entry::valid()`。
- PR6 当时 `require_retention` 的判据：内核铸的 `retains()` 对象 + 活的 Pie + `VEST` +
  **owner 就是调用者**。普通自造 Nole、`Mark`、任务数字、只有 `FETCH/STORE/VEST`
  都造不出授权；同一枚 cap 转给别人之后，别人也过不了 owner 那一关。
  这是历史检查点；当前已批准的委托合同改为核调用者表内活 Pie、VEST 与内核铸保留类，
  不以历史 owner 限定唯一使用者。源范围/源权限、资源归属及 Seal 规则不随之放宽。

### 持有账（`programs/src/driver/iommu/hold/`）

它原先住在 System（`system/supply/retention/`），现随迁移住 **IOMMU 驱动自己的功能域**：
只有那一台驱动持有它（生产调用方只此一台，故不升成公共库 API）。

`Holds` 唯一拥有设备范围及其引用，不是独立的 `Resources` 项。
`Scope` 保存真实设备隔离范围的 `DeviceRange`（基址、长度）、使用者 Task、
驱动自己装的平台控制对象（`IommuPlatform`，只持本域 VA）、阶段与该范围全部 `FlatReference`。
不存旧 IOVA / 传输进度。

| 操作 | 单一效果 |
|---|---|
| `attach` | 绑定范围、控制者、独立平台控制对象；拒绝范围重叠与 provider 范围不符 |
| `register` | 先准备元数据，再以真实保留 cap 执行 Acquire；成功后无后续可失败登记 |
| `publish` | 只能发布已登记引用；执行回调前先保守标记可能可达，部分失败进入故障关闭 |
| `offline` | 立即关闭该控制者所有范围的发布闸；**不宣告设备已停** |
| `close(Normal)` | 关闸，要求可信平台完成正常同步、失效及在途访问排空 |
| `close(Fault)` | 关闸，要求驱动自己的平台入口完成整个范围的隔离及停止确认 |
| `retry` | 平台确认后才 Release；释放失败只重试释放，不重复隔离、不重开业务 |

`Platform` 的两个停止入口都是有界进展步骤。`Pending`、隔离超时或其他错误
原样保留引用；一个范围失败不冻结其他范围。缺少 provider 时禁止发布；
未发布的登记在关闭后可直接结清。已经关闭的范围即使结清也不允许自动重新 attach。

不存在由 `Phase` 自算外部可达性的接口。CPU Pie 撤销、TaskGone、`VEST` 位、
仅写软件状态都不构成设备停止证明。平台实现属于可信组成，不接受设备服务
提交一个布尔值或伪造确认。本阶段只提供实际有状态模拟实现，真实寄存器实现属 PR7。

### 接进真实产品链的那几手（迁移后）

- System 只做装配：从引导账取保留 cap 与三段窗，**逐枚 Accord 交进驱动自己的表**
  （保留要带 `VEST`——`require_retention` 的判据），再把收据推进孔里、放开 held 的驱动。
  驱动自己用那份 cap 在硬件范围内登记任意经过来源授权的页窗口；数据、描述符、DDT、
  页表、三类队列共用这一条规则。
- 帧持有、IOMMU 编程、端点排空与停止确认全在驱动自己的 Task/Space 里；System 不再有
  `retention.close` / `retention.sweep` / `retention.reclaim` 三步，也不读寄存器。
- 使用者（端点服务）死亡：驱动是它的 sire，自己看见并走 `close(Fault)`；
  正常收场由 System 发关闭请求、驱动确认后回终端结果。
- 关闭计划：System 先 `device.close`（发关闭请求、等成功回执、join/回收驱动），
  之后才撤镜像/供给/命名空间或走通用任务终止；没收到成功回执就不报已停。

### 实际验证

以下是父线程返工后的实跑结果；旧候选测试结果不作验收证据：

| 检查 | 结果 |
|---|---|
| workspace dev/release、kernel all-targets semihosting release、programs all-targets release | 通过；programs 最新 dev 10 项未接入 PR7 硬件建立处的 dead-code 诊断，kernel release 6 项既有诊断，不称无告警 |
| `cargo test -p env --target x86_64-unknown-linux-gnu --offline` | 23 passed；既有 signals 未用 Wire import 告警 |
| 新 retention 宿主套件 | 6 passed |
| `nu scripts/qtest.nu --package kernel` | **59 passed** + 缺 initrd 的 `scene` 哨兵 1 failed，退出 101 |
| `--scene accept` / `product` / `system-fault` | 签名归并后复跑各 1 passed，10.28s / 5.33s / 13.40s |
| programs/tests 全部 31 套 | 265 passed；初跑 system-shape 17/18，修正签名后 18/18；未删或放宽结构断言 |
| `cargo fmt --all -- --check` | 退出 1，58 块既有格式债；只格式化本次相关文件 |
| `git diff --check` | 通过 |

结构门会计入 receiver：`attach` 接唯一实际 `Scope`、`register` 接三字段
`Registration` 载荷，`publish` 只接全局单调 id，去掉重复传递的设备范围。
没有用放宽“最多三参数”断言来制造通过。

### 宿主与原生的独立访问证据

宿主直接导入生产 `Holds`，外部访问者持有独立的 active / pending / timeout 状态，
只持 RAM 的 Weak，不从保留账的 Phase 推断可达性。验证源强持丢失后继续真实写 RAM、
超时与在途访问阻止回收、正常同步与故障隔离、Release 失败只重试释放、
部分发布失败、关闭禁登记/发布、缺 provider 不执行发布、不同范围分别进展。

新增原生 `external_retention_orders_real_frames_and_isolation`：
用同一份生产 `Holds`、真实内核调用和引导铸造 cap 登记 7 个独立页窗口。
来源 Task doom 与普通 pend 清理先完成，CPU 映射已不存在，死亡通知尚未交付，
独立访问者仍经相同 Frame 物理地址逐页写入并读回。
丢失/重复通知不释放，隔离超时仍可写且记录保留；
平台关闭访问后才结清，全部 Frame Weak 失效，验证没有其他 CPU View 偷偷 pin 住页。

### 内核侧的反向与保留证据

- `resource_registration_and_revocation`：
  PR6 验证引导 cap 仅对铸给它的表授权，同一枚 cap `accord` 给别人之后别人 `Denied`；
  本用例取过的每一份引用逐格结清。该委托拒绝预期由当前获批合同替代：
  合法子授权应成功，非本表 token、伪造对象、缺 VEST 及撤销仍拒绝新增保留。
- `memory_branch_revocation_and_retention`：来源那一枚被摘掉之后，**独立保留仍持同一批帧**；
  对已经不在表里的来源再做一次 release 报 `Denied`（重复/迟到的通知不动保留）。
- `memory_retention`：普通自造 Nole 不授权；内核铸的那一枚才授权，且保留自己强持范围。

### 限制与未做（不宣称已验证的东西）

- **真实 IOMMU 寄存器已接**（见本文件末「PR 7 第一段」与「真实端点」两节）：寄存器、
  DDT/DC/Sv39 表、队列、`IOFENCE` 完成记录与调试口都是真读数；**端点真 DMA 也已接**
  （`iommu-dma` 景）。仍未接的是**产品那一侧**：`programs/src/system/` 没有安装真实
  `Platform`、也没有独立 DMAC 服务，故「服务死亡 → System 独立隔离」在产品组合里仍没接上。
- PR7 必须安装独立平台控制者，把真实数据/描述符/表/队列逐段登记后才发布。
  IOMMU Off + busy 的数据隔离与 CQ/FQ/PQ 各自 on/busy 的控制页停用分别确认；
  仅清 CPU PTE、仅屏蔽 IRQ、仅写 Offline、切 Bare 都不能满足 `Platform` 合同。
- PR6 原合同按收件任务的 owner 判据限制保留 cap，System 当时不能转授保留权；
  当前已批准允许沿既有 Accord/Grant 链委托，仅实际持有合法活授权者可新取保留。
  本节保留历史验收事实；本轮实现与运行结果记入 `system-next-refactor-plan.md`。
- 多 hart 交错、真实撤销与在途发布的整机竞态仍不在本轮证据里。

### 同组独立审查与范围保留

原 worker 两次连接服务失败后封存，父线程接管返工；同组 namer 与 reviewer
均实读父 checkout，机制、所有权与停止确认规则通过。reviewer 独立复跑
宿主 6/6、原生 59 + 哨兵 1，发现并复现结构门超参失败；随后修正接口而非断言。

reviewer 的范围异议仍保留：生产组合目前不登记设备工作范围、不安装 `Platform`，
故不能把本阶段独立提交描述为产品保留/隔离已闭环，也不能描述为无保留批准。
本阶段是经过真实内核对象验证、已接入生命周期的机制；真实产品接线及
data/descriptor/DDT/页表/三类队列的硬件闭环必须在 PR7 完成。
在真实接线审查通过前不把这个机制检查点单独推送为 PR6 完成。

## PR 7 第一段：真实 RISC-V IOMMU 的寄存器与隔离确认（部分完成）

基线 `HEAD = 73af1c13d1bfeaffa787f29cb0952ee77e3728bc`。这一段的实际状态：
**用户态**在真实硬件上把 DDT / DC / Sv39 表 / 队列建起来、编程、让硬件翻一次、
再拆除并确认隔离；**带窗口的分支**（精确 BAR 跨主体那一枚原语）已实现并有实测；
**端点真 DMA 与产品 `Platform` 安装尚未做**（下面「未做」一节逐条列，不以本段代替那些验收）。

### 选定的硬件与实测坐标

| 事实 | 证据 |
|---|---|
| 这台机器有一颗真实 RISC-V IOMMU：`-M virt,iommu-sys=on` | FDT `/soc/iommu@3010000` `reg = <0x0 0x3010000 0x0 0x1000>` `compatible = "riscv,iommu"`；`riscv_iommu_pci_setup_iommu` 把它装到 PCI 根总线，PCI 端点的 DMA 因此过它 |
| 它的寄存器窗是设备树里的一段 `reg` | 与其它设备同一条路登记成 `Page::Region(0x3010000)`，交给引导镜像；`programs/src/support/machine.rs::platform_window` 已把它与 PCI 宿主挡在设备账外（不发给驱动） |
| 复位后 DDTP = Off（设备访存不翻译） | 本景实读 `ddtp_reset=0x0 mode=0 busy=false`；QEMU 属性默认 `off=TRUE`（`riscv-iommu.c`），sys 变体不覆盖它 |
| 固件不挡这一段 | 固件 banner 实测 `Domain0 Region06 : 0x0-0xffffffffffffffff S/U: (R,W,X)`（S/U 可访任何物理地址）；console 走 semihosting，故裸 `-serial stdio` 看不到 banner |
| 寄存器表 | QEMU `hw/riscv/riscv-iommu-bits.h`（规范 1.0）：CAP 0x0000 / DDTP 0x0010 / CQB 0x0018 / CQH 0x0020 / CQT 0x0024 / FQB 0x0028 / FQH 0x0030 / FQT 0x0034 / CQCSR 0x0048 / FQCSR 0x004C / PQCSR 0x0050；队列 CSR `enable[0] / on[16] / busy[17]`；`DDTP.mode` Off=0 / 1LVL=2，busy[4] |
| DC 与命令的格式 | DC 是 64 字节扩展格式（`intremap` 默认开 ⇒ `dc_fmt = !enable_msi = 0`），一级 DDT 按 `devid * 64` 索引、`devid < 64`；`tc.V[0]`、`fsc = mode[63:60] | ppn[43:0]`、`iohgatp` mode 0 = Bare；无 process id（`tc.PDTV=0`）时 process id 必须是 NOPROCID(=0) |
| IOFENCE.C | `dword0 = 2 | AV[10] | DATA[63:32]`，`dword1 = 完成记录地址 >> 2`；完成记录是硬件写回内存的 4 字节存储（`riscv_iommu_iofence` 的 `dma_memory_write(target_as, addr, &data, 4)`） |
| 叶 PTE 必须带 `U` | 没有 process id 时硬件把访问当用户态：`riscv_iommu_spa_fetch` 里 `!(pte & PTE_U) && !pv` 直接判故障（实测：不带 U 时报 `RD_FAULT_S`(13)） |
| 翻译响应里的页号是 PPN | `PPN_DOWN(phy) = phy >> 12`，故 `TR_RESPONSE.PPN[53:10]` 装的是页号本身，不是页对齐地址 |

### 这一段实际做了什么（`iommu-probe` 景）

一台 supervisor 引导镜像（`programs/src/harness/probe/iommu*`）：从**引导账**取
IOMMU 寄存器窗与内核铸的保留授权 → `Dock::open` 把设备区映进本域 → 用
`resource::raw::pole` + 带保留的 `AcquireSource::Source` 取八页，并由
`execution::memory::traverse` 读回**真实物理页起址** → 写 DDT/DC/Sv39 表/队列 →
开 FQ 与 CQ → `DDTP = 1LVL` → `IOTINVAL.VMA` 与 `IOFENCE.C` → 用调试翻译口让硬件
按我们写下的表翻一次 → 拆除（DDTP Off 与三个队列各自确认）→ 同一 requester、
同一 IOVA 再翻一次必须被拒 → 显式撤图并放下保留。

实跑读数（`nu scripts/qtest.nu --package kernel --scene iommu-probe`，release，1 passed）：

```text
cap=0x78e2cf4f10 version=16 pas=56 ddtp_reset=0x0 mode=0 busy=false
ddt=0x8fe63000 root=0x8fe64000 commands=0x8fe67000 faults=0x8fe68000 fence=0x8fe69000 data=0x8fe6a000 iova=0x200000000
ddtp_on=true ddtp=0x23f98c02 fenced=true cqh=2 fqh=0 cqcsr=0x10001 fqcsr=0x10001 cqb=0x8fe67 fqb=0x8fe68 response=0x23f9a800 faults=0 cause=0
ddtp_off=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 isolated_response=0x40001
translated 0x200000000 -> 0x8fe6a000
exit tid=1 reason=0x0 note: iommu-probe: acceptance passed
```

逐条对应：`cqh=2` 说明硬件**真消费了两条命令**；`fenced=true` 说明 IOFENCE 的完成记录
由硬件写回（`FENCE_DATA`）；`response=0x23f9a800` 的 FAULT 位为 0、PPN 字段 = `0x8fe6a`，
就是 `data` 那一页的页号 ⇒ 硬件**按我们写下的 DDT/DC/Sv39 表把 IOVA 翻到了保留页**；
`cqb=0x8fe67` / `fqb=0x8fe68` 说明编程进去的基址正是保留窗里那两页；
拆除后 `ddtp_off=0x0`，两个**已启用**的队列 CSR 回读 0（`on` 落了、`busy` 也清了），
页请求队列本景从未启用（`pqcsr=0x0`，没有 ATS 请求，也没有它的页）；
`isolated_response=0x40001` = `FAULT[0]` 置起且原因码 `bits[53:10]` = 256 = `DMA_DISABLED`
⇒ 同一请求随后被拒，且**不是**拿“页表坏了也是故障”充数（那种情形实测报 13 = `RD_FAULT_S`）。
这些判据都在景内逐条断言（`iommu-probe: <哪一条>` 就是失败名），不是拿复位默认值充负例。

两处口径要说明白，别当成硬件已立的证据：

- **`busy` 位在 QEMU v11.1.2 里是白送的**：`riscv_iommu_write_ddtp` 先清 BUSY 再写，
  `riscv_iommu_process_cq_control` 停用时同步清 `BUSY|ON`。本段已按规范那一手走
  （`regs::ddtp_set` / `queue::raise`/`lower`：写 → **有界轮询**到 busy 落 → 回读校验，
  与 Linux `drivers/iommu/riscv/iommu.c` 同一口径），但在这台机器上 busy 半边恒成立
  ⇒ 握手本身**没有被这台硬件证过**；真硬件真拉 busy 时才会显出差别。
- **次序现在是显式的**：表页 / 描述符 / 命令与门铃之间、看到完成与读回载荷之间都过
  `order::sync()`（`fence iorw, iorw`——设备 I/O 与内存同在一条栅栏下才有序）。QEMU 单线程 +
  BQL 下不给栅栏也成立，故这一条在这里同样**证不了**：它是按规范写下来的，不是量出来的。
- **这一段是线性代码，不是故障安全**：保留在发布之前取得、`close()` 在确认之后才调用，
  但本域被 abort / 杀任务时这里一行都不跑（**Drop 也不跑**，故不把停机寄托在 guard 上）。
  这一窗里能想到的失败不止 `format!` 那一句；真正的答案是产品那一侧 System **独立**持保留、
  并在确认停住之后才释放——服务死掉不改变这一点。

### 跨主体精确 BAR：实测缺口（已补，见下节）

现有接口**只能把精确子窗口给持有者自己**：
`AcquireSource::Source { token, offset, count }`（`kernel/src/entry/call/memory.rs::acquire`）
与 `execution::memory::map(team, at, size, pole, offset, …)` 都按页偏移在**本任务**的
授权里取子窗口。交给**别的**任务时只有整段：`PrivateRoot::derive/Accord`
（`programs/src/system/supply/lease.rs`）只带权限子集、不带 offset/size；
`create_pole`（`kernel/src/entry/call/pole.rs`）明确要求「整段来源 + 私有阶段 +
一次性资格」（`offset == 0 && count == source.page_count()`、`private_owner`、
`hold.branch().is_none()`），且设备区在引导期已 `adopt` 到根 Grant，故从它铸不出新根。
⇒ 把 PCI aperture 交给某个驱动只能是**整段**，这与「只给必需 BAR」冲突。
最小补法（待裁决）：由**源的真实管理者**派生一段有界只 RW/NX 的子窗口并沿既有交付链
给指定主体，复用源的撤销与生命周期，不放宽 `create_pole` 的三条限制。

### 精确 BAR 跨主体：带窗口的分支（已实现）

上一节那一节列的缺口已补：窗口绑在**交付 token 的 branch**（`Grant`）上，
复用源的授权与撤销，不放宽 `create_pole` 的三条限制。

| 面 | 事实 |
|---|---|
| 窗口的坐标 | `Grant.window: Option<(PageOffset, PageCount)>`，用**相对来源起点的绝对页偏移**（不是相对父分支的偏移）⇒ 嵌套派生不会叠加两层偏移 |
| 建窗口的唯一入口 | 新子操作 `PieCall::AccordWindow { src, dst, subset, mark, offset, count }`（slot 11）。旧 `Accord` 的**形状**一字未动（slot 4 的四个字段与语义照旧）；**行为**有一处收紧：
父带窗口时它派生出来的子**继承**那一段（不再是
`None` = 整段），父不带窗口时仍是 `None` = 整段（承重用例 `window_inheritance`）|
| 单调收窄 | `effective_window`：不请求就继承父的有效窗口；请求的必须落在父的有效窗口里（父无窗口就是整段来源），空长与溢出当场拒。非 Pole（无真实来源）上请求窗口一律拒 |
| 使用路径都守窗口 | `acquire`（`AcquireSource::Source`）在解析出源坐标后再核一次窗口（越窗 `Denied`）；`pole::open` 只映窗口那一段，报出的长度与关系也都只是它（本域看不到窗口外的页） |
| 不放大 | 窗口不动 `Access` 上限：`open` 的访问仍由门闩权限折出（只读 / 读写），MMIO 永不可执行；窗口不放宽 `subset` 之外任何一位 |
| 生命周期 | 窗口就在那一枚 Grant 上：撤销 / 封印 / 交回照旧令分支及其 Views 失效，窗口不另开一条命 |
| 仍拒建根 | 带窗口的分支 `hold.branch().is_some()` ⇒ `create_pole` 照旧 `Denied` |

窗口的两条边界要写清（都不是“整张任务图”那一种）：

- 窗口属于**那一枚分支**：同一任务手里可以有同源的几枚 token，各自有自己那一段（权限与窗口各算各的，不并成一张本任务的图）。`AcquireSource::Source` 当场核窗；`AcquireSource::Reference` **不另核**——引用建立时走的就是 `Source` 那条路，其范围已落在建它的那一枚分支的窗口里，`hold.branch()` 也就是那一枚（同任务另一枚更宽的 token 换不掉这个边界）；将来若添直接建引用的入口，同样要从按窗口筛过的分支上建，这条不变量才继续成立。
- 窗口**不可回读**：`PieInfo` 里没有它（`Open` 只报这一段自己的长度，起点不报），故交付方必须把必要坐标说清；给 `PieInfo` 加窗口两字是一次 ABI 变更，本段不做。

实测（`nu scripts/qtest.nu --package kernel`，新增用例 `branch_window_narrows_and_binds` 走**真实调用面**
`EnvCall::Pie` / `EnvCall::Memory`）：越窗 `(2,3)`/`(4,1)` 与嵌套放宽 `(0,2)`/`(2,2)`/`(1,3)` 均 `Denied`、
空长 `(1,0)` 报 `NotAligned`、四笔被拒的派生在目标表里**一格里也不落地**；
窗口 `(1,2)` 落地且回读就是这一段；嵌套 `(1,1)` 更窄成立；
`Open` 只报 `2 * PAGE_SIZE` 且首 PA 等于来源第 1 页、窗口后一页未映射、窗口页**不可执行**；
`Acquire` 窗口外 `Denied` / 窗口内成功；带窗口的分支 `create_pole` 仍 `Denied`；
`Revoke` 之后分支与它的 View 一起消失；不带窗口的 `Accord` 仍拿到整段 4 页。
整轮 `75 passed / 1 failed`（仍只是缺 initrd 的 `scene` 哨兵），
`--scene iommu-probe` 1 passed，`--scene accept` 1 passed(10.39s)，`system-shape` 18 passed。

仍未做：**供给 / 租约那一层已带窗口**（窗口随 `RootAssignment` 与 `Accord` 落到
`env::pie::accord_window`；`LeaseRequest::derive` / `RootRequest` / `resource::Capability::grant_window`
都已接），但**还没有任何调用点真填一个窗口**——能拿得出“精确 BAR 那对绝对页坐标”的真实设备
管理者（System 侧 DMAC/设备 provider）仍未接，故产品里的“System 把精确 BAR 交给某个驱动”还差那一手。

### 真实端点在 IOMMU 翻译下 DMA（本段）

`iommu-probe` 只证了「我们写下的表能翻」；**端点按 IOVA 真访存**是这一景（`--scene iommu-dma`）。

| 面 | 事实 |
|---|---|
| 端点 | QEMU `virt` 上的 `virtio-blk-pci,drive=…,iommu_platform=on,disable-legacy=on`：**真实设备**（不是测具），盘是脚本每次按同一花纹重建的小镜像（`<TRACE_OUT>/iommu-dma.img`，只读挂上） |
| 发现 | `ecam.rs` 在设备树 `pci@…` 那段 `reg`（ECAM）上扫 `1af4:1001`/`1af4:1042`；`virtio.rs` 读能力表拿四个区与 BAR 号；`endpoint.rs` 按标准那一手量长度并落进 64 位 PCI 窗口 |
| 精确交付 | 从 `Page::Region(0x4_0000_0000)` 那一枚门闩上 `PieCall::AccordWindow` 派生**正好那一根 BAR**（16KiB = 4 页）的有界窗口（RW、不可执行、可再授出）；`Open` 出来的视图长度就是它 |
| 发布 | 硬件到得了的六页（desc/avail/used/data/header/status）先经保留强持，再映到**另一段 IOVA**（`0x2_0000_0000`，不是物理地址）；DDT/DC/页表/队列都用真实物理页起址 |
| 正证 | 队列三段指向那三个 IOVA；一轮 `VIRTIO_BLK_T_IN` 读扇区 0 → `used=1 id=0 len=513 status=0`，数据页 512 字节与镜像花纹逐字节相同，FQ 无记录，调试口 `translate` 指回同一物理页、另一 requester 被拒 |
| 负证 | 先 `IOTINVAL` ＋ `IOFENCE.C`（拿到完成记录），再把 `DDTP` 落 Off 并回读确认；同一端点、同一段数据再打一次门铃 → 不再前进（`idle=1`）、哨兵页一字节未动、状态字节未被改写、调试口同一条请求报 `0x40001`（cause 256 = `DMA_DISABLED`） |
| 收口 | `IOFENCE.C` 再走一遍（队列还活着时它能完成），再按握手停两台队列；确认到位之后才 `close()` 归还保留 |

**两个角色**：设备那一半（配置空间之后的寄存器那一面）归**端点服务**——一台同 Team 的真任务，
它手里只有一枚精确到一根 BAR 的窗口（自己核 `Open` 长度与交付的页数相等）与几个 IOVA；
内存那一半（队列/描述符/数据页）与 IOMMU 寄存器归控制者，那是所有者的保留。
服务在第二轮摆好之后**退场**（不撤队列、不复位设备），于是“服务死时端点还是活的”是真的：
`ddtp_off` 那一步、以及它之后由**控制者自己那一枚窗口**打的那一次门铃，都不经过任何服务；
确认（`refusals` 里那条 did=8 的 256 记录 ＋ 哨兵页一字节未动）之后才 `close()` 归还保留。

**这里的“控制者”是本景那一台探针任务（harness owner），不是 System 产品实现**：它自己持保留、
自己收口，靠的是本域直接编程 IOMMU 寄存器；`0c9b2608` 的标题里的 “System close alone” 是**过度宣称**
（那一版提交不改写，以本段为准）。同理，两台是**同一 Team（同一 AS）**：服务看完成记录读的是
控制者那一页的**裸 VA**（`HANDOFF.used_at`，`used` 环不在它那一段 BAR 里），跨 AS 的独立服务需要
它自己的授权映射或经一次完成通知，故“服务死 → System 独立隔离”在产品组合里仍未接上。

**QEMU 那两格不上不行的**（都实测过）：`iommu_platform=on` 才让设备的 DMA 走那台 IOMMU——
`virtio_bus_device_plugged` 默认给的是 `address_space_memory`，不开它时 `riscv_iommu_dma`
一次都不出现、设备静默停在读不到可用环那一格；`disable-legacy=on` 是它的代价
（QEMU 不给过渡态同时开这两样）。两格都只写进 `scripts/qemu-args.nu` 的 `--endpoint` 那一口，
板子默认参数没动（`--iommu` 仍只在 `iommu-*` 景打开）。

**证据**（`nu scripts/qtest.nu --package kernel --scene iommu-dma`，release）：
`cap=0x78e2cf4f10` / `requester=8 bar=4 probe=Some((16384, true))` / `address=0x400000000` /
`ddtp=0x23ff0002`（1LVL）`queue=256 served=1 used=1 id=0 len=513 status=0 faults=0` /
`translate=0x23ff2800`（＝数据页的页号）`neighbour=0x40001`；
服务那一行 `service queue=256 used=1 status=15`（它自己立起队列、打响门铃、看到完成）；
服务退场之后 `ddtp_off=0x0 fenced_before_off=true idle=1 refusals=8 refusal_cause=256 refusal_did=8 refusal_ttype=3`
（`refusals` 那几条就是设备在隔离态被拒的真尝试：每次都记一条），
末行 `iommu-dma: acceptance passed`。

### 未做（不宣称已完成）

> **本节是历史定位。** 下面记的是 PR7 第一段那个基线（`HEAD = 73af1c13…`）当时的状态；“产品接线”
> 与“供给/租约无调用点”两条已被后来的工作接上，以本文件末节
> [「当前交付状态」](#当前交付状态产品链已接head-7237712b) 为准（本条不重写旧记录，只定位）。

- **产品接线**：`programs/src/system/` 没有安装真实 `Platform`，也没有独立 DMAC 服务，
  故「服务死亡 → System 不依赖死服务独立隔离」这条路径在**产品组合**里仍未接上
  （PR6 的 `Holds`/`Platform` 机制仍在，但没有真实寄存器实现挂上去）。`iommu-dma` 景里
  这条形状已经真跑过：服务是一台另起的真任务、只持精确 BAR 窗口，它退场之后由控制者独自收口；
  但那两台同 Team（同一 AS），故内存那一侧还没分成两个域。本景的收口是本域直接编程寄存器
  （线性代码），**abort / 被杀任务时一行都不跑**（Drop 也不跑），故它**不构成 System 的独立停机权威**，
  不能拿本景的正负证当产品 retention 完成。
- **供给 / 租约那一层已带窗口，但无调用点**：窗口随 `RootAssignment`（`PrivateRoot::derive`）与
  `Accord` 落到 `env::pie::accord_window`，`LeaseRequest::derive` 收 `RootAssignment`，
  `resource::Capability::grant_window` / `Loan::accord_window` 也在（都有用例）。缺的是**产品侧
  那个知道精确 BAR 坐标的真实设备管理者**：没有它，产品里就没人填得出那个窗口。
- 多 hart 交错、真实在途发布竞态、ATS/PRI、二级（G 阶段）翻译都不在证据里。

### 验证命令与读数

| 检查 | 结果 |
|---|---|
| `nu scripts/qtest.nu --package kernel` | 75 passed / 1 failed（缺 initrd 的 `scene` 哨兵，与基线逐字相同） |
| `nu scripts/qtest.nu --package kernel --scene iommu-probe` | 1 passed（读数见上） |
| `nu scripts/qtest.nu --package kernel --scene iommu-dma` | 1 passed（端点真 DMA 的正证与隔离负证，读数见「真实端点」那一节） |
| `nu scripts/qtest.nu --package kernel --scene accept`（带 `iommu-sys=on` 的预跑） | 1 passed，10.39s |
| `cargo test --manifest-path programs/tests/system-shape/Cargo.toml --target x86_64-unknown-linux-gnu` | 18 passed |
| `cargo build -p programs --release` | 通过；新代码无新增告警（既有 10 项保留） |
| `git diff --check` | 通过 |

## 追加：Transfer / Traverse

物理页迁移与 PA 出口采用 `Transfer / Traverse`；追加顺序、驻留保证、
ABI 槽号及安全验收门见 [实施计划](memory-transfer-traverse.md)。
这不把原 PR7 的真实硬件接线或产品验收自动标记为完成。

## 当前交付状态：产品链的持有与停止已迁进 IOMMU 驱动（待父线程封存；上一封存 `6d9ecf68`）

本节记的是**当前**产品链的交付状态与可复跑的验收命令；前面各 PR 段落的执行记录
仍作各方当时的基线保留，与本节不一致处以本节为准。

### 产品链（已验证的形状）

IOMMU 驱动（镜像 `iommu` 一族，自己的 Task/Space）起一台**跨 Team** 的端点服务
（镜像 `system-dma`，sire 就是它），把**精确到那一根 BAR** 的窗口
（实测 4 页 / 16KiB）经 `accord_window` 交给它；服务手里没有任何裸 VA，只有那一枚窗口与几个 IOVA。
内存那一半（DDT / DC / Sv39 表 / 三类队列 / 描述符 / 数据，共 13 页）由 IOMMU 驱动保留并发布，
驱动自己的持有账是唯一的收口权威。

- **正证**：真 `virtio-blk` 读 512B —— `iommu: served used=1 len=513 status=0 written_ok=true sector_ok=true`。
- **正常收口**：System 发关闭请求 → `iommu: confirmed and released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 master=0x0 status=0 queue_enable=0 stopped=true`，
  驱动回成功结果后才报 `system: device closed (driver confirmed)`。
- **故障收口**：杀掉服务 → `iommu: service doomed doom=Ok(()) slay=Some(Ok(()))`，
  第一次停住答 `Pending` 时保留全部（`iommu: NOT released holds=1 phase=Closing(Fault) failure=Some(Pending)`），
  恢复真确认后才释放（同一行 `stopped=true`）。
- **起手受控失败**（两档各自的景）：落位后·发布前读 `bus-master off=0x0 … stopped=true`（设备从未接上，
  停机链不走）；发布与授权都过了之后才失败那一档读 `stopped=true`（平台真把设备停了才释放）。
  两档都报 `child reclaimed`（`join_task` 看见子任务真被回收才计数）与 `range close=Ok(())`。
- **授权与落位分开**：进配置先核 `bus-master off=0x0`，帧持有/登记/IOMMU 约束就绪之后才
  `bus-master on=0x4`；交出去的三段窗在 System 侧再 `Dock::open` 得 `handed over`（负证）。

### 停止依据与它的边界

停链次序是 `DDTP` 落 Off → `IOTINVAL` → `IOFENCE.C` 等本次完成 → 端点 bus-master ＋
`virtio` 复位回读 → 两台队列停机确认。**端点侧那一步是本所选平台能给的凭据**：QEMU v11.1.2 下
`status = 0` 同步走到 `virtio_blk_reset` 的 `blk_drain()` 并摘掉排队请求（BQL 下在那一刻写完），
故回读那两格可作本设备“无在途”的凭据；本机也**没有** posted-write 模型层。
**边界**：物理 PCIe 上“已到线、未落内存”那一档要 interconnect-specific fence，本平台给不出，
本节也不为它作保。

### 受控注入与自然超时分得开

两个起手故障点是**镜像变体的注入**（同一身子、不同镜像，景名选哪一张写在 `Deployment.resources`；
生产无数常开的后门），它自己报 `startup fault injected …`；**自然超时**走的是同一个驱动层期限
（`regs::Handshake`），报的是 `iommu: stop pending at …`。两者不是同一件事，不能互相代替。

### 验收命令与读数

| 检查 | 结果 |
|---|---|
| `nu scripts/qtest.nu --package kernel` | 77 例：**76 passed / 1 failed**（缺 initrd 的 `scene` 哨兵，**不是全绿**，与基线一致） |
| `nu scripts/qtest.nu --package kernel --scene iommu-product` | 1 passed |
| `nu scripts/qtest.nu --package kernel --scene iommu-product-fault` | 1 passed |
| `nu scripts/qtest.nu --package kernel --scene iommu-product-start-before` | 1 passed |
| `nu scripts/qtest.nu --package kernel --scene iommu-product-start-after` | 1 passed |
| `nu scripts/qtest.nu --package kernel --scene iommu-dma` | 1 passed |
| `nu scripts/qtest.nu --package kernel --scene iommu-probe` | 1 passed |
| `nu scripts/qtest.nu --package kernel --scene product` | 1 passed（无设备那一档，行为不变） |
| `cargo test --manifest-path programs/tests/iommu-queue/Cargo.toml --target x86_64-unknown-linux-gnu` | 15 passed |
| `cargo test --manifest-path programs/tests/retention/Cargo.toml --target x86_64-unknown-linux-gnu` | 6 passed |
| `cargo test --manifest-path programs/tests/supply/Cargo.toml --target x86_64-unknown-linux-gnu` | 23 passed |
| `cargo test --manifest-path programs/tests/system-shape/Cargo.toml --target x86_64-unknown-linux-gnu` | 18 passed |
| `cargo test --manifest-path programs/tests/lease-window/Cargo.toml --target x86_64-unknown-linux-gnu` | 2 passed |
| `cargo check -p programs --release` | 通过，零告警 |

本节三笔提交：`75d08d67` 非阻塞产品帧、`615356b3` 两条受控起手失败、`7237712b` 窗口分派承重测试。
**其后两项裁决已落地（提交 `6d9ecf68`）**：文件按“一型一文件”机械归位（行为未改，只改了可见性与
导入路径），以及**发布之前失败也撤端点发起权限**（回退清 ECAM bus-master 并按平台 drain
同一形状回读；已发布那一档仍走完整 Platform 链，不用清 master 冒充排空）。因此
`iommu-product-start-before` 的现场现在是 `endpoint bus-master=0x0` 与
`master=0x0 … stopped=true`（master 也归 0 之后五项读数全落到停止值），
`start-after` 另加一条“回退自己那一行不得出现”的反向断言。**这一笔已过小增量复核（见下节）。**

### 仍未覆盖

- 物理 PCIe 的 posted-write 排空（见上“停止依据与它的边界”）。
- `NativePieEffects` 的 `inspect`/`revoke` 两半与 `PrivateRoot::derive` 那道门没有宿主见证
  （`inspect` 在宿主上是真 ecall）；`iommu-queue` 桩也见证不到 `enable` 成功路径与“busy 抬起再落”。
- 自然 `Handshake` 到期（平台报 `stop pending`）没有端到端场景见证（QEMU 必答）。
- `reset endpoint idle` 仍不敏感，只当“复位后端点闲”命名，不是 IOMMU-only 负证。
- `!self.attached` 那一条（`startup failed, no registered scope`）**仍无场景**：三个起手分支里两个
  已见证，第三档与前面的裁断一致——要它就得各自的小景/进程。
- 物理 PCIe 的 posted-write 排空仍未覆盖（见上“停止依据与它的边界”）；未发布那一档现在不留
  发起权限，但那是**撤回我们自己写下的 PCI 配置**，不是 interconnect-specific fence。

### 独立审查结果与已记录的边界

同组独立 reviewer 对 sealed `HEAD = 7237712b`（blob `ccfe78adce6b` / `3bfe0c40f5b7` /
`ff8f8e66b92b` / `e8c6b1804393` / `06c80a42dc18` / `dd2cbf6b55ec`）与文档提交 `88ef1072`
（仅 docs，+71）给出了结论：**acceptable，无阻塞**——**接受的正是 `7237712b` 那一版**。
三个关键负例都真红：`fence_step` 每次重发→15 例中 6 例红；`IommuPlatform::stop()` 换成空实现
→ `iommu-product` 红且 `confirmed and released … stopped=true` 与 `reset endpoint idle …`
两行同时消失（`arm(round)` 修后这一行是真的）；`reclaim` 里吞掉 doom/slay → `start-before` 红
（`缺少 … child reclaimed`）。静态面也核过：产品路不再碰同步入口，`system/device/` 下无
`park`/`Wait::AtMost`/`Wait::Forever`，System 帧可达处没有 2s / 10M 等待。

审查一并记录的边界（均无阻塞）：

1. **发布前失败留下的 PCI 配置**（这一格已按裁决改过，下面是**现行**说法）：`spawn` 后·发布前失败时
   停机链正确地被跳过（什么都还没发布），故回退**自己**清掉 ECAM 的 bus-master 并按平台 drain 同一
   形状回读确认，未发布不再留下**发起权限**。仍不回退的是**内存空间使能与 BAR 落位**那两笔
   （它们不给发起权）；若项目要的是把这几笔 PCI 配置也全部拆掉，那仍是发布前回退的**行为**变更。
2. `!self.attached` 那一条（`startup failed, no registered scope`）没有场景；三个起手分支里两档已见证，
   第三档与前例同一裁断：要它就得各自的小景/进程。
3. `dma.rs` 现持有 5 个类型（`Dma`/`Started`/`Path`/`Stage`/`Start`）、`platform.rs` 4 个。几个小枚举
   都只服务于那一个带状态的结构，但若“一个文件一个数据结构”按严格读法执行，这一格待定。
   （**已裁决并落地**：严格适用，已按一型一文件拆开，见下。）
4. reviewer 未重跑：`iommu-dma`/`iommu-probe`/`product` 三景、supply 23 / system-shape 18、
   kernel 76+1 哨兵、`cargo check` 告警与 fmt/diffcheck（均归本报告）。

### 增量复核（`6d9ecf68`）：acceptable，无阻塞

同一位 reviewer 对 `6d9ecf68d80a7a6a9c05dd1c44eee3d287ab0657`（在它接受的 `7237712b` 之上
**一笔代码提交**；它那个 checkout 的 HEAD 仍是 `99f03159`、新提交只作为对象存在，故封存按 hash 记）
做了小增量复核：逐个 blob 核过（`queue f19e1f4654db` / `regs 2cb745ec7e72` / `handshake 3a48ca5f8ace` /
`fence 1750d839116d` / `outstanding 5294a8b62a84` / `dma 1899c6cee067` / `platform ff9f10b12e93` /
`endpoint_drain dd862785c683` / `start e1011ee3844c` / `stage 251bf12c5a71` / `path d1f372ea13cb` /
`started 250c583112d5` / `stop c3181c46e6e6` / `step 080754b5c1c7` / `holds 06c80a42dc18` /
`lease dd2cbf6b55ec` / `qtest b686e0e52231`），并从拆分前后各文件抽出 fn/struct/enum/trait/impl
多重集做对比（91 vs 93）：差异只有预期的四处（`Dma::new` 多了两个字段、`rollback` 多一句
`revoke_authority`、两个新方法、`Outstanding` 字段放宽到 `pub(super)`），
`stopped`/`stop`/`step` 与已接受那一版**逐字节相同**；一型一文件处处成立。实测 `start-before`
与 `start-after` 均 1 passed，且“移除 `revoke_authority` 的 `published` 提前返回”能真弄红
`start-after`（反向断言承重）。

它一并记下的两条：

- **不要再用 `stopped` 当“哪一档”的信号**：master 归 0 后两档起手失败的 `stopped` 都是 true；
  区分靠注入标记与 `endpoint bus-master=` 那一行的有/无（`start-after` 断言它不出现）。
- **两条 fail-closed 分支仍无场景（设计如此）**：`authority_clear` 的“未确认”那一支（QEMU 做不出
  配置写失败）与 `!self.attached`。两者都**保留责任**而不是报成成功，故它们是代码级的保证，
  不是被见证的保证。
