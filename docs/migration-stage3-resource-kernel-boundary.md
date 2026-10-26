# Stage 3: resource 静态内核边界与测试解耦

## 来源与范围

- 来源仓库：<https://github.com/anreonyr/sqware>
- 来源基线：`a1891de648de5e842507ce9684481228b642ba65`（PR30）
- 分支：`codex/shuyra-resource-test-boundary`
- 范围：只准备 `crates/resource` 的静态内核缝与五个混合 host 测试的公开资源依赖；
  不创建新的 resource 仓，不迁出源码快照，不改 Env / System / Unit / Wire / Mold
  的 Git rev，不引入 Cargo patch、source override 或 Git cache 源码投影。

Capability／Loan／Dock／Busy 仍只有一份真实实现。五个混合测试不再 `#[path]` 指向
`crates/resource/src`，而是以
`resource_impl = { package = "resource", path = ... }` 正常依赖，并用各自的
`Kernel` 单次调用适配落到原 `STATE`/`NATIVE`。

resource 自有的 capability host 套件仍是同 owner 投影，但它不再为了默认 `Dock`
ABI 引入假的 `pole::shut`。该套件使用自己的 capability-only `Native` 适配：只实现
`CapabilityKernel` 并忠实转发原 fake Env，不投影 production `native`/`mapping`/`dock`，
也不实现 `MappingKernel`。默认 `Dock`/`View` 的 ABI 由真实 package 探针
（root 的 `abi-current` 与本 PR 的 `abi-after`）覆盖，不依赖 fake mapping 成功。

## 效果、权威与失败

| 机制 | 独立效果 | 状态权威 | 失败责任 |
| --- | --- | --- | --- |
| Capability | 持有本任务创建的一枚能力；派生、授出、封印、释放 | Kernel 独占 token、权限、映射与资源生死；resource 只保存 `token`/`active` | release 单次调用；`Busy` 同参重试并 `starve`，其他错误原样返回；失败保留 token/active，Drop 继续同一 cleanup，不额外 seal |
| Loan | 借用来源 token，授给指定 peer；`keep` 移交远端责任 | 远端 token 由 Kernel 判决；resource 只记 peer/remote/active | revoke 单次调用；`Busy` 同参重试，其他错误原样返回；失败保留 peer/token/active，Drop best-effort 同一 cleanup |
| Dock | `open` 产生 base+size 的 `View`；`shut(self)` 撤图 | Kernel 的 Pole open/shut 与映射 | `shut` 消费 Dock；失败不保留 Dock；不自动 Drop、不 release token |
| Busy cleanup | 临时阻抗时让步后重试 | 唯一的 `cleanup` 泛型 | 只把 `K::is_busy` 当可重试；其他结果原样交出 |

`Native` 的 `release` 只调用一次
`env::pie::release(token, ReleaseMode::Revoke)`；`open` 保持
`VirtAddr::get()` 映射转换；`is_busy` 只认 `PieFail::Busy`；`starve` 仍是
`env::room::starve()`。Native 内没有第二套重试循环。

Driver Executor 的停链仍直接调用 `env::pole::shut(mapping.token)`，成功后
才清 `mapping`；没有改成 `mapping.take().unwrap().shut()`，失败保留映射是刻意
的同一句柄重试责任。

## 实现

- `kernel.rs`：`CapabilityKernel`，关联 `PieError`/`HoleError`/`PoleError`，
  九个单次入口：`unseal_hole`、`seal_hole`、`unseal_pole`、`derive`、`accord`、
  `release`、`revoke`、`is_busy`、`starve`。
- `mapping_kernel.rs`：独立 `MappingKernel`，只有关联 `Error` 与 `open`/`shut`；
  不继承能力 trait，映射夹具无须实现无用能力操作。
- `native.rs`：`#[doc(hidden)] pub struct Native`，实现两个 trait；错误型保持
  `erra::Error<env::PieFail/HoleFail/PoleFail>`。
- `cleanup.rs`：唯一 `cleanup<K: CapabilityKernel>`。Capability release/Drop、
  Loan revoke/Drop、默认 raw release/revoke 全部复用它。
- `capability.rs` / `loan.rs`：`Capability<K>`、`Loan<'a, K>` 保留原有
  token/active 与 `_source`/peer/remote/active，只增加
  `PhantomData<fn() -> K>`；参数、消费与借用方式不变，`grant` 返回同一个 `K`
  的 `Loan<'_, K>`。
