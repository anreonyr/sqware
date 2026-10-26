# Stage 3: sbi 公开提供方消费

## 来源与范围

- 提供方：<https://github.com/shuyra-arche/sbi.git>
- 完整公开 revision：`2dc10bafb919eb2e8f6ca37c95cfd56f9a05831d`
- 来源快照：本仓 `c970afaf7dae1145633278253351be729b625717`，
  分支 `codex/shuyra-sbi-extraction`
- 来源公开上下文：`codex/shuyra-resource-extraction` 的 PR #32，
  <https://github.com/anreonyr/sqware/pull/32>
- 本地发布仓 `sbi/`：HEAD `2dc10bafb919eb2e8f6ca37c95cfd56f9a05831d`，
  clean；本阶段未改其内容。
- 公开 clone 与真实 RV64 locked/offline build 证据：
  `evidence/pr10/public-clone/summary.json`
- 提供方快照与本仓提取证据：`evidence/pr10/provider/`
- 本次消费方证据：`evidence/pr10/consumer/`

本阶段只把 `sbi` 从本仓源码成员改为公开 Git 依赖，并删除本仓实现。
`sbi` 是 RV64 S-mode ABI wrapper 基础库，不是 Unit；提供方不产出
UnitFile、ELF 或 server。这里构建的 Kernel `shuyra` ELF 是既有消费方
产物，不是新增的 `sbi` 产物。其他 provider 的 Git revision、registry
版本和宿主依赖不变。

## 六文件、一个消费者、一个锁

| 本仓原路径（`c970afaf`） | 公开提供方（`2dc10ba`） | 本仓动作 |
| --- | --- | --- |
| `crates/sbi/Cargo.toml` | 根 `Cargo.toml` | 删除；消费方不再保留第二份 |
| `crates/sbi/src/ecall.rs` | `src/ecall.rs` | 删除；原字节由公开 provider 持有 |
| `crates/sbi/src/eid.rs` | `src/eid.rs` | 删除；原字节由公开 provider 持有 |
| `crates/sbi/src/extension.rs` | `src/extension.rs` | 删除；原字节由公开 provider 持有 |
| `crates/sbi/src/fid.rs` | `src/fid.rs` | 删除；原字节由公开 provider 持有 |
| `crates/sbi/src/lib.rs` | `src/lib.rs` | 删除；原字节由公开 provider 持有 |

本仓唯一的消费边是 `kernel/Cargo.toml`：

```toml
sbi = { git = "https://github.com/shuyra-arche/sbi.git", rev = "2dc10bafb919eb2e8f6ca37c95cfd56f9a05831d" }
```

根 `Cargo.toml` 的 `"crates/sbi"` workspace member 已移除。唯一的锁是
根 `Cargo.lock`。`kernel/src` 与 `programs` 没有为这次迁移改任何 Rust
代码；`programs` 不直接依赖 `sbi`。

`SBI.bin` 是独立固件，不属于 `sbi` crate，也不属于公开提供方；本阶段
没有拷贝、移动或替换它。其 SHA-256 仍为
`c53d58aa9ab17f95fa1af159f0bf05fad5c5622ebc0f1084463435fa6e5752b6`。

## 锁

实际用 `cargo metadata --format-version 1 --offline` 解析完整 Git
revision，并由 Cargo 更新唯一根锁：

| | 值 |
| --- | --- |
| 更新前 | `2af97d1b738c5d01c75d67855f8d03c33403ef529bcb3fe77395ac3db60b0a05` |
| 更新后 | `db96275f12183fa02c9c4748e962287b0ae636414f4d397ef604019c3d449abb` |
| packages | 100 → 100 |
| 变更块 | 仅 `sbi 0.1.0` |

`sbi` 块只增加了一行完整 Git source：

```text
source = "git+https://github.com/shuyra-arche/sbi.git?rev=2dc10bafb919eb2e8f6ca37c95cfd56f9a05831d#2dc10bafb919eb2e8f6ca37c95cfd56f9a05831d"
```

所有非 `sbi` package 块逐项相同；没有新增、删除或升级任何 package，
没有改任何 registry checksum。`env`、`wire`、`mold`、`system-api`、
`unit`、`resource` 六个旧 Git pin 与更新前相同。实际
`cargo metadata --locked --offline` 中只有一个 `sbi` package，其 source
就是上面的完整公开 revision。

锁 hash 在更新、locked metadata、workspace check、Kernel debug/release
build 和五景 QEMU 前后都是
`db96275f12183fa02c9c4748e962287b0ae636414f4d397ef604019c3d449abb`。

## 构建

以下命令均从 `source/` 执行，显式 RV64 target、`--locked --offline`，
原始退出码直接取 cargo 进程：

| 命令 | 原始 exit |
| --- | ---: |
| `cargo metadata --locked --format-version 1 --offline` | 0 |
| `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked --offline` | 0 |
| `cargo build -p kernel --bin shuyra --target riscv64gc-unknown-none-elf --locked --offline --message-format=json` | 0 |
| 同上加 `--release` | 0 |

