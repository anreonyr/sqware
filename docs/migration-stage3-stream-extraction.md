# Stream 提取消费阶段

本阶段把 `stream` 从 `source` 的路径依赖切换到已发布的规范 Git 仓，保留 SPSC
header/ring 机制不变，并删除源码树内第二份实现。

## 源与 provider

- 源码仓: <https://github.com/anreonyr/sqware.git>
- 源码冻结 HEAD: `68e7afb7416f8008d5ed39cbc1daa53aa2b5d087`
- 本地分支: `codex/shuyra-stream-extraction`
- 规范 provider: <https://github.com/shuyra-arche/stream.git>
- provider 完整 revision: `ebd1c6db4823e28591650d967d7e8c8dd1939687`

两文件映射，均保持语义；`src/lib.rs` 为逐字节相同：

| 源码 | provider | SHA256 |
| --- | --- | --- |
| `crates/stream/Cargo.toml` | `Cargo.toml` | source `a4d4f25f…`；provider `ce0936cd…`，仅 workspace 继承改为显式字段 |
| `crates/stream/src/lib.rs` | `src/lib.rs` | `e661d726ee704c74e6f0bb532c54a5d8380aecfbe9fdece64a0613aa8b535711` |

`Header`/`Ring`、`MAGIC`、`VERSION`、容量、`repr(C, align(64))`、原子序、
部分进度、EOF drain、`BrokenPipe`、`Pending`、`Fail` 与全部断言均沿用 provider。
分配、映射、端点生命周期、唤醒、调度与重试仍由调用方持有；没有新增状态、锁、
包装、服务或 Unit 文件。

## 直接清单与锁

五个直接清单把 `stream` 从路径依赖改为
`https://github.com/shuyra-arche/stream.git` 的完整 revision：
`programs/Cargo.toml`、`programs/src/service/pipe/api/Cargo.toml`、
`programs/src/service/pipe/client/Cargo.toml`、
`programs/tests/shell/Cargo.toml`、`programs/tests/shell-adapter/Cargo.toml`。
根 workspace 移除 `crates/stream` member，并删除其 `Cargo.toml` 与
`src/lib.rs`；没有源码 Rust 测试或生产算法/结构调整。

五个受影响 tracked lock 为 `Cargo.lock`、`programs/tests/shell/Cargo.lock`、
`programs/tests/shell-adapter/Cargo.lock`、`crates/image/Cargo.lock`、
`tools/shuyra-image/Cargo.lock`。对照固定
`git show 68e7afb…` 的完整 package record multiset，每个 lock 只移除本地
`stream 0.1.0`、加入同一版本的规范 Git source record；其余 record 逐项不变。
52 个 tracked lock 中仅这 5 个变化，47 个逐字节不变；8 个 ignored 本地 lock
单独审计，全部不变。现有 IPC `29ad97dc…`、System API `9e5d63ca…` 等 pin
未动，`alloc-probe` vendor patch 未接触。

五个 workspace（根、shell 测试、shell-adapter 测试、image、shuyra-image）的
实际 metadata 均只有恰好一个 `stream` package，identity 为
`git+https://github.com/shuyra-arche/stream.git?rev=ebd1c6db…#ebd1c6db…`。
直接依赖与 `pipe-api -> stream` 边指向同一 identity。

## 验证

Host（`--target x86_64-unknown-linux-gnu`，locked 与 locked-offline 各一遍）：

| 命令 | tests |
| --- | ---: |
| `cargo test --manifest-path programs/tests/shell/Cargo.toml …` | 14 |
| `cargo test --manifest-path programs/tests/shell-adapter/Cargo.toml …` | 6 |
| `cargo test --manifest-path crates/image/Cargo.toml …` | 25（18 + 3 + 4） |

以上已含生产 inline 用例；两模式均 0 failed。`tools/shuyra-image` 的 host
library/CLI 实际 `cargo build` 两模式均 exit 0。

RV64 `cargo check --workspace --bins` 两模式 exit 0。
`cargo build -p kernel --bin shuyra -p programs --bins --message-format=json`
的 debug/release 均产生 1 kernel + 89 programs = 90 ELF；两 profile 各 90 个
唯一 hash，`_start` 90/90 唯一、`rust_begin_unwind` 90/90 唯一、alloc shim
至多 1 个（未用即 stripped）。locked/offline 的 artifact 集合一致。QEMU 前已
保存只读的 normal debug/release kernel；QEMU 后只记录 cache hash 观察，不再
重建。

QEMU 六景均为未改动的 `scripts/qtest.nu` 原脚本，`CARGO_NET_OFFLINE=true`、
`CARGO_TARGET_DIR` 未设、每景独立 trace，默认 60s 超时：

| 景 | HART | 输入 | exit/capture | entry | frames |
| --- | ---: | --- | --- | --- | --- |
| product | 4 | `terminal/session.py` | 0 / 0 | task 1 / `system: done` | 85 → 85 |
| product | 8 | `terminal/session.py` | 0 / 0 | task 1 / `system: done` | 153 → 153 |
| accept | 4 | `terminal/session.py` | 0 / 0 | task 1 / `accept: done` | 85 → 85 |
| iommu-product-fault | 4 | EOF | 0 / 0 | task 1 / `system: done` | 85 → 85 |
| product | 4 | `shell/smoke.py` | 0 / 0 | task 1 / `system: done` | 85 → 85 |
| product | 4 | `shell/edges.py` | 0 / 0 | task 1 / `system: done` | 85 → 85 |

六景都只有一个 `boot: entry task=1`、一个正确 reason 0 的 entry exit、
自然 halt，严格 frame 回收相等；五个 fed 景的独立 result 均为 `ok`。
`iommu-product-fault` 保留 binding 1/2 两条真实停链顺序：domain stopped →
device reset/status 0/queue 0 → IOMMU queues 0 → release `frames=0 stopped=true`；
故障侧 binding 2 保留 `faults=127 last_did=8` 与 sentinel
`sentinel_held=true status_held=true`。
保护性终止没有被当作 DMA 证明。

证据根固定为 `/home/anreonyr/Develop/shuyra/evidence/pr16/consumer`，位于源码
checkout 之外且不进入跟踪。原始命令、日志、退出码、五份 lock 全文/审计、五份
metadata、180 行 artifact 表、只读 kernel、六份 console 与 independent judge
均在该目录；汇总只引用它们，不复制完整表。

## 边界

Native host clock 仍不支持；缺 initrd 的 baseline health 仍不是全绿；
未跑 1024 代长测或全 health 面。本阶段不宣称 Stage 3/4 总体完成、协调器创建、
provider 重发布或 consumer 之外的 Unit/用户流改动。provider 仓未改动。
