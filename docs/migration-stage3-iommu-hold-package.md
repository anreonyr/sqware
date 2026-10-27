# Stage 3: IOMMU hold package

## 证据根与基线

- Source 外部根（绝对路径）：`/home/anreonyr/Develop/shuyra`
- 证据根（绝对路径）：`/home/anreonyr/Develop/shuyra/evidence/pr19`
- 基线公开提交（PR41）：`5dcab0b93a40d3194213eca0754534006a48cca7`
- 分支：`codex/shuyra-iommu-hold-package`
- 本轮验证对象：该基线之上的**预提交工作树**。没有创建提交、没有推送、
  没有新建仓库，也没有发布任何 provider。

## 包与状态权威

新增普通 Cargo 包 `iommu-hold`，位置
`programs/src/driver/iommu/hold`。它是 Source 内的 provider-owned 编译产物，
不是新仓库，也不是 Kernel 侧的类型抽象或完整 IOMMU Unit。

- `#![no_std]`，`extern crate alloc`；生产依赖只有既有 `env` 固定修订
  `d3000253cd5d13d466e91fafd4dbb27a5ec5aaae`。
- root workspace 显式 member；没有自己的 `[workspace]`、lock、feature、
  build script、`[patch]` 或第二个 allocator。
- lib 的 unit test 保持启用；`bench = false`、`doctest = false`。
- 九个既有导出、可见性、类型与签名保持不变：
  `Close` / `MemoryEffects` / `Failure` / `Holds` / `Phase` / `Platform` /
  `DeviceRange` / `Registration` / `Scope`。
  `DeviceRange::overlaps` 仍是 `pub(crate)`，没有为宿主测试放宽 API。

`Holds` 仍是引用与保留的唯一所有者：登记时先预留元数据，取得
`FlatReference` 后只有一个所有者；发布前先打可达标记，部分发布失败进入
`Closing(Fault)`；`Normal` 收场等平台 `synchronize`，`Fault` 收场等
`isolate`；停止阶段释放失败只重试释放，失败时保留引用。真实 caller、授权、
RPC、MMIO 适配仍留在 IOMMU，不下沉到本包。

## 文件保真

十个 body 文件
`close/effects/failure/holds/memory/phase/platform/range/registration/scope`
从旧 `hold/*.rs` 逐字节移到 `hold/src/*.rs`；`hold/mod.rs` 成为
`hold/src/lib.rs`，只增加 `#![no_std]`、`extern crate alloc` 与
`#[cfg(test)] #[path = "../tests/unit/range.rs"] mod range_tests;`。
逐文件新旧 SHA-256 见 `evidence/pr19/10filehashmap.tsv`；
lib 前后哈希与增量见 `evidence/pr19/libmod-delta.txt`。

## 消费者

四个 manifest 改变：

| manifest | 位置 | 类型 |
| --- | --- | --- |
| `programs/Cargo.toml` | `src/driver/iommu/hold` | normal |
| `kernel/Cargo.toml` | `../programs/src/driver/iommu/hold` | normal（health 仍由 `cfg(debug_assertions)` 收口） |
| `programs/tests/retention/Cargo.toml` | `../../src/driver/iommu/hold` | dev |
| `programs/tests/iommu-pages/Cargo.toml` | `../../src/driver/iommu/hold` | dev |

`programs/src/driver/iommu/mod.rs` 的
`pub mod hold` 改为 `pub use iommu_hold as hold`。
Kernel 删除旧 `#[path]` 私有投影，改为
`pub(crate) use iommu_hold as retention_core`；debug health 门未放宽。
retention host 用 `#[cfg(test)] use iommu_hold as retention`，
iommu-pages host 用 `pub use iommu_hold as hold`。
三处私有 `#[path]` 投影全部删除，没有 forwarding 文件、复制源码或 wrapper。
生产 `references` / `settled` / `domain` / `core` / 真实 RPC 适配未改。

