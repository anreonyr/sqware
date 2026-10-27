# Stage 3: Device serialization API packages (IOMMU / PCI)

## 证据根与基线

- 证据根（Source 之外，绝对路径）：
  `/home/anreonyr/Develop/shuyra/evidence/pr18`
- 基线公开提交：`2725fd5469eeab2f9ba4bf613cdb59ca0e920b87`
  （`https://github.com/anreonyr/sqware/pull/40`，
  分支 `codex/shuyra-device-api-packages`）。
- 本轮验证对象：该基线之上的**预提交工作树**；不是新提交，也不是另一个仓库。
- Source 只新增/改写两个普通 Cargo 包与其消费者；没有新建仓库、
  没有发布、没有改 Unit/declaration/UnitFile，也没有搬 Kernel。

## 范围

新增两个**无状态**的协议包，放在既有 legacy 仓库内：

| 包 | 路径 | 内容 |
| --- | --- | --- |
| `iommu-api` | `programs/src/driver/iommu/api` | `Wire`/`Acquire`/`Step`/`Establish`/`Said`/`Fail`/`Call` 与动作码、`PARTY`、`BACK`、`ENTRY` |
| `pci-api` | `programs/src/driver/pci/api` | `Wire`/`Place`/`Toggle`/`Said`/`Fail`/`Call` 与动作码、`BAR_MARK`、`BACK`、`ENTRY` |

两包都是 `#![no_std]`，依赖只有既有 Foundation full SHA
（`env` `d3000253…`、`wire` `0ff77dc4…`、`mold` `f52501f4…`）
与 declaration/Hub API 的普通 path；没有 server/client/test-fixture 依赖。
root workspace 以显式 member 接入，没有自己的 `[workspace]`、lock、
feature、build script 或 `[patch]`。

## 状态权威

协议包只拥有线上词汇：字段、长度、动作码、失败码、记号与缓冲上界。
它们不持有 Domain、绑定、页帧、BAR、队列或任何 live hardware state；
不执行 acquire/release、rollback、sync、retry，也不决定调用者身份。

IOMMU/PCI 的 server/core 仍全部留在 `programs/src/driver/{iommu,pci}`：
实际 caller 核对、授权、停止、释放、生命周期与硬件读写都没有搬。
`programs` 的驱动根只把旧 `pub mod api` 改为对编译产物的
`pub use iommu_api as api` / `pub use pci_api as api`，公共词汇不变。

PCI `CONFIG` 仍是 `pci-declaration` 的私有记号：
API 不再 `pub(crate) use` 它；`pci/inspection/mod.rs` 直接使用
`pci_declaration::marks::CONFIG`。没有把 `CONFIG` 公开，也没有复制常量
或新 wrapper。

## 文件与保真

IOMMU 的 7 个 body 文件（`acquire/call/establish/fail/said/step/wire`）
从 `api/*.rs` 逐字节移到 `api/src/*.rs`；`api/mod.rs` 成为
`api/src/lib.rs`，只增加 `#![no_std]` 前缀。PCI 旧 `api.rs` 拆为
`src/{lib,place,toggle,wire,said,fail,call}.rs`；每个文件一个既有数据类型
及其 impl，root 只留常量与 reexport。`call.rs` 保留私有 `reply_to`。

字段、`#[frame(len = …)]`、`#[code(…)]`、`#[wire(…)]`、match 分支与
历史编解码行为均保留；没有趁搬家“修正”长度、容量或分支。PCI 的
`Place`/`Toggle`/`Said` 实际编码仍分别是 22/21/13 字节，而 `LEN`
仍是 24/24/16。

## 消费者

7 个 manifest 按合同改变：

| manifest | 位置 | 类型 |
| --- | --- | --- |
| `programs/Cargo.toml` | `src/driver/{iommu,pci}/api` | normal |
| `kernel/tests/dma-confirmation/Cargo.toml` | `../../../programs/src/driver/{iommu,pci}/api` | dev |
| `programs/tests/virtio-blk-coord/Cargo.toml` | `../../src/driver/{iommu,pci}/api` | dev |
| `programs/tests/iommu-pages/Cargo.toml` | `../../src/driver/iommu/api` | dev |
| `programs/tests/pci-placement/Cargo.toml` | `../../src/driver/pci/api` | dev |
| `programs/tests/pci-scan/Cargo.toml` | `../../src/driver/pci/api` | dev |
| `programs/tests/device-recovery/Cargo.toml` | `../../src/driver/pci/api` | dev |