- `mapping.rs` / `view.rs` / `dock.rs`：`Dock<K>` 保留 pie/view 且无 Drop；
  `View` 独立，字段私有、`Copy`/`Clone`/`Debug` 与只读 `base`/`size` 不变，
  构造保持 crate 内可见。`dock::Dock` 是
  `crate::mapping::Dock<crate::native::Native>` 的具体别名，`dock::View` 保持
  原路径。
- `raw.rs`：`Capability`/`Loan<'a>` 改为默认后端的具体别名；`Hole` 与 `Pies`
  按一文件一数据机械移到 `raw/hole.rs`、`raw/pies.rs`，`raw::Hole`、
  `raw::Pies`、`raw::pies()` 路径与消息等待/队列算法不变。
- `reply.rs`：改用默认 concrete alias；`ReplyError` 局部拆到
  `reply_error.rs`，`reply::ReplyError` 与 `port` 的重导出路径不变。

五个混合测试的适配：

| 测试 | 实现的 Kernel | 公开资源面 |
| --- | --- | --- |
| `virtio-blk-coord` | 仅 `MappingKernel`，`Error = PoleFail` | `dock::Dock` |
| `device-recovery` | 仅 `MappingKernel`，`Error = PoleFail` | `dock::Dock` |
| `iommu-pages` | 仅 `MappingKernel`，`Error = erra::Error<PoleFail>` | `dock::Dock` |
| `pci-placement` | 两个 trait；Pie/Hole/Pole 均为正式 Env wrapper | `capability::Capability`、`loan::Loan`、`dock::Dock` |
| `operator-watch` | `CapabilityKernel`；`PieError = ()`、`is_busy = false`、`starve` 为不可达断言 | `capability::Capability`、`loan::Loan` |

每个夹具只有一个 ZST `Kernel`，没有新增 shadow state。Mapping-only 夹具只实现
`open`/`shut` 两格。PCI 的 `fail_next_release` 仍是 `PieFail::Denied`；没有改
失败码、吞外部失败或放宽再试断言。

`operator-watch` 的 `HoleError`/`PoleError` 保持原
`erra::Error<abi::HoleFail/PoleFail>`，直接返回原函数结果，不用 `map_err` 只取
`source` 丢掉 payload。它的 `PieError` 仍是 `()`，`is_busy` 恒 `false`，故
`starve` 声明为不可达；没有发明第三套错误或静默空转。

resource capability 套件只保留有用 Cap/Loan 具体 alias、布局与同 `K` 借用编译门；
PhantomData-only 的 std trait 测试已删除，默认 `Dock`/`View` ABI 不再由 fake
harness 冒充。

## 验证

宿主命令统一显式 `--target x86_64-unknown-linux-gnu`，先 `--locked` 再
`--locked --offline`；共享
`CARGO_TARGET_DIR=/home/anreonyr/Develop/shuyra/target/shared-build` 只作缓存。
每条退出码直接取 cargo 进程，不取 `tee`/`tail` 管道状态。

| 套件 | `--locked` | `--locked --offline` |
| --- | ---: | ---: |
| `crates/resource/tests/capability` | 22 passed, exit 0 | 22 passed, exit 0 |
| `programs/tests/virtio-blk-coord` | 20 passed, exit 0 | 20 passed, exit 0 |
| `programs/tests/device-recovery` | 8 passed, exit 0 | 8 passed, exit 0 |
| `programs/tests/iommu-pages` | 53 passed, exit 0 | 53 passed, exit 0 |
| `programs/tests/pci-placement` | 17 passed, exit 0 | 17 passed, exit 0 |
| `programs/tests/operator-watch` | 5 passed, exit 0 | 5 passed, exit 0 |

六套共 125 个测试，`0 failed`。既有测试正文与断言未改；新增的只有 resource
capability 套件的 Cap/Loan 布局/具体 alias 编译门。

默认 package 的真实 ABI 由两个直接依赖 `source/crates/resource` 的探针覆盖：
root 的 `abi-current` 与 `abi-after`。二者都得到
`Capability`/`Loan`/`Dock`/`View` 的 64 位布局 16/32/24/16、对齐 8；
`Capability::derive`、`unseal_hole`、`unseal_pole`、`Dock::open`、
`Loan::accord` 旧具体签名可编译；`View` 为 `Send + Sync`，
`Capability`/`Loan`/`Dock` 仍因 `PieToken` 的指针 `PhantomData` 不是
`Send`/`Sync`，与基线一致。root `abi-current` 记录的资源机制指纹为
`8a20b213d0532a658c1a918fb44677dd2b4347fb2b272bd01f8c63d2dfc4c849`；
本轮 rework 只改测试适配与文档，未改 `crates/resource/src` 的生产机制。

