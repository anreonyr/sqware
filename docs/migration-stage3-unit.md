# Stage 3: Unit 消费者迁移

## 来源与发布

- 来源仓库：<https://github.com/anreonyr/sqware>
- 来源提交：`a6c6824ef91ac487919d87374a71b62174d465bd`
- 来源分支：`codex/shuyra-unit-extraction`
- 迁出路径：`crates/unit`（34 个 `src/**/*.rs` 与 `Cargo.toml`）
- 独立仓库：<https://github.com/shuyra-arche/unit.git>
- 包名：`unit`
- 发布分支：`codex/source-snapshot`
- 发布完整 SHA：`90396f0fda376e0693fb1b9553aa21864d241057`

独立仓的 `tests/guard_selection.rs` 已在 provider 阶段实际执行
`guard_selection_rejects_unknown_duplicate_and_unstarted_names`，四个
`assert_eq!` 全部通过；本阶段不重跑该 provider 测试，也不把它的通过
算作 source 的测试数。

## 13 个直接消费者

7 个 normal 依赖逐文件改为：

```toml
unit = { git = "https://github.com/shuyra-arche/unit.git", rev = "90396f0fda376e0693fb1b9553aa21864d241057" }
```

| normal manifest |
| --- |
| `programs/Cargo.toml` |
| `crates/composition/Cargo.toml` |
| `crates/image/Cargo.toml` |
| `tools/shuyra-image/Cargo.toml` |
| `programs/src/user/shell/declaration/Cargo.toml` |
| `programs/src/driver/iommu/declaration/Cargo.toml` |
| `programs/src/driver/pci/declaration/Cargo.toml` |

6 个测试包保留原有 alias 名与 `package = "unit"`：

```toml
unit-declarations = { package = "unit", git = "https://github.com/shuyra-arche/unit.git", rev = "90396f0fda376e0693fb1b9553aa21864d241057" }
```

| alias manifest |
| --- |
| `programs/tests/startup-authority/Cargo.toml` |
| `programs/tests/operator-api/Cargo.toml` |
| `programs/tests/namespace-installation/Cargo.toml` |
| `programs/tests/daemon-control/Cargo.toml` |
| `programs/tests/control-publisher/Cargo.toml` |
| `programs/tests/publication-admission/Cargo.toml` |

root `Cargo.toml` 删除了 `crates/unit` member。原路径的 35 个 tracked
文件全部删除：34 个 `src/**/*.rs` 与 `Cargo.toml`。source 中不保留
Unit 源码、wrapper、复制实现、旧私有 path、跨仓源码投影或读 Git cache
源码的检查；没有 `source` override、`[patch]` 或 local 回链。

其余 foundation 仍唯一指向既有 full SHA，没有改动：

- System API (S1) `9e5d63ca536a176583cf1b0054b4ecf7ddd88257`
- Env `d3000253cd5d13d466e91fafd4dbb27a5ec5aaae`
- Wire `0ff77dc4573fa2620d2fe995a18c01949d9c7e57`
- Mold `f52501f4c91b686eded8b4bcd743b4e96084b7f9`

composition 中实际的 `UnitFile` / `PROGRAMS` / `DEPLOYMENTS` 值仍留在
source；`crates/image` 的 `#[cfg(test)]` composition 值也未搬入独立仓。

## Guard 测试归属

原 `programs/tests/guard-deployment` 的 16 个测试拆为：

| owner | 测试数 | 位置 |
| --- | ---: | --- |
| Unit provider | 1（4 个断言） | `<unit>/tests/guard_selection.rs` |
| source System 场景 | 15 | `programs/tests/guard-deployment/tests.rs` |

source 只删除首个 `guard_selection_rejects_unknown_duplicate_and_unstarted_names`
（原 `tests.rs:9-27`）及其分隔空行；其余 15 个测试与基线逐字节一致
（基线去掉该段后的 diff 为空）。同时删除：

- `lib.rs` 中指向 `crates/unit/src/guard_selection.rs` 的
  `pub mod guard_selection` include；
- `#[path = "fixtures/guard_selection.rs"] mod guard_selection_error`
  与 `pub use guard_selection_error::GuardSelectionError`；
- 只服务该首测的 `fixtures/guard_selection.rs` fake enum 文件。

未改其余算法、公共 API、系统场景夹具或断言；未把 fake `UnitFile`
适配成真实 Unit，也没有给 15 个测试新增 Unit 依赖。Guard 图不含
`unit` / `system-api`，只含 `env` / `wire` / `mold` 与本仓 `schedule`。

因此 Unit 消费图与 Unit 锁仍是 **15** 个；Guard 是改动后的额外
metadata 入口（共 16 个图），但它没有 Unit 依赖，不是新增第 16 个
Unit 消费锁。

## 锁

15 份含 Unit 的 tracked 锁由实际离线 `cargo metadata` 最小更新：

