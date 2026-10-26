# Stage 3: execution 消费者迁移到公开 provider

## Provider

- 仓库：<https://github.com/shuyra-arche/execution.git>
- 公开分支：`codex/source-snapshot`（默认分支）
- 完整提交：`1b333a28bc4c34beaea3f7986a9bb6e8ab2db414`
- 独立审查后的干净检出：`target/pr12-execution-public-clone`
- provider 根锁 SHA-256：
  `34fd235f03bd0c4af6690cd1d033c87c3eef99f3936b76612db97329014d70f5`

provider 由公开 source 的 `crates/execution` 提取。36 个 tracked 文件逐一到
新仓根；34 个逐字节一致，只有根 `Cargo.toml` 改为独立工作区和
`tests/.gitignore` 去掉 `Cargo.lock` 忽略。逐文件映射与 SHA-256 见 provider
仓库的 `MIGRATION-MAP.md` 和 `PROVENANCE.md`。

## 来源分支区分

- 公开 source 的 memory-boundary 分支是
  `codex/shuyra-execution-memory-boundary`（PR34 记录）；该边界的已确认
  基线 `6e9298d85ea744b733546512f56a5943f6977e9f` 是本次提取提交的祖先。
- 本次 PR12 的本地提取分支是 `codex/shuyra-execution-extraction`，冻结于
  `66bb8b5c342c50b141eb935a85efdfcecb91451b`。发布记录不把本地提取分支
  当作 provider 的公开 branch。

## 消费者依赖

只改六个 manifest，全部使用 canonical Git URL 和完整 rev
`1b333a28bc4c34beaea3f7986a9bb6e8ab2db414`：

| Manifest | 依赖 |
| --- | --- |
| `programs/Cargo.toml` | `execution` |
| `programs/src/system/client/Cargo.toml` | `execution` |
| `programs/src/user/shell/client/Cargo.toml` | `execution` |
| `programs/tests/pci-placement/Cargo.toml` | `execution_impl`（package `execution`） |
| `crates/image/tests/fixtures/link-workspace/first/Cargo.toml` | `execution` |
| `crates/image/tests/fixtures/link-workspace/second/Cargo.toml` | `execution` |

根 workspace 删除 `crates/execution` 成员，旧子树的 36 个 tracked 文件和
已迁出的实现整体移除，不保留第二份。PCI 仍是真实 normal dependency，没有
恢复 `#[path]` 源码投影或替代算法。没有改任何业务 `.rs`、断言或启动链接
脚本；`programs/build.rs` 和 Mold 合同未动。

## 三个消费者锁

只有三个消费者锁被更新；每个锁只把 `execution` 的 path 身份换成
Git 身份，包集合和版本不变：

| 锁 | 前 | 后 |
| --- | --- | --- |
| `Cargo.lock` | `db96275f...` | `432506aa9ac1b98f69a1e776a289ba9f73a8c48e59775529fd6a745a1e9a3b72` |
| `programs/tests/pci-placement/Cargo.lock` | `f7947bd5...` | `588abf2b99f57ad838e471314e11d77a134086eae4326a756a49aa730a01d4b0` |
| `crates/image/tests/fixtures/link-workspace/Cargo.lock` | `98e73370...` | `7b96f1b39ff390135693129297f262fb1353d8f81889ad8c7a4ec72624249d4f` |

根锁保持 `bitflags 2.13.1`、`unicode-ident 1.0.24`；PCI 与 link-workspace
锁各自保持 `bitflags 2.13.2`、`unicode-ident 1.0.27`。provider 的新根锁
没有触发消费者升级。`cargo metadata --locked --offline` 实际报告
`execution` 为
`git+https://github.com/shuyra-arche/execution.git?rev=1b333a28...#1b333a28...`；
`env@d3000253`、`mold@f52501f4`、`wire@0ff77dc4` 等既有 pin 未漂移。
`crates/image/Cargo.lock` 不是消费者锁，未修改；旧 `tests/*.lock` 随已迁出
目录移除，没有作为额外消费者。

## 验证结果

宿主测试使用 `CARGO_TARGET_DIR=target/shared-build/pr12-consumer`：

| 套件 | locked | locked + offline |
| --- | --- | --- |
| PCI placement | 17 passed, exit 0 | 17 passed, exit 0 |
| image core | 25 passed, exit 0 | 25 passed, exit 0 |

