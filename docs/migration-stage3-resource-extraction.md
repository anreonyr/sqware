# Stage 3: resource 公开提供方消费

## 来源与范围

- 提供方：<https://github.com/shuyra-arche/resource.git>
- 完整 revision：`dbc96834f50f094b5c386e54a0a2ae5560dcd7f8`
- 来源分支：`codex/shuyra-resource-extraction`
- 本仓检查点：`667f9635f21b2291563f06aaaf1a64cebd894aea`
- 本阶段只把 resource 从本仓源码成员改为公开 Git 依赖，并删除本仓的
  resource 实现；其他 provider 的 Git revision 与 registry 版本不变。

原 `crates/resource` 的 26 个 tracked 文件全部删除，根 workspace member
`"crates/resource"` 移除。该目录原来的 22 个 capability 测试、20 个
`src/*.rs` 与测试投影现在只归公开 resource 仓，本仓不复制第二实现。
`programs/tests/*` 中五个混合 suite 的 fixture Rust 与全部既有业务断言原样；
本仓本次没有修改任何 `.rs` 文件。

## 直接依赖映射（19）

14 处保留 `resource` 名称，直接引用公开包：

| 位置 | 分类 |
| --- | --- |
| `crates/ipc/Cargo.toml` | workspace dependency |
| `programs/Cargo.toml` | workspace dependency |
| `programs/src/driver/router/client/Cargo.toml` | client dependency |
| `programs/src/service/account/client/Cargo.toml` | client dependency |
| `programs/src/service/hub/client/Cargo.toml` | client dependency |
| `programs/src/service/pipe/client/Cargo.toml` | client dependency |
| `programs/src/system/client/Cargo.toml` | client dependency |
| `programs/src/user/shell/client/Cargo.toml` | client dependency |
| `programs/src/user/terminal/client/Cargo.toml` | client dependency |
| `programs/tests/lease-window/Cargo.toml` | suite dependency |
| `programs/tests/namespace-installation/Cargo.toml` | suite dependency |
| `programs/tests/namespace-layout/Cargo.toml` | suite dependency |
| `programs/tests/operator-client/Cargo.toml` | suite dependency |
| `programs/tests/supply/Cargo.toml` | suite dependency |

5 处保留 `resource_impl` alias 与 `package = "resource"`：

| 位置 | 分类 |
| --- | --- |
| `programs/tests/device-recovery/Cargo.toml` | dev-dependency alias |
| `programs/tests/iommu-pages/Cargo.toml` | dev-dependency alias |
| `programs/tests/operator-watch/Cargo.toml` | dependency alias |
| `programs/tests/pci-placement/Cargo.toml` | dev-dependency alias |
| `programs/tests/virtio-blk-coord/Cargo.toml` | dev-dependency alias |

所有 19 行都只把 `path = ".../crates/resource"` 换成：

```toml
git = "https://github.com/shuyra-arche/resource.git"
rev = "dbc96834f50f094b5c386e54a0a2ae5560dcd7f8"
```

`startup-authority` 没有直接 resource 行；它经 `crates/ipc` 间接消费，
其独立锁同样更新。

## 锁

实际用 `cargo metadata` 更新了 12 个 tracked lock。除 resource 包从
workspace path 变为公开 Git source 外，每个锁的非 resource package 条目
逐项相同；没有升级任何 registry 或 Git provider 版本。

| 锁 | packages | SHA-256 |
| --- | ---: | --- |
| `Cargo.lock` | 100 | `2af97d1b738c5d01c75d67855f8d03c33403ef529bcb3fe77395ac3db60b0a05` |
| `programs/tests/device-recovery/Cargo.lock` | 15 | `b893465646dc27b9486a48451f5e597dec677dd29683b18a094ccb6e66b3d1c9` |
| `programs/tests/iommu-pages/Cargo.lock` | 16 | `18aff3a478830c6338c20d593b175c98b6319d036c022650aa4d54a877c07657` |
| `programs/tests/lease-window/Cargo.lock` | 12 | `71595975308fe60e58b6a44ba7d3e97e7404106791b35825828c15ed09e96169` |
| `programs/tests/namespace-installation/Cargo.lock` | 13 | `750429fcddd802d6d8c075fd801b35e1756602ac78b5933dc7d7f6bc9a0a6ce6` |
| `programs/tests/namespace-layout/Cargo.lock` | 13 | `0759771e3331d958f1c8834fe01e9f866daf2be9b525b8d768934665d35def10` |
| `programs/tests/operator-client/Cargo.lock` | 13 | `c325843ea5b240ad8cc10f64fd176825649394c4cc1c3e94f440a59c0d559871` |
| `programs/tests/operator-watch/Cargo.lock` | 12 | `6d0b215a656a6b700bc9ef77d25ca1288dd604593ec7056e78bcbe2f955414f7` |
| `programs/tests/pci-placement/Cargo.lock` | 14 | `9fb724fdae1830ea81175adcef24038d6660cf61362d438dd30327f1ee6ebcde` |
| `programs/tests/startup-authority/Cargo.lock` | 15 | `9158b5593fa1d8a38e16c4f3c83db5975d0221e63fff416d14aa52201b7754fe` |
| `programs/tests/supply/Cargo.lock` | 14 | `a804788c3f72bbacd359e47c150280338e8827617a609803418c08dabf9ca4a8` |
| `programs/tests/virtio-blk-coord/Cargo.lock` | 16 | `846adf0c3cc0a408f7136cd3bf51d169f0d61e6cf8414a2e1c5f0cdceee607d8` |

