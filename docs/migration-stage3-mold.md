# Shuyra 阶段 3：mold 消费者迁移检查点

本轮把 `mold` 从同仓 path 依赖切到已发布独立仓，并删除原实现。基线为
`2c17c3877e3f8c67d9db5634d693fa4303d47f8c`；发布仓
<https://github.com/shuyra-arche/mold.git> 的核验 ref 为
`f52501f4c91b686eded8b4bcd743b4e96084b7f9`（branch `codex/source-snapshot`）。
本文件只记录 mold 消费者阶段；其他阶段 3 组件与阶段 4 尚未完成。

## 1. 依赖切换

18 个 manifest 的 `mold` 依赖统一改为：

```toml
mold = { git = "https://github.com/shuyra-arche/mold.git", rev = "f52501f4c91b686eded8b4bcd743b4e96084b7f9" }
```

| # | manifest |
| ---: | --- |
| 1 | `crates/env/Cargo.toml` |
| 2 | `crates/wire/tests/host/Cargo.toml` |
| 3 | `crates/image/tests/fixtures/link-workspace/first/Cargo.toml` |
| 4 | `crates/image/tests/fixtures/link-workspace/second/Cargo.toml` |
| 5 | `kernel/tests/dma-confirmation/Cargo.toml` |
| 6 | `programs/Cargo.toml` |
| 7 | `programs/src/driver/router/api/Cargo.toml` |
| 8 | `programs/src/service/account/api/Cargo.toml` |
| 9 | `programs/src/service/hub/api/Cargo.toml` |
| 10 | `programs/src/service/pipe/api/Cargo.toml` |
| 11 | `programs/src/system/api/Cargo.toml` |
| 12 | `programs/src/user/terminal/api/Cargo.toml` |
| 13 | `programs/tests/device-recovery/Cargo.toml` |
| 14 | `programs/tests/iommu-pages/Cargo.toml` |
| 15 | `programs/tests/loader-api/Cargo.toml` |
| 16 | `programs/tests/pci-placement/Cargo.toml` |
| 17 | `programs/tests/pci-scan/Cargo.toml` |
| 18 | `programs/tests/virtio-blk-coord/Cargo.toml` |

根 `Cargo.toml` 已移除 `crates/mold` workspace member；`crates/mold` 的 8 个源文件与
旧 manifest 已删除，没有保留 wrapper、副本或第二实现。其他源码语义未改。

## 2. 锁与解析检查

- 原树 53 份 `Cargo.lock`，其中 46 份 tracked 锁含 `mold`；46 份全部用 Cargo 实际
  解析更新。每份锁只改变 `mold` 包的 `source`，其他包的 name/version/source/checksum
  未变。
- 所有 46 份锁的 `mold` 唯一 source 均为
  `git+https://github.com/shuyra-arche/mold.git?rev=f52501f4c91b686eded8b4bcd743b4e96084b7f9#f52501f4c91b686eded8b4bcd743b4e96084b7f9`。
- `crates/wire/tests/host` 与 `programs/tests/loader-api` 原无 tracked 锁，按原
  `.gitignore` 政策生成并保持 ignored；两者 `unicode-ident` 均为 `1.0.24`。
- 首轮 48 个入口（46 个 tracked 锁工作区 + 2 个 ignored 锁工作区）不是完整闭包：
  root DAG 审计发现另有 14 个独立 workspace 受 mold 传递影响，但原无锁、未纳入。
  旧 48 记录保留在 `evidence/pr3/consumer/previous-48/`，本轮已补齐。
- 这 14 个入口按 Cargo manifest 依赖闭包实际生成 ignored 锁（遵守原 `.gitignore`
  政策，不提交新锁；未改任何 tracked 锁或源码），`unicode-ident` 均为 `1.0.24`。
- 最终 62 个入口（60 个独立 workspace + root + virtual fixture）逐项跑
  `cargo metadata --locked --offline`：62/62 exit 0，均只解析出 1 个 `mold`，
  source 精确等于上述 Git URL + full SHA。首轮缺 14 的问题已修复，明细见
  `gap14-closure.tsv`、`metadata-check.json`。
- 未对 mold 使用 `[patch]`、path 回链、本地 Cargo source override 或 `local` 发布回链。
  唯一既有 `[patch.crates-io]` 在 `crates/alloc-probe`（`smartalloc-sys`），与本轮无关且未改。
- 没有批量 `cargo generate-lockfile` 升级既有锁；既有锁的其他 dependency 版本保留。

机器可读明细：`evidence/pr3/consumer/mold-lock-inventory.json`、
`metadata-check.json`。日志：`evidence/pr3/consumer/logs/`。

## 3. 构建与测试

均为显式 host target（除 root riscv check）并加 `--locked`；联网准备后同一组命令
用 `--offline` 复验。

| 命令 | 原 exit | 结果 |
| --- | ---: | --- |
| `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked` | 0 | 通过；保留 `programs` 2 条既有 warning |
| 同上 + `--offline` | 0 | 通过；同样 warning |
| `crates/wire/tests/host`: `cargo test --target x86_64-unknown-linux-gnu --locked` | 0 | 5 passed |
| 同上 + `--offline` | 0 | 5 passed |
| `programs/tests/loader-api`: 同上 | 0 | 6 passed |
| 同上 + `--offline` | 0 | 6 passed |
| `programs/tests/system-shape`: 同上 | 0 | 22 passed |
| 同上 + `--offline` | 0 | 22 passed |
| `crates/image`: 同上 | 0 | 25 passed（18 lib + 3 catalog + 4 second_package） |
| 同上 + `--offline` | 0 | 25 passed |

`crates/image` 的 4 个 `second_package` 测试实际调用嵌套 Cargo，构建
`link-workspace` 的 `first`/`second`，检查 riscv ELF 入口、release profile 与
`link-dep/loud` 多 feature 各自产物；两轮均通过。host 测试合计 58 passed。

本轮补 14 入口只生成 ignored 锁并跑 `cargo metadata`，不重跑首轮已通过的 58 项
host 测试，也不重跑 root check。

## 4. 未验证与限制

- 其他阶段 3 组件未做；阶段 4 pipe 独立迁移未做。
- 未重跑 9 个 QEMU 景与 1024 长测；本轮不改宏展开语义，stage 4 仍需完整新组合回归。
- PR1 既有 health 哨兵限制保留原样，未放宽或改写；不称全绿。
- 未重生成冻结的 `docs/migration-baseline.md` / `docs/migration-baseline.json`；
  未跑全树 `cargo fmt`；未改无关文件。

## 5. 检查点

| 项 | 值 |
| --- | --- |
| source base | `2c17c3877e3f8c67d9db5634d693fa4303d47f8c` |
| mold provider | `f52501f4c91b686eded8b4bcd743b4e96084b7f9` |
| mold 仓 | <https://github.com/shuyra-arche/mold.git> |
| branch | `codex/shuyra-mold-extraction` |
| 消费者 | 18 manifest；46 tracked 锁含 mold |
| metadata 入口 | 62 = 60 独立 workspace + root + virtual fixture；62/62 exit 0 |
| 首轮缺口 | 首轮 48 未覆盖完整闭包；补 14 个 ignored-lock 入口后为 62 |
| 本阶段 | 消费者迁移；commit/push/draft PR 由 root 处理 |
| 证据 | `evidence/pr3/consumer/`（首轮 48 在 `previous-48/`） |
