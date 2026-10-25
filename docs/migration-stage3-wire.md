# Stage 3: wire 迁出

## 来源与发布

- 来源仓库：<https://github.com/anreonyr/sqware>
- 来源提交：`6660f3d43a9d78b521542092da9ed891e36b38da`
- 来源分支：`codex/shuyra-wire-extraction`
- 迁出路径：`crates/wire`
- 独立仓库：<https://github.com/shuyra-arche/wire.git>
- 发布分支：`codex/source-snapshot`
- 发布完整 SHA：`0ff77dc4573fa2620d2fe995a18c01949d9c7e57`

## 消费者与删除

48 个 source manifest 的直接 `wire` 依赖已从本仓 path 改为上述 Git
URL 加完整 SHA。root `Cargo.toml` 移除了 `crates/wire` member，原
`crates/wire` 的 8 个 tracked 文件全部删除；没有 wrapper、复制实现或
源码跨仓投影。

原 crate 的 4 个源码文件和 `tests/host/lib.rs` 及其 5 个 host 测试随
独立仓发布，并已在 wire 仓库中验收。source 不再运行已删除的 wire host
测试。

## 锁

53 份 tracked 锁中 46 份含 `wire`。这 46 份由真实 Cargo 解析更新，
只有 `wire` 新增了 Git source；其余 package 的 name、version、source、
checksum 和依赖结构不变，既有 `mold` 仍唯一指向
`https://github.com/shuyra-arche/mold.git` 的
`f52501f4c91b686eded8b4bcd743b4e96084b7f9`。另外 7 份 tracked 锁不含
`wire`，未改动。

独立入口按 manifest 路径依赖闭包推导，共 62 个：60 个独立 workspace、
root workspace 和 `crates/image/tests/fixtures/link-workspace` 虚拟
fixture。相对上一阶段清单，去掉原 `crates/wire/tests/host`，加入
`crates/ipc/tests/session-establish`。62 个入口全部通过
`cargo metadata --format-version 1 --locked --offline`；每个图恰好解析
1 个 `wire`，source 均为上述完整 Git SHA。图内出现 `mold` 时也只有一个，
source 仍为上述 `f52501f…`。

`crates/ipc/tests/session-establish` 原先无锁；按原政策生成了 ignored
锁，仅新增该缺锁，原已有锁未重新生成。迁出的 wire host 也不再需要
source 锁。

## 验证

联网完成缓存准备后，同一组命令以 `--offline`（并设
`CARGO_NET_OFFLINE=true`）复验。两轮结果一致：

| 套件 | 联网 | 离线 |
| --- | ---: | ---: |
| `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked` | 0 | 0 |
| `crates/ipc/tests/host` | 20 passed | 20 passed |
| `crates/ipc/tests/session-establish` | 23 passed | 23 passed |
| `programs/tests/loader-api` | 6 passed | 6 passed |
| `programs/tests/system-shape` | 22 passed | 22 passed |
| `crates/image` | 25 passed | 25 passed |

host 测试合计每轮 96 passed、0 failed。`crates/image` 的 25 个用例为
18 个 lib、3 个 catalog、4 个 `second_package`；后者实际构建
`first`/`second` RISC-V ELF，覆盖多 feature 变体与指定 release。
root check 保留既有 2 条 `programs` warning；image catalog 保留既有 1
条 dead-code warning。没有删除断言、吞错或放宽判据。

## 未验证

- 其他 stage 3 组件没有迁移。
- stage 4 pipe 没做。
- 9 个 QEMU 景和 1024 长测没跑。
- 本阶段不是全绿声明；冻结的 `docs/migration-baseline.*` 和上一阶段
  `docs/migration-stage2.md`、`docs/migration-stage3-mold.md` 未改动。
