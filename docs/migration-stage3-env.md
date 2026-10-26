# Stage 3: env 消费者迁移

## 来源与发布

- 来源仓库：<https://github.com/anreonyr/sqware>
- 来源提交：`fb4be8eb5cf607fccef3aa4d696f1a24701edb34`
- 来源分支：`codex/shuyra-env-extraction`
- 迁出路径：`crates/env`
- 独立仓库：<https://github.com/shuyra-arche/env.git>
- 发布分支：`codex/source-snapshot`
- 发布完整 SHA：`d3000253cd5d13d466e91fafd4dbb27a5ec5aaae`

## 消费者与删除

83 个 source manifest 的直接 `env` 依赖已从本仓 path 改为上述 Git URL
加完整 SHA。其中 71 个使用 `env` 名，12 个使用 `abi = { package = "env", ... }`
别名；别名和其余字段均保留。另有两个动态生成 Cargo 的入口同步更换：

- `programs/src/system/identity/book/test-host.sh`
- `programs/src/harness/probe/identity-smoke.sh`

两者的生成依赖均为：

```toml
env = { git = "https://github.com/shuyra-arche/env.git", rev = "d3000253cd5d13d466e91fafd4dbb27a5ec5aaae" }
```

root `Cargo.toml` 移除了 `crates/env` member。原 `crates/env` 的 71 个
tracked 文件（62 个 `src/*.rs`、8 个 `tests/*.rs`、`Cargo.toml`）全部
删除；没有 wrapper、复制实现或跨仓源码投影。source 中不再保留 Env 的
第二份源码。

## shape 断言分拆

原 `programs/tests/system-shape/lib.rs` 的合并测试
`execution_mechanism_and_environment_marks_have_no_domain_dependencies`
拆为两半：

- source 保留 schedule 半：读 `crates/schedule/src` 并检查其 manifest
  不含 `protocol`，函数更名为
  `schedule_mechanism_has_no_domain_dependencies`。
- env 半已搬入独立仓的
  `tests/shape.rs::environment_marks_have_no_domain_dependencies`：读取
  `src/marks.rs`，经 `syn::parse_file` / `visit_file` 应用原四个前缀判据。

source 不再 `fs` 读取 EnvGit cache 源码。原 8 个 env 测试文件、40 个测试
全部随独立仓发布；新增 shape 成为第 9 个目标、第 41 个测试。source
`programs/tests/system-shape` 仍是 22 个测试，只有上述函数更名与
env 半移除。

## 锁

53 份 tracked 锁中 46 份含 `env`。这 46 份由真实 Cargo 解析更新，每份
只增加一行 `env` 的 Git `source`；其余 package 的 name、version、source、
checksum 和依赖结构不变，既有 `mold` 与 `wire` 仍唯一指向各自的既定
Git SHA。另外 7 份不含 `env` 的 tracked 锁字节未动。

15 份受 `env` 影响的 ignored 锁也按原政策用实际 Cargo 解析更新，没有
手写锁或伪造 source。46 + 15 共 61 个含 `env` 的 workspace 均有实际锁；
未新增缺锁入口。没有使用 `cargo update -p env`，锁只经离线 Cargo
metadata 最小解析更新。

入口由 manifest 路径依赖闭包推导：83 个直接 `env` manifest、87 个
消费者闭包，归并为 61 个 workspace；另加已改的
`programs/tests/system-shape`，共 62 个入口。相对上一 stage 3 wire 清单，
去掉不含 Env 的 `crates/ipc/tests/session-establish`，加入
`programs/tests/system-shape`。

62 个入口全部通过
`cargo metadata --format-version 1 --locked --offline`。61 个含 Env 的图
各恰好解析 1 个 `env`，source 精确为上述完整 Git SHA；shape 图不含
`env`。图内出现 `mold`/`wire` 时各只有一个，source 均为既有 SHA。

## 验证

联网完成缓存准备后，同一组命令以 `--offline` 复验；动态脚本本身已
`--offline`。

| 套件 | 联网 | 离线 |
| --- | ---: | ---: |
| `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked` | 0 | 0 |
| `crates/execution/tests` | 6 passed | 6 passed |
| `crates/execution/tests/unit` | 1 passed | 1 passed |
| `programs/tests/loader-api` | 6 passed | 6 passed |
| `programs/tests/system-shape` | 22 passed | 22 passed |
| `programs/tests/identity` | 19 passed | 19 passed |
| `crates/image` | 25 passed | 25 passed |

`crates/image` 的 25 个用例为 18 个 lib、3 个 catalog、4 个
`second_package`；后者实际构建 `first`/`second` RISC-V ELF，覆盖多
feature 变体与指定 release。root check 保留既有 2 条 `programs`
warning。没有删除断言、吞错或放宽判据。

## 动态 host 入口

`programs/src/system/identity/book/test-host.sh` 修复了两条已经失配的
`source.replace`：匹配 `host.rs` 当前实际注解
`#[path = "../../src/system/identity/service/answer.rs"]` 与
`#[path = "../../src/system/identity/book/mod.rs"]`，仍替换到
`here/../service/answer.rs` 与 `here/mod.rs` 的绝对路径；生成 Cargo
同步使用上述 Env Git 常量，未改 identity 核心、断言或参数接口。

历史基线在任何 Env 改动前 `rawexit=101`，失败原因是旧的
`#[path="mod.rs"]` / `../serve/answer.rs` 匹配不到当前注解。修复后
`sh programs/src/system/identity/book/test-host.sh` 实际 `exit=0`，
19 passed、0 failed。

`programs/src/harness/probe/identity-smoke.sh` 整体已知在冻结源码上为
`6!=9`，本阶段不重跑 QEMU、不把它冒充通过，也不改其判据或超时。只
执行其 Python 生成段，得到真实 temporary Cargo；实际
`cargo metadata --locked --offline` 解析到唯一 Env Git source，随后
`cargo check --target x86_64-unknown-linux-gnu --locked --offline`
编译 `identity-smoke-image` helper，`exit=0`。

## 未验证

- 其他 stage 3 组件和 stage 4 未迁移。
- 未全面验证其他依赖组合；动态临时 workspace 实际解析到
  `bitflags 2.13.2` 与 `unicode-ident 1.0.27`（tracked source 锁保留
  `2.13.1`/`1.0.24`），已通过本次 host 19 passed 与 helper `cargo check`
  exit 0。
- 未对最终新版本整机组合做 QEMU/长测，不外推全部组合。
- 9 个 QEMU 景和 1024 长测未跑。
- `identity-smoke.sh` 整体仍为已知 `6!=9`，不计为通过。
- 已发布的 env 41 个测试没有在本仓重跑；它们属于 provider 仓验证。
- 本阶段不是全绿声明；冻结的 `docs/migration-baseline.*` 和旧阶段
  migration 文档未改动，全树 fmt 未跑。