源码构建（显式 target、`--locked`，退出码直接取 cargo 进程）：

| 命令 | 原始 exit |
| --- | ---: |
| `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked --offline` | 0 |
| `cargo build -p kernel --bin shuyra -p programs --bins --target riscv64gc-unknown-none-elf --locked` | 0，kernel + 89 个 programs bins debug |
| 同上传记加 `--release` | 0，kernel + 89 个 programs bins release |

仅保留既有 `programs` 两条 warning。

QEMU 九景在首次运行使用 `nu scripts/qtest.nu --package kernel --scene ...`，
QEMU 阶段取消共享 `CARGO_TARGET_DIR`，每景独立 `TRACE_OUT` 与
`QEMU_TEE_LOG`，显式 `QEMU_SMP`。rework 只改测试适配与文档，未改生产机制，
因此没有重复整机；运行时证据仍是首轮九景的原始 `qemu/` 目录。

| 景 | 核数 | 输入 | exit | capture | entry / reason0 | frames |
| --- | ---: | --- | ---: | ---: | --- | --- |
| `product` | 4 | `terminal/session.py` | 0 | 0 | task 1 / `system: done` | 85 → 85 |
| `product` | 8 | `terminal/session.py` | 0 | 0 | task 1 / `system: done` | 153 → 153 |
| `product` | 4 | `shell/smoke.py` | 0 | 0 | task 1 / `system: done` | 85 → 85 |
| `accept` | 4 | `terminal/session.py` | 0 | 0 | task 1 / `accept: done` | 85 → 85 |
| `system-fault` | 4 | EOF | 0 | 0 | task 1 / `system-fault: acceptance passed` | 85 → 85 |
| `iommu-product` | 4 | EOF | 0 | 0 | task 1 / `system: done` | 85 → 85 |
| `iommu-product-fault` | 4 | EOF | 0 | 0 | task 1 / `system: done` | 85 → 85 |
| `iommu-product-start-before` | 4 | EOF | 0 | 0 | task 1 / `system: done` | 85 → 85 |
| `iommu-product-start-after` | 4 | EOF | 0 | 0 | task 1 / `system: done` | 85 → 85 |

九景的实际运行 console 各自报 `Platform HART Count`：四核景为 4，八核 product 景为
8；这与各景显式 `QEMU_SMP` 一致。每景都保留唯一 `boot: entry task=1`、自然
`task: all tasks exited, system halted` 与严格
`frame_allocations before == after`。fed 景的独立
`scene-*-feed.log.result` 均为 `ok`。`qemu/hartcount/` 里修正后的 DTB 只作辅助，
不替代实际运行 console。

`iommu-product-fault` 保留真实停链与故障哨兵：第二代 `faults=127 last_did=8`、
`sentinel_held=true status_held=true`，停链顺序与 `frames=0 stopped=true` 断言
仍在。`start-before` 的发布前中止没有“已发布”行，`start-after` 有真实发布行；
两者都在 `settled after abort` 后收场。九景均自然退出 0，没有把保护性终止当作
清 DMA 证明。

## 未验证与边界

- 未创建或改写新的 resource 仓；Stage 4 与源码快照迁出仍未完成，不宣称总体完成。
- 未跑 1024 代长测、未跑默认 health 面（已知缺 initrd 的 scene 哨兵不算绿）、
  未跑裸 qtest Nu / idle / identity `6!=9` 等已知边界。
- 未改冻结的 `docs/migration-baseline.*` 与其他阶段文档；未做全树 `cargo fmt`。
- 未改已发布的 Unit / System / Env / Wire / Mold rev，也没有新的全局配置或密钥读取。
- QEMU 九景均为四/八核短程；不宣称更长时间或多版本组合。
- 额外 malformed firmware-only hartcount probe 已被 root SIGTERM 精确终止并记录在
  `root-probe-termination.json`，不计 runtime 证据。

首轮证据保留在父目录 `evidence/pr8/` 的 `host/`、`build/`、`qemu/` 与
`abi-after/`；本轮 rework 的真实退出码、console 重解析、依赖与 lock 检查在
`evidence/pr8/rework/`。