各 host 仍是独立 workspace。所有旧 API `#[path]` 入口改为普通 crate
alias（`api`/`pci`/`iommu`/`pci_api`/`iommu_api` 原名保留），没有源码
再包含或转发实现。Kernel DMA host 只改这两处 import；Kernel core、
hooks、Kernel test API、Fresh/Race 门均未动。

7 份 tracked lock（root 与六 host）只增加两个 API 包记录与指向它们的
父边；不存在的版本/checksum/source 或其他 package 字段零漂移。
`/home/anreonyr/Develop/shuyra/evidence/pr18/locks/` 保存每份完整记录
multiset 与 diff，`lock-metadata-audit.json` 给出 7 个 workspace 的
single-identity 与 Foundation 检查。

## 新增协议测试

`programs/src/driver/pci/api/tests/protocol.rs` 是三个独立的公开
golden 测试，没有 helper 框架或新类型：

1. 四个请求形的精确字节、截短/多余/未知动作拒绝、变体与 `op`
   不匹配时 `store` 拒绝；
2. 应答精确 13 字节与 16 字节容量，截短/多余/16 字节补齐的旧畸形
   全部拒绝；
3. `Call::BACK`、每个变体的回信 token、`BAR_MARK`、`ENTRY`/路径、
   动作码与失败码。

IOMMU 既有 `programs/tests/iommu-pages` 套件未改；其中
`provider_wire.rs` 仍作为公开消费者逐字覆盖 6 个动作、长度
13/61/42/38、接收缓冲 64/48 及截短/多余/未知动作拒绝；失败词汇与
协调行为由同套件的域/协调用例实际覆盖。没有复制第二套 IOMMU codec
测试，也没有给这个 host 增加新的 API 源码投影。

## 实际验证

最终计入的矩阵是 42 条命令。每个命令先只带 `--locked`（进程级
`CARGO_NET_OFFLINE` 未设），缓存准备后再带 `--locked --offline`；
command/env/raw exit/log 在
`/home/anreonyr/Develop/shuyra/evidence/pr18/{host,rv,metadata}`。
这 42 条全部 `exit=0`。该 42/42 **不包含**本会话早期的失败尝试；
失败尝试另见 `evidence/pr18/rework/historical-attempts.md`
（`Said::fetch` 编译失败、PCI 缺 `#![no_std]` 的 RV64 编译失败、
QEMU runner 的 `env -u` 解析失败、judge 硬编码 handle、旧 artifact
审计缺陷）。没有改全局 Cargo home、Rustup 或 Cargo 配置。

| 检查 | 结果 |
| --- | --- |
| 新 PCI golden | 3 passed（两模式） |
| 六个既有 host 套件 | 125 passed（两模式；21+17+8+53+20+6） |
| 两 API 的 host / RV64 `--lib` build | exit 0（两模式） |
| root workspace check | exit 0（两模式） |
| normal Kernel 1 + Programs 89，debug/release | exit 0（两模式，Cargo JSON） |
| 修正后的 180 行 artifact 表 | 4 个 capture 各 90；debug/release 各 90；hash 唯一；start/panic=1；allocator≤1 |
| offline 与 locked-only capture hash | debug 90/90、release 90/90 逐项相等 |

## 证据返工：normal build 与 chronology

2026-10-27 的 evidence rework 只重跑四次 actual normal build，没有改
source code、tests、locks、timeout、reclaim、assertion 或 codec。命令 ID：

- `debug-locked-only`、`debug-offline`
- `release-locked-only`、`release-offline`

每次命令的 CargoJSON、raw exit 和 90 个 ELF 的 hash/symbol 记录在
`evidence/pr18/rework/builds/<command-id>/`；记录是在该命令结束后、
下一条命令开始前立即写下的。后续比较只读这些 capture 记录，不再从
可变 executable 路径取 hash。新的 180 行表在
`evidence/pr18/rework/artifacts.tsv`，审计在
`evidence/pr18/rework/artifact-audit.json`。

debug 90/90、release 90/90；debug allocator 出现 83、release 80，
均 ≤1。两种模式的 capture hash 逐项相等。

四次 build **都在既有 8 个 QEMU 场景之后**。新的不可变 normal Kernel
副本在：

