# Stage 3: System API 消费者迁移

## 来源与发布

- 来源仓库：<https://github.com/anreonyr/sqware>
- 来源提交：`e6302af2156a6fe3766d3ccc22f080a481948bf3`
- 来源分支：`codex/shuyra-system-api-extraction`
- 迁出路径：`programs/src/system/api`、`programs/tests/identity-api`、
  `programs/tests/loader-api`
- 独立仓库：<https://github.com/shuyra-arche/system.git>
- 包名：`system-api`（独立仓内路径 `api/`）
- 发布完整 SHA：`9e5d63ca536a176583cf1b0054b4ecf7ddd88257`

独立仓已经原样持有 41 个 API Rust 源文件、原 `identity-api` 的 9 个测试
和原 `loader-api` 的 6 个测试（含 `generation.rs`）。本阶段只做 source
消费者迁移，不复制回源码，也不保留 wrapper 或第二份实现。

## 消费者与删除

迁移前 source 有 55 个直接 `system-api` manifest；两个纯 API 测试包迁出
后剩 53 个：

| 位置 | 直接 manifest |
| --- | ---: |
| `programs/Cargo.toml` | 1 |
| `programs/src/**/Cargo.toml` | 13 |
| `programs/tests/**/Cargo.toml` | 35 |
| `crates/**/Cargo.toml` | 4 |
| 合计 | 53 |

53 个 manifest 的 `system-api` 依赖全部逐文件改为：

```toml
system-api = { git = "https://github.com/shuyra-arche/system.git", rev = "9e5d63ca536a176583cf1b0054b4ecf7ddd88257" }
```

没有使用别名，没有保留 path 作为备选，也没有手写第二来源。其他字段
未改；`env` / `wire` / `mold` 仍唯一指向既有 full SHA：

- `env` `d3000253cd5d13d466e91fafd4dbb27a5ec5aaae`
- `wire` `0ff77dc4573fa2620d2fe995a18c01949d9c7e57`
- `mold` `f52501f4c91b686eded8b4bcd743b4e96084b7f9`

root `Cargo.toml` 删除了 `programs/src/system/api` member。共删除 49 个
tracked 文件：

- `programs/src/system/api` 42 个：41 个 `src/**/*.rs` 与 `Cargo.toml`。
- `programs/tests/identity-api` 3 个 tracked 文件。
- `programs/tests/loader-api` 4 个 tracked 文件。

两个测试目录内被忽略的 `Cargo.lock` 随目录一并删除，不再作为 source
入口。source 中不保留 API 源码、测试、wrapper、复制实现或跨仓源码投影。

## 4 个结构断言

原 source 的四个 API 结构判据已由公开仓的 `api/tests/shape.rs` 持有，
本阶段只移除旧对应片段：

| source 旧断言 | 新 owner | 新测试 |
| --- | --- | --- |
| `system-shape/lib.rs:562-564`：API manifest 只允许 `env`/`wire`/`mold` | `system/api/tests/shape.rs` | `provider_api_keeps_one_way_dependencies` |
| `system-shape/lib.rs:775`：API 无 `src/control/publication.rs` | 同上 | `api_has_no_control_publication_module` |
| `operator-api/lib.rs:481-483`：API `operator/mod.rs` 无 `pub enum Grant` | 同上 | `sdk_separates_read_discovery_from_entry_capabilities` |
| `operator-api/lib.rs:481-483`：API `operator/mod.rs` 无 `mod grant` | 同上 | 同上 |

只删除了这些旧片段，未删整个 mixed test。`provider_api_and_public_clients_keep_one_way_dependencies`
仍保留 client、terminal、router、hub 等检查；
`account_has_its_own_task_and_api_and_uses_authorized_control_contract`
仍保留 account、control、operator、client、launch、material 等检查；
`sdk_separates_read_discovery_from_entry_capabilities` 仍保留
client/Capability/Rein/granted_berth 检查。registry APIS/LOADER 组合检查
仍归 source，没有搬到 API 仓测试。source 不再通过文件系统读取 Git cache
中的 API 源码，删除目录也不会让空检查自然通过。

## 锁