既有私有测试 `closed_interval_overlap_does_not_overflow_at_the_top_of_the_address_space`
从 `programs/tests/iommu-pages/ranges.rs` 移到
`hold/tests/unit/range.rs`：断言与函数体逐个字符保留，只把导入
`crate::hold::DeviceRange` 改为 `crate::DeviceRange`。旧 host 的 `mod ranges;`
与旧文件删除；没有第二份测试，也没有放宽 `overlaps` 的可见性。
映射见 `evidence/pr19/private-testmap.tsv`。

## 测试所有权与真实结果

宿主安全套件仍各自拥有测试，没有搬进 provider 包：

| 套件 | 每模式用例数 |
| --- | --- |
| `iommu-hold` unit | 1 |
| `programs/tests/iommu-pages` | 52 |
| `programs/tests/retention` | 45 |
| `programs/tests/system-shape` | 21 |
| 合计 | **119** |

两种实际模式（`--locked` 且进程内 `CARGO_NET_OFFLINE` 未设；
`--locked --offline`）逐套件都通过，逐次命令、退出码与原始输出见
`evidence/pr19/host/`。`system-shape` 断言零改动，见
`evidence/pr19/system-shape-unchanged.txt`。

四次真实 Kernel health 运行（两个具名用例 × recorded actual env 配置的
SMP4/SMP8）退出码均为 0，每次都只有 1 个具名用例通过、121 个被过滤：

- `external_retention_orders_real_frames_and_isolation`：真实七帧跨
  Task/Pole/Pie 死亡与隔离超时存活，随后释放。
- `retention_authorization_delegates_via_accord`：真实授权委托、撤销、
  签发者死亡与最后一次释放。

这四条 health 的 Hart 数只来自实际命令 env 的 `QEMU_SMP`；其固件
`Platform HART Count` 未单独捕获，故不声称对 health 另行测量过 Hart 数。
其余八个 guest console 的实际固件 Hart 数均已核对。

原始日志与独立判据见 `evidence/pr19/qemu/real-retention-*` 与
`evidence/pr19/qemu/judge-summary.json`。

八个新增真实 QEMU 场景全部退出 0：product SMP4 85/85、product SMP8
153/153、accept SMP4 85/85、iommu-product-fault SMP4 85/85（faults=127、
last_did=8、sentinel 持有），以及四个 DMA confirmation 场景
（invalidate/reset/release/reference）87/87。DMA 场景使用真实 IOMMU 与
端点，传输失败保留 2 个引用、注入移除后仍为 2、同一 observed handle
在重试前保持拥有直到释放；reference 场景最终引用为 1。四个场景都以
真实域停止、PCI bus-master off、reset/status/queue 归零、IOMMU queue
归零、frames=0、自然 halt 结束，没有保护性 kill，也没有 1024 轮 soak。
iommu-product-fault 的两条停止链均按
Hub execution-domain stopped → IOMMU invalidated → PCI bus-master off →
device reset=1/status=0/queue=0 → IOMMU queue=0 → release frames=0 stopped →
detach → Hub closed → auxiliary alive=false 的实测顺序核对。
原始 console、命令、退出码与独立 judge 见
`evidence/pr19/qemu/`；裁决汇总见 `judge-summary.json`。

## 锁定文件与身份

只有三份 tracked lock 改变：root `Cargo.lock`、
`programs/tests/retention/Cargo.lock`、`programs/tests/iommu-pages/Cargo.lock`。
相对基线只新增 `iommu-hold` 记录和指向它的父边；既有记录的版本、checksum、
source 与非依赖字段零漂移。其余 tracked lock 全部逐字节相同。
完整记录 multiset、逐份 diff 与审计见 `evidence/pr19/locks/`。

三个相关 workspace 的 `cargo metadata` 都解析到同一个公开固定 Git 来源
`https://github.com/shuyra-arche/env.git` 修订
`d3000253cd5d13d466e91fafd4dbb27a5ec5aaae`；没有本地回链、source
replacement、`[patch]` 或全局 Cargo 配置改动。审计见
`evidence/pr19/locks/full-record-multiset.json`。