| lock | packages | 变化 |
| --- | ---: | --- |
| `Cargo.lock` | 100 | 仅 unit 增加 Git `source` |
| `crates/image/Cargo.lock` | 32 | 同上 |
| `tools/shuyra-image/Cargo.lock` | 33 | 同上 |
| `programs/tests/startup-authority/Cargo.lock` | 15 | 同上 |
| `programs/tests/operator-api/Cargo.lock` | 12 | 同上 |
| `programs/tests/namespace-installation/Cargo.lock` | 13 | 同上 |
| `programs/tests/daemon-control/Cargo.lock` | 13 | 同上 |
| `programs/tests/control-publisher/Cargo.lock` | 12 | 同上 |
| `programs/tests/publication-admission/Cargo.lock` | 12 | 同上 |
| `kernel/tests/dma-confirmation/Cargo.lock` | 15 | 同上 |
| `programs/tests/device-recovery/Cargo.lock` | 14 | 同上 |
| `programs/tests/iommu-pages/Cargo.lock` | 15 | 同上 |
| `programs/tests/pci-placement/Cargo.lock` | 13 | 同上 |
| `programs/tests/pci-scan/Cargo.lock` | 13 | 同上 |
| `programs/tests/virtio-blk-coord/Cargo.lock` | 15 | 同上 |

每份锁的 package 数量不变，除 `unit 0.1.0` 外每个 package 的
name/version/source/checksum/依赖结构逐项不变；四个既有 Git 源各唯一。
`programs/tests/guard-deployment/Cargo.lock` 不含 Unit，字节未动。
没有使用 `cargo update -p unit`，也没有手写锁或伪造 source。未进入其他
source workspace；`crates/image` 测试内嵌的 `link-workspace` 复用其
tracked 且未变的锁，其他 tracked/ignored 锁均未改动。

入口由 manifest/path/member 依赖闭包推导，而不是只枚举旧锁：

- root / `crates/image` / `tools/shuyra-image`；
- 6 个 alias 测试包；
- 6 个间接消费者：`kernel/tests/dma-confirmation`、
  `device-recovery`、`iommu-pages`、`pci-placement`、`pci-scan`、
  `virtio-blk-coord`（经 pci/iommu declaration path 闭包进入）。

15 个 Unit 图与 Guard 图共 16 个入口，全部通过实际
`cargo metadata --format-version 1 --locked --offline`。15 个 Unit 图
各恰好解析 1 个 Unit Git `90396f0f...`，且 S1/Env/Wire/Mold 各唯一为
上述 full SHA；Guard 图无 Unit、无 SystemAPI。

## 验证

共享 `CARGO_TARGET_DIR=/home/anreonyr/Develop/shuyra/target/shared-build`
只是构建缓存，不是源码覆盖。联网完成后，同一组命令以 `--offline`
复验；`crates/image` 的离线复验同时设置 `CARGO_NET_OFFLINE=true`，
使其嵌套 `cargo build --locked` 也真实离线。

| 套件 | 联网 | 离线 |
| --- | ---: | ---: |
| root `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked` | 0 | 0 |
| `programs/tests/startup-authority` | 6 passed | 6 passed |
| `programs/tests/operator-api` | 25 passed | 25 passed |
| `programs/tests/namespace-installation` | 4 passed | 4 passed |
| `programs/tests/daemon-control` | 14 passed | 14 passed |
| `programs/tests/control-publisher` | 11 passed | 11 passed |
| `programs/tests/publication-admission` | 3 passed | 3 passed |
| `programs/tests/guard-deployment` | 15 passed | 15 passed |
| `crates/image` | 25 passed | 25 passed |

每次 pass 的 host 测试为 78 个；加 image 的 25 个（18 lib、3 catalog、
4 `second_package`）共 103 个。`second_package` 实际构建 `first` /
`second` RISC-V ELF，覆盖多 feature 变体与指定 release。所有套件
`0 failed`，没有删除剩余断言、吞错或放宽 timeout/最终回收判据。
root check 只保留既有 2 条 `programs` warning。

## 未验证与边界

- 已发布的 Unit provider 测试没有在 source 重跑；其 1 个测试、4 个断言
  只由 provider 阶段证据持有。
- 其他 stage 3 组件、runtime/client 和 stage 4 未迁移。
- `tools/shuyra-image` 只经锁定 metadata 验证，未单独构建；它不在本轮
  要求的 host 测试清单内。
- 未跑 QEMU、1024 长测、全树 fmt；未改无关 source 行为。
- 未全面验证其他版本组合；本阶段不外推全部组合。
- 冻结的 `docs/migration-baseline.*` 与旧阶段 migration 文档未改动。
- 既有 baseline 哨兵与 identity-smoke 的已知限制维持，不把本阶段
  称为全绿。

本阶段证据在 parent `evidence/pr7/consumer/`：`verification.md`、
`summary.md` 与必要的 rawexit / metadata / byte 清单。