source 共 53 份 tracked 锁。其中 38 份含 `system-api`，由实际离线
`cargo metadata` 最小更新；每份只增加一行 `system-api` 的 Git `source`，
package 的 name、version、source、checksum 和依赖结构不变。其余 15 份
tracked 锁不含 `system-api`，字节未动。

迁移前 16 份 ignored 锁中有 11 份含 `system-api`；删除两个已迁测试包后
剩 9 份。9 份 ignored 锁按原政策用实际 Cargo 解析更新，仍保持 ignored；
另 5 份不含 `system-api` 的 ignored 锁未动。38 + 9 共 47 个含
`system-api` 的 workspace。

47 份锁中 `system-api` 都恰好一个，source 精确为上述完整 Git SHA；
`env`、`wire`、`mold` 各自唯一，source 仍为既定 full SHA。没有使用
`cargo update -p system-api`，也没有手写锁或伪造 source。

入口由 manifest/path/workspace member DAG 推导，而不是只扫旧锁：53 个
直接 manifest、64 个消费者闭包归并为 47 个 workspace。另加已改但本就不含 SystemAPI 的
`programs/tests/system-shape`，共 48 个入口。48 个入口全部通过
`cargo metadata --format-version 1 --locked --offline`；shape 图不含
`system-api`。DAG 推导出的 47 个 workspace 与 47 个实际锁图逐项一致。

## 动态 host 入口

`programs/src/system/identity/book/test-host.sh` 的本地 API 覆盖入口已
移除：

- 删除 `api_root` 与 Python 的 `api` 参数，不再接受另一份 API checkout。
- 生成的 `system-api` 固定为上述公开 Git URL 与完整 SHA。
- 任何位置参数都打印 usage 并以 2 退出，不静默忽略。
- 保留原有两条绝对模块路径替换、`Env` Git `d300025` 常量，以及测试
  core/断言。

脚本本身为 tracked mode `100644`，按原调用方式 `sh` 执行。默认无参数
实际 `exit=0`，`19 passed`、`0 failed`；带一个位置参数实际 `exit=2`，
输出 `usage: ... (no arguments)`。该调整只移除 local API override，不改
产品 API/ABI。

## 验证

联网完成缓存准备后，同一组命令以 `--offline` 复验。共享
`CARGO_TARGET_DIR=/home/anreonyr/Develop/shuyra/target/shared-build`，它
只是构建缓存，不是源码 override。

| 套件 | 联网 | 离线 |
| --- | ---: | ---: |
| `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked` | 0 | 0 |
| `programs/tests/operator-api` | 25 passed | 25 passed |
| `programs/tests/control-api` | 10 passed | 10 passed |
| `programs/tests/device-api` | 5 passed | 5 passed |
| `programs/tests/interface-marks` | 5 passed | 5 passed |
| `programs/tests/system-shape` | 22 passed | 22 passed |
| `programs/tests/identity` | 19 passed | 19 passed |
| `crates/image` | 25 passed | 25 passed |
| `programs/tests/guard-deployment` | 16 passed | 16 passed |

`crates/image` 的 25 个用例为 18 个 lib、3 个 catalog、4 个
`second_package`；后者实际构建 `first`/`second` RISC-V ELF，覆盖多
feature 变体与指定 release。root check 保留既有 2 条 `programs`
warning。本轮 8 套件与 root check 没有真实测试失败。没有删除断言、
吞错或放宽 timeout/最终回收断言。

## 未验证与边界

- System runtime/client 未迁，unit 尚未独立；不能称为 stage 3/4 完成。
- 其他版本组合未全面验证。动态临时 workspace 实际解析到
  `bitflags 2.13.2` 与 `unicode-ident 1.0.27`；tracked source 锁保留
  `2.13.1` / `1.0.24`。这些临时解析组合已由本次 host 19 passed 覆盖，
  但不外推全部组合。
- 未重跑 provider 仓的 18 个测试；它们属于已发布 API 仓验证。
- 未跑 QEMU、1024 长测、全树 fmt；未改无关 source 行为。
- 冻结的 `docs/migration-baseline.*` 和旧阶段 migration 文档未改动。

本阶段证据在 parent `evidence/pr6/consumer/`：`verification.md`、
`summary.json` 与必要的 exit/清单。