- `rework/normal-kernel-copies/debug-locked-only/shuyra-debug`
- `rework/normal-kernel-copies/debug-offline/shuyra-debug`
- `rework/normal-kernel-copies/release-locked-only/shuyra-release`
- `rework/normal-kernel-copies/release-offline/shuyra-release`

debug 副本 SHA-256 为
`e480ac1c35421a046779333b7e1940480bf6a240be5ae6f3ca0597bebebf499b`，
release 副本为
`85aaf6b5fcc0f1b936c0e90aa3f6a42116651f58ed27a464b09a9330e5e53c00`。
这些副本带 command ID 与 capture 时间，但**不证明**先前 QEMU 用的是
哪一枚 Kernel；它们只证明这四次 rework normal build。release 的
`85aaf6b5…` 是这四次新 build 的产物，不是从 PR16/PR17、
`rv/kernel-images/` 或任何历史 artifact 恢复的。

原 `rv/kernel-images/` 的“QEMU 前副本”声明作废：旧
`tools/artifacts.py` 会覆盖固定路径并无条件写
`copied_before_qemu=True`，因此原 release 声明的
`85aaf6b5…` 与当前文件
`dd518601c3305e3c18f7df7df4bdf44e2a9821b3e89765d10dc1110f445b5b37`
不一致；没有从历史 artifact 恢复旧文件。详细记录：
`evidence/pr18/rework/original-pre-qemu-copy-invalidity.md`。
旧的审计/汇总脚本已保留在 `rework/superseded/tools/`，当前
`tools/artifacts.py` 与 `tools/finalize.py` 拒绝运行。

## QEMU

八个新场景全部使用仓内固定的 `scripts/qtest.nu`，没有自定义 runner、
没有缩短 timeout、没有保护性 kill。普通四景未设
`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS`；四个 DMA confirmation 场景
显式设为 `true` 并使用既有 `semihosting,dma-confirmation-scene` 与
自动 `confirmation.py` 驱动。结果与独立判定见
`/home/anreonyr/Develop/shuyra/evidence/pr18/qemu/`（每个场景自己的
`run.log`、`exit`、console 与 `independent-judge.json`）。

普通帧数与 boot entry TID 按实际基线读取：normal `1`、debug `10`。
8/8 场景的实际读数：

| 场景 | SMP | frames | entry | exit |
| --- | ---: | --- | ---: | ---: |
| product / session | 4 | 85 → 85 | 1 | 0 |
| product / session | 8 | 153 → 153 | 1 | 0 |
| accept / session | 4 | 85 → 85 | 1 | 0 |
| iommu-product-fault | 4 | 85 → 85 | 1 | 0 |
| iommu-fail-invalidate | 4 | 87 → 87 | 10 | 0 |
| iommu-fail-reset | 4 | 87 → 87 | 10 | 0 |
| iommu-fail-release | 4 | 87 → 87 | 10 | 0 |
| iommu-fail-reference | 4 | 87 → 87 | 10 | 0 |

独立判定还核对了 IOMMU fault 的两条有序停链、`faults=127 last_did=8`、
sentinel 状态保持，以及 DMA 的 transport/reference 失败、同一 handle
在重试前仍持有、注入移除时 `references=2`、reference 最终
`references=1`、stop/release 与自然停机；没有用保护性 kill 作为证明。

这 8 个场景是**未改代码**上的既有结果；本次 rework 没有重跑
host/QEMU 矩阵。它们的 console/judge 仍可复核，但 prior QEMU 使用的
Kernel binary chronology 没有被新的 post-QEMU 副本证明；原来的
“QEMU 前副本”声明已按
`evidence/pr18/rework/original-pre-qemu-copy-invalidity.md` 撤回。

## 未验证边界

本阶段只完成 **device serialization API packages**。以下都不在本轮
完成或未验证：

- Kernel 仍不是新仓库；没有 Kernel 跨仓发布，也没有 Stage 3/4 的
  整体完成声明。
- IOMMU Hold 与 allocator 没有抽取、没有改归属；Kernel 的私有对象
  保留测试仍归 Kernel 且未改。
- Hold 与两个 API 包是**独立**的 sequencing domains：两者都在 Kernel
  抽取之前需要，但 API 包不是 Hold 的依赖，反之亦然。
- 没有 coordinator；root 负责最终 review/commit/push。
- QEMU 失败若出现应保留原现场并诊断基线；本轮不通过放宽断言或改
  codec 语义制造通过。