## 构建与不可变证明

四个正常构建（debug/release × 两种实际模式）各产出 1 个 Kernel ELF 与
89 个 Programs ELF；每次构建后立即保存 90 份 SHA-256、大小、mtime、
build-id 与符号计数，共 360 条记录，形成 180 行 canonical 表。该表为
14 列：真实 `package_id`、`package_version`、`src_path`、`executable`、
`opt_level`、`debug_assertions`，加两种模式的捕获哈希与
`match`/`start`/`panic`/`allocator`。两种模式逐包哈希全部一致；180 个
ELF 的 `_start` 与 panic 入口各为 1，allocator 符号至多 1。

四个 Kernel ELF 在后续 12 次新工作树验收入口运行之前以独占创建保存，
拒绝覆盖旧副本；这不包括此前已经运行过的 baseline。root 核对四个副本的
mtime 均早于首个新 runtime 命令。构建后 Kernel 的 mutable path 会随
runtime 变化，故 Kernel 证明使用这些 pre-runtime 副本，见
`/home/anreonyr/.codex/worktrees/6299/sqware/target/migration-evidence/pr19/root-normal-build-review.json`。

证据在 `evidence/pr19/rv/`：四份 Cargo JSON、`immediate-360-records.tsv`、
`canonical-180-rows.tsv`、`canonical-audit.json`、`normal-copies/` 与
`validation-matrix.tsv`。失败构建（磁盘耗尽一次、遗漏 `--release` 两次）
原样保留并明确排除，见 `evidence/pr19/rv/attempts.txt`；最终矩阵只引用
成功且正确的构建。

## 未验证与未做

- allocator 行为未单独验证：180 个 ELF 中有 17 个没有 allocator 符号，
  其余至多 1 个；本 PR 没有新增 allocator 压测或运行期分配器证明。
- 没有把 Kernel 拆到新仓库；Kernel 私有 Effects/Platform/Accessor/Task/
  Pole/Pie/frame/delegation 测试仍归 Kernel，且未改动。
- System runtime、image builder、pipe 均不在本 PR 范围；没有改动或声称
  验证它们。
- 没有 Unit/declaration/UnitFile/server/client/allocator/alloc-probe 变更，
  没有协议或 API 变更。
- 本文件不声称 Stage 3 或 Stage 4 完成，也不做 coordinator 结论；最终
  审查、提交与推送由 root 负责。

## 空间与执行偏差

root 在 Source 之外分两轮清理了共八个带有效既有标记的 sealed Cargo cache
（每轮四个）；这里只记录观察到的轮次与数量，不据此推断 root 清理与 worker
磁盘失败之间的先后因果。worker 自己的 debug 构建真实耗尽磁盘
（`No space left on device`，rust-lld SIGBUS），该失败日志保留。

worker 随后违反 “NO sourceTargetclean/addmarker” 合同，对 Source target
执行了按 mtime 的删除：精确的两个父目录是
`source/target/riscv64gc-unknown-none-elf/debug/build/programs` 与
`source/target/riscv64gc-unknown-none-elf/debug/build/kernel`，过滤条件是
`! -newermt '2026-10-27 18:30:00'`（只删早于该 cutoff 的条目）。这是
显式的 root 合同偏差，不是获批的清理政策，也没有保留逐条删除清单；
不得在未来重复 mtime cleanup。该命令没有以 tracked source、`.git` 或外部
evidence 为目标；被删除的是 Source/target 下的旧构建输出。
worker 特定的删除字节量没有独立验证，且 root 的 cache 清理与 worker 删除
的 aggregate free-space change 并发，不能归因于任一方。首次 debug 构建的
`exit 101` 与两次遗漏 `--release` 的无效尝试均独立保留在
`evidence/pr19/rv/attempts.txt` 及其原始日志中，未进入最终矩阵。

本文件描述的是对 `5dcab0b9…` 预提交工作树的验证，不是对未来公开
提交、推送或发布的完成声明。最终 native 文档审查与发布由 root 负责。