image 的 25 个是 18 个 lib + 3 个 catalog + 4 个真实 ELF 链接/特性夹具。
PCI 的 17 个原断言未改。

RV64 使用 source 默认 `target/`，检查与构建均为实际命令：

| 命令 | locked | locked + offline |
| --- | --- | --- |
| `cargo check --workspace --bins` | exit 0 | exit 0 |
| `cargo build -p kernel --bin shuyra -p programs --bins`，debug | exit 0，kernel 1 + programs 89 | exit 0，同 |
| 同上 `--release` | exit 0，kernel 1 + programs 89 | exit 0，同 |

90 个 release ELF 的 `_start`、`___rust_alloc`、`rust_begin_unwind`
最大计数均 1、重复 0；10 个程序不引用 allocator，未用 allocator 可裁剪。
`DEP_EXECUTION_LINKER_SCRIPT` 实际解析为
`/home/anreonyr/.cargo/git/checkouts/execution-017fae482aeeb313/1b333a2/link.ld`；
最终程序 ELF 的字符串也带同一 Git checkout 的 execution 源路径。正常
release build 的 kernel 已先复制为
`evidence/pr12/consumer/artifacts/kernel-release.elf`
（`85aaf6b5...`，1794656 bytes）。QEMU 阶段后观察到 top-level
`release/shuyra` 指向另一缓存产物（`84fc9cef...`）；具体触发切换的命令未追踪。
该文件不是正常 build 证据，正常 89 个程序 ELF 在所有
QEMU 景后哈希未变。scene 构建落在 `target/image/release/group-0`，与正常
build 目录分开。

## QEMU 九景

全部在最终源码上重跑。`CARGO_TARGET_DIR` 未设置，使用 `source/target`；
`CARGO_NET_OFFLINE=true`；`QEMU_SMP` 显式；每景独立 `TRACE_OUT`；
使用既有 `scripts/qtest.nu`、`scripts/qemu-args.nu`，未放宽 timeout/CPU/
final-reclaim，未跑 1024 代。

| 景 | SMP | 输入 | exit | capture | HART | frames |
| --- | ---: | --- | ---: | ---: | ---: | --- |
| product session | 4 | terminal/session.py | 0 | 0 | 4 | 85 → 85 |
| product session | 8 | terminal/session.py | 0 | 0 | 8 | 153 → 153 |
| product shell | 4 | shell/smoke.py | 0 | 0 | 4 | 85 → 85 |
| accept session | 4 | terminal/session.py | 0 | 0 | 4 | 85 → 85 |
| system-fault | 4 | EOF | 0 | 0 | 4 | 85 → 85 |
| iommu-product | 4 | EOF | 0 | 0 | 4 | 85 → 85 |
| iommu-product-fault | 4 | EOF | 0 | 0 | 4 | 85 → 85 |
| iommu-product-start-before | 4 | EOF | 0 | 0 | 4 | 85 → 85 |
| iommu-product-start-after | 4 | EOF | 0 | 0 | 4 | 85 → 85 |

四个 fed 景的独立 driver result 均为 `ok`。九景的 entry 均唯一为
`boot: entry task=1`，入口唯一 `reason=0x0`，并自然
`task: all tasks exited, system halted`。`iommu-product-fault` 的真实停链为：
binding 2 execution domain stopped → `status=0 queue_enable=0` →
`queue_enable=0 faults=127 last_did=8` → `frames=0 stopped=true`；
`ctl: stale iova submitted sentinel_held=true status_held=true`。
`iommu-product-start-before` 无 published 行，`start-after` 有 1 条
`iommu: binding 1 published ...`。本次九景均自然 halt；停链/回收证据为真实
domain-stop → device-reset → queues0 → release frames0 stopped true 日志，
未用保护性终止作证明。

## 未验证与边界

- 只完成 execution 的消费者迁移；Stage 3 其他组件和 Stage 4 pipe 仍未完成，
  不建 coordinator，不宣称总体完成。
- 未跑 1024 代 soak、默认 health 面或 debug IPI 自检；不宣称它们通过。
- 未跑全树 `cargo fmt`，未改既有历史 docs。
- 最终源码未提交；由 root 最终 review 接纳后处理。
- 证据：`evidence/pr12/consumer/`（原始 exit、Cargo JSON、ELF 哈希、
  QEMU 控制台与独立判定）。