12 个 `cargo metadata --locked`（含 `--offline`）图中，`resource` 的唯一
source 是完整 Git revision
`git+https://github.com/shuyra-arche/resource.git?rev=dbc96834f50f094b5c386e54a0a2ae5560dcd7f8#dbc96834f50f094b5c386e54a0a2ae5560dcd7f8`。
`env`、`wire`、`mold`、`system-api`、`unit` 的完整 SHA 与更新前相同。
QEMU 前、QEMU 后 12 个锁的 SHA-256 均未变化。

## 宿主验证

每个 suite 都显式 `--target x86_64-unknown-linux-gnu --locked`，随后再执行
实际 `--locked --offline`。原始退出码直接取 cargo 进程，不取 `tee`/`tail`
状态。

| suite | `--locked` | `--locked --offline` |
| --- | ---: | ---: |
| `device-recovery` | 8 passed, exit 0 | 8 passed, exit 0 |
| `iommu-pages` | 53 passed, exit 0 | 53 passed, exit 0 |
| `lease-window` | 3 passed, exit 0 | 3 passed, exit 0 |
| `namespace-installation` | 4 passed, exit 0 | 4 passed, exit 0 |
| `namespace-layout` | 18 passed, exit 0 | 18 passed, exit 0 |
| `operator-client` | 7 passed, exit 0 | 7 passed, exit 0 |
| `operator-watch` | 5 passed, exit 0 | 5 passed, exit 0 |
| `pci-placement` | 17 passed, exit 0 | 17 passed, exit 0 |
| `startup-authority` | 6 passed, exit 0 | 6 passed, exit 0 |
| `supply` | 23 passed, exit 0 | 23 passed, exit 0 |
| `virtio-blk-coord` | 20 passed, exit 0 | 20 passed, exit 0 |

11 套共 164 个测试，两轮均 0 failed。构建缓存只使用共享
`CARGO_TARGET_DIR=/home/anreonyr/Develop/shuyra/target/shared-build`，没有
改 `CARGO_HOME`、toolchain 或 source override。

## RISC-V 构建

以下命令均显式 RISC-V target、`--locked`；QEMU 阶段使用脚本默认的
`source/target`，显式取消 `CARGO_TARGET_DIR`。

| 命令 | 原始 exit |
| --- | ---: |
| `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked --offline` | 0 |
| `cargo build -p kernel --bin shuyra -p programs --bins --target riscv64gc-unknown-none-elf --locked` | 0 |
| 同上加 `--release` | 0 |

debug/release 各自产出 `target/riscv64gc-unknown-none-elf/<profile>/shuyra`
和 89 个可执行 `prog-*`；两个 profile 的 89 个 bin 名称一致。没有编译错误；
`programs`/`kernel` 仍有既有 warning，未为本次迁移消除或新增。

## QEMU 九景

九景逐场顺序执行，`CARGO_NET_OFFLINE=true`，`CARGO_TARGET_DIR` 未设置，
每景显式 `QEMU_SMP`、独立 `TRACE_OUT` 与 `QEMU_TEE_LOG`。非 IOMMU 景不带
额外 feature；四个 `iommu-*` 景沿用 `--features semihosting`。qtest 不支持
`--locked`，故在运行前后核对 12 个锁的 SHA-256，均未变化。

| 景 | 核数 | 输入 | exit | capture | feed result | entry/reason0 | frames |
| --- | ---: | --- | ---: | ---: | --- | --- | --- |
| `product` | 4 | `terminal/session.py` | 0 | 0 | ok | task 1 / `system: done` | 85 → 85 |
| `product` | 8 | `terminal/session.py` | 0 | 0 | ok | task 1 / `system: done` | 153 → 153 |
| `product` | 4 | `shell/smoke.py` | 0 | 0 | ok | task 1 / `system: done` | 85 → 85 |
| `accept` | 4 | `terminal/session.py` | 0 | 0 | ok | task 1 / `accept: done` | 85 → 85 |
| `system-fault` | 4 | EOF | 0 | 0 | n/a | task 1 / `system-fault: acceptance passed` | 85 → 85 |
| `iommu-product` | 4 | EOF | 0 | 0 | n/a | task 1 / `system: done` | 85 → 85 |
| `iommu-product-fault` | 4 | EOF | 0 | 0 | n/a | task 1 / `system: done` | 85 → 85 |
| `iommu-product-start-before` | 4 | EOF | 0 | 0 | n/a | task 1 / `system: done` | 85 → 85 |
| `iommu-product-start-after` | 4 | EOF | 0 | 0 | n/a | task 1 / `system: done` | 85 → 85 |

每景 console 都只有一条 `boot: entry task=1`，OpenSBI console 报
`Platform HART Count` 4，八核景报 8；每景有唯一 task 1 的
`exit ... reason=0x0`、自然 `task: all tasks exited, system halted`，且
`frame_allocations before == after`。fed 景的独立 `.result` 均为 `ok`；
每景 qtest 都报 `1 passed; 0 failed`。initrd 路径存在且非空，九个 console
均没有 `test scene requires an initrd` 哨兵。

`iommu-product-fault` 的真实停链保留为：执行域停 →
`virtio-blk` reset 队列停 → IOMMU queues stop（`faults=127 last_did=8`）→
IOMMU release（`frames=0 stopped=true`）；stale IOVA 的
`sentinel_held=true status_held=true` 保留。`start-before` 只有发布前中止，
没有 published 行；`start-after` 保留真实 published 行与发布后中止。二者都
自然收场并 `frames=0 stopped=true`。

## 未验证

- Stage 3 的其他组件与 Stage 4 pipe 迁移未完成。
- 未跑 1024 代长测、未跑完整 health 面、未做全树 `cargo fmt`。
- 未改冻结的 `docs/migration-baseline.*` 与历史阶段文档；没有新增 patch、
  replace、cache 投影、local 回链或全局配置。
- 本 worker 未 commit、push、创建仓库或 PR；等 root 审查稳定源码 checkpoint。