`--message-format=json` 的 `compiler-artifact` 记录：

| profile | sbi package_id | Kernel executable | SHA-256 |
| --- | --- | --- | --- |
| debug | `git+https://github.com/shuyra-arche/sbi.git?rev=2dc10bafb919eb2e8f6ca37c95cfd56f9a05831d#sbi@0.1.0` | `target/riscv64gc-unknown-none-elf/debug/shuyra` | `e480ac1c35421a046779333b7e1940480bf6a240be5ae6f3ca0597bebebf499b` |
| release | 同上 | `target/riscv64gc-unknown-none-elf/release/shuyra` | `85aaf6b5fcc0f1b936c0e90aa3f6a42116651f58ed27a464b09a9330e5e53c00` |

两个产物都是 `ELF 64-bit LSB executable, UCB RISC-V`，入口地址
`0x80200000`。本阶段没有为 `programs` 重复链接全部 89 个 bin；受影响
且实际消费 `sbi` 的是 Kernel。表中的 hash 是 normal standalone 构建
产物；QEMU 执行证据是另列的五景运行日志，不用这两个 ELF 的 hash 代替。

五景结束后观察到默认 release `shuyra` 路径的 hash 为
`84fc9cef...`；随后用正常 `cargo clean -p kernel --release` 和同一条
standalone release 命令重建，exit 0，复现了检查点产物 `85aaf6b5...`。
具体触发该路径变体的重建子命令未单独追踪，这里不作推断；两段 hash
和恢复命令都记在 `evidence/pr10/consumer/build/`。root 已把 normal
debug/release ELF 复制到
`evidence/pr10/consumer/root-artifacts/normal-{debug,release}.elf`，并记录
`root-artifacts/immutable-normal-builds.json`，避免后续 Cargo 输出覆盖。

## QEMU 五景

五景逐场顺序执行，`CARGO_TARGET_DIR` 未设置（沿用脚本默认的
`source/target`），`CARGO_NET_OFFLINE=true`，每景显式 `QEMU_SMP`、
独立 `TRACE_OUT` 与 `QEMU_TEE_LOG`。场景 runner 仍强制 release。

| 景 | 核数 | 输入 | nu exit | capture | feed result | HART Count | task 1 / reason | halt | frames |
| --- | ---: | --- | ---: | ---: | --- | ---: | --- | --- | --- |
| `product` | 4 | `terminal/session.py` | 0 | 0 | ok | 4 | `1 / 0x0` | 一次 | 85 → 85 |
| `product` | 8 | `terminal/session.py` | 0 | 0 | ok | 8 | `1 / 0x0` | 一次 | 153 → 153 |
| `product` | 4 | `shell/smoke.py` | 0 | 0 | ok | 4 | `1 / 0x0` | 一次 | 85 → 85 |
| `accept` | 4 | `terminal/session.py` | 0 | 0 | ok | 4 | `1 / 0x0` | 一次 | 85 → 85 |
| `system-fault` | 4 | EOF | 0 | 0 | n/a | 4 | `1 / 0x0` | 一次 | 85 → 85 |

每景 console 有唯一的 `boot: entry task=1`、唯一的 task 1 成功退出
证明和唯一的自然 `task: all tasks exited, system halted`；没有
`test scene requires an initrd` 哨兵。`product` 与 `accept` 的独立
`.result` 均为 `ok`。没有为通过而缩短 timeout、放宽帧断言、改变 CPU
条件或保护性终止；未跑 1024 代。未跑新的 IOMMU 场景，也没有用历史
IOMMU 运行冒充本轮结果。

## 未验证与边界

- Kernel IPI 自检只在 `cfg(debug_assertions)` 内存在，而场景 runner 强制
  release；本批五景**不能**用来声明 debug IPI 自检执行过。
- PMU、CPPC、NaCl 等本树未使用的 SBI 扩展未验证。
- `sbi` 不支持 x86 宿主，也没有 owned tests；本阶段没有做 host stub、假
  host 成功或把零 tests 当作覆盖。
- 公开提供方的 RV64 locked/offline build 与真实 Git 消费已由 root 的
  public clone 证据覆盖，本文件只引用，不重复冒充。
- Stage 3 其他组件与 Stage 4 pipe 迁移未完成，不宣称总体完成。
- 未改冻结的 `docs/migration-baseline.*` 与历史阶段文档；未做全树
  `cargo fmt`。

## 检查点

工作树只应有：

```text
M  Cargo.lock
M  Cargo.toml
D  crates/sbi/Cargo.toml
D  crates/sbi/src/ecall.rs
D  crates/sbi/src/eid.rs
D  crates/sbi/src/extension.rs
D  crates/sbi/src/fid.rs
D  crates/sbi/src/lib.rs
M  kernel/Cargo.toml
```

本 worker 未 commit、push、创建仓库或 PR；交回 root 审查的稳定未提交
checkpoint。
