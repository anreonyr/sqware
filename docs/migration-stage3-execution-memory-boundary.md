# Stage 3: execution 静态引用/映射内核边界

## 来源与范围

- 来源仓库：<https://github.com/anreonyr/sqware>
- 公开基线：`6e9298d85ea744b733546512f56a5943f6977e9f`（PR33）
- 分支：`codex/shuyra-execution-memory-boundary`
- 范围：只准备 `crates/execution` 的引用/映射机制边界及其 owner 测试、
  `programs/tests/pci-placement` 的跨 owner 依赖适配；不创建 execution 新仓，
  不迁出源码快照，不改已发布的 Env / Resource / Unit / Wire / Mold Git rev。

Capability 式的 Kernel 权威没有变：Kernel 持有并解析真实句柄和映射；
execution 只保存调用方仍需负责的本地记录。默认后端 `Native` 只做一次转发，
编码、寻址和失败词汇仍只有原 `memory` 函数那一份。

## 效果、权威与失败

| 机制 | 独立效果 | 状态权威 | 失败责任 |
| --- | --- | --- | --- |
| Reference | 取一份来源持有；显式释放同一句柄 | Kernel 持有来源与引用表；`Reference<K>` 只保存 `Option<FlatReference>` | acquire 失败不产生 owner；release 单次调用，成功才清 handle，失败保留同一句柄；已释放再调用返回 `Denied`；无 Drop、无 Busy 自动重试 |
| View | 建立一段映射；撤图后清身份 | Kernel 持有映射；`View<K>` 保存 team、`Option<ViewId>`、address、size 与同 K 的 pending Reference | `close` 先 detach，成功才清 id，再释放 pending；detach 失败不释放 pending 且保留 id；pending release 失败保留 pending，重试不再二次 detach；无 Drop |
| Attach | 在既有 Reference 上建立映射 | Kernel 的 Attach/Detach/SetAccess | 范围或 `count * PAGE_SIZE` 校验失败返回 `NotAligned`，不调用 Kernel；attach 失败不消费 Reference；成功返回同一个 K 的 View |
| SetAccess | 修改选定区间权限 | Kernel | Kernel 的错误 payload 原样返回 |

`Reference<K>` 的 `acquire` / `handle` / `release` 只要求 `ReferenceKernel`；
只有 `attach` 额外要求 `MappingKernel`。因此只持有来源的夹具无须实现假映射。
`View<K>` 的 pending 类型固定为 `Reference<K>`，不能换后端。

## 实现

- `memory/reference_kernel.rs`：公开静态 `ReferenceKernel`，只有
  `acquire` / `release`，签名沿用原 `memory` raw 函数。
- `memory/mapping_kernel.rs`：独立公开 `MappingKernel`，只有
  `attach` / `detach` / `set_access`，不继承引用 trait。
- `memory/native.rs`：`#[doc(hidden)] pub struct Native`，逐入口单次转发
  `memory::acquire/release/attach/detach/set_access`，没有第二套编码或重试。
- `memory/reference.rs`：`Reference<K>` 保存 `handle: Option<FlatReference>`，
  仅增加 `PhantomData<fn() -> K>`；成功释放才把 handle 置空。
- `memory/view.rs`：`View<K>` 保留 team / id / address / size / pending 五个字段；
  `pending: Option<Reference<K>>`；`close` 用 `?` 保住 detach 失败现场。
- `memory/mechanism.rs`：公开泛型机制模块，重导出 `Reference<K>`、`View<K>`。
- `memory.rs`：默认 `Reference` / `View` 是
  `mechanism::Reference<Native>` / `mechanism::View<Native>` 的具体别名，
  旧调用点仍按原有权具体签名推导。
- 没有 `unsafe impl Send/Sync`，没有 Drop，没有新的错误类型、后端对象、
  锁或全局状态。`FlatReference` 自带的 `PhantomData<*const ()>` 仍使
  默认别名和 `FixtureKernel` 泛型实例都是 `!Send + !Sync`。

## 宿主边界

旧整包 host 的实际失败是 `unit/tls.rs` 的裸 `mv ... tp`，
不是 `never_type`。本轮只把真实 RV64 运行时入口按目标架构限定：

- `tls::base`、`tls::deallocate`、`tls::bootstrap`、
  `unit::bootstrap`、`boot::bootstrap` 只在 `riscv64` 编入；
  宿主没有返回 0 或伪造分配成功的替身。
- `unit/task.rs` 的 trampoline 只在 RV64 调用 `tls::deallocate`；
  RV64 函数体本身没有改写。
- `_start`、panic handler、global allocator 仍各自按 RV64 唯一编入；
  `links = "execution"`、`build.rs`、`link.ld` 与
  `execution::boot::entry` 的合同未改。
- `boot::Report` / `Exit` 是纯值；`entry` / `finish` 只保留可 host 编译的返回适配，
  但二者会调用真实 Env/reap：native host runtime 最终遇到既有
  `env::ecall::trap` 的 unimplemented，不提供宿主成功替身，也不宣称 host task
  runtime 成功。`memory::current_task`、allocator、`map`、`attach_alloc` 没有一律
  gate；只有实际汇编入口被架构限定。

宿主行为验证只用 fixture Kernel 的泛型引用/映射机制：`execution` 作为 path
dependency 由 PCI 测试包实际编译并运行该 fixture，不拿 `cargo check` 当通过。
host 正常包的 build 只证明可编译；native host task runtime 未执行。

## PCI 适配

`programs/tests/pci-placement` 不再用 `#[path]` 投影
`crates/execution/src/memory/reference.rs`，改为正常依赖：

```toml
execution_impl = { package = "execution", path = "../../../crates/execution" }
```

`lib.rs` 公开
`Reference<FixtureKernel> = execution_impl::memory::mechanism::Reference<FixtureKernel>`；
`FixtureKernel` 只实现 `ReferenceKernel`，忠实转发既有
`state::acquire` / `state::release_reference`，不实现 `MappingKernel`。
旧的无使用假 `View`、假 `attach` 和本地 acquire/release 包装已删除。
`cleanup.rs` / `multibar.rs` 的 17 个测试正文与断言未改。

## Owner 测试

`crates/execution/tests` 保留原有 allocator/startup 测试，并通过同 owner 源码投影
编入新的生产 `reference.rs` / `view.rs` / 两个 Kernel trait。夹具状态只有一份，
内核适配只有 `FixtureKernel`。新增八个失败/边界测试：

- acquire 失败不产生 owner；
- release 失败保留同一 handle，重试成功后才清；
- 重复 release 返回 `Denied` 且不再调用 Kernel；
- drop 不自动 release；
- attach 非法范围/溢出返回 `NotAligned` 且不调用 Kernel；
- attach 失败保留 Reference 并返回原错误 payload；
- set_access 返回原错误 payload；
- detach 失败保留 id/pending 且不 release；
- detach 成功后 pending release 失败，重试只 release、不二次 detach。

这些测试通过 owner harness 的私有 `pending` 访问验证 View 合同，没有新增
test-only 公共 API，也没有另拆 feature/功能包。

显式 `cargo test -p execution --lib --target x86_64-unknown-linux-gnu --locked`
实测 raw exit 101：尽管 metadata 中 `test=false`，显式 `--lib` 仍编入
`allocator` 的 `#[cfg(test)]` 投影模块，而该模块依赖 owner harness 的夹具，
在 lib test 里不可执行。因此采用 owner harness 回退；这是既有测试分层，
不是生产机制失败。

## 验证

宿主命令显式 `--target x86_64-unknown-linux-gnu`，先 `--locked` 再
`--locked --offline`，退出码直接取 cargo 进程。

| 套件 | locked | locked/offline |
| --- | ---: | ---: |
| `programs/tests/pci-placement` | 17 passed, exit 0 | 17 passed, exit 0 |
| `crates/execution/tests` | 14 passed, exit 0 | 14 passed, exit 0 |
| `crates/execution/tests/unit` | 1 passed, exit 0 | 1 passed, exit 0 |
| `crates/image` | — | 25 passed, exit 0 |
| `execution` host 正常包 build | exit 0 | exit 0 |

PCI 的 17 个测试仍包含父引用释放失败保 `bar.reference`、同 local root、
不 grant、不重复 unseal，以及清理 parent 先 child 后。execution owner
14 个测试是原有 6 个加新增 8 个；unit owner 仍是原 1 个。
image 的 25 个是 18 个 lib + 3 个 catalog + 4 个真实 ELF 链接/特性夹具。

### ABI 与负 trait

旧证据保留在 `evidence/pr11/abi-baseline/`；本轮后测在
`evidence/pr11/abi-after/`，均在 `riscv64gc-unknown-none-elf` 上实测：

- `Reference` 16 bytes / align 8，`View` 56 bytes / align 8；
  `Reference<FixtureKernel>` / `View<FixtureKernel>` 同为 16/8、56/8。
- 旧具体签名 `Reference::acquire`、`Reference::handle`、`Reference::release`、
  `View::close` 仍可编译；新增 `attach` / `set_access` 签名也按原参数编译。
- `evidence/pr11/abi-after/` 的默认 aliases 4 个 Send/Sync 负探针原始日志同时含
  `E0277` 与 `E0603` collateral；原日志全部保留，不据此宣称干净负验证。
- 最终干净负验证以 root 独立 `evidence/pr11/root-abi-negative/summary.json` 为
  主证据：默认 aliases 与 `FixtureKernel` 泛型共 8 个 Send/Sync 探针，全部只有
  `E0277`、raw 101。ABI layout / 旧签名证据仍以 `abi-after/` 为准，未重跑。
  负 trait 的原因是 `FlatReference` 本身的 `PhantomData<*const ()>`，不是新增的
  `PhantomData<fn() -> K>`。

RV64/IR 使用 `CARGO_INCREMENTAL=0` 与 canonical 单份 IR；没有改动全局
Cargo 配置、Cargo home 或 rustup 工具链（`nightly-2026-09-04`）。

### RV64 构建

| 命令 | 原始 exit |
| --- | ---: |
| `cargo check --workspace --bins --target riscv64gc-unknown-none-elf --locked --offline` | 0 |
| `cargo build -p kernel --bin shuyra -p programs --bins --target riscv64gc-unknown-none-elf --locked` | 0，kernel + 89 bins |
| 同上加 `--release` | 0，kernel + 89 bins |

仅保留既有两条 `programs` warning。90 个 release ELF 中 `_start`、allocator
入口、panic 入口的最大计数均为 1，重复数为 0；build script 仍发布
`cargo::metadata=linker_script=.../crates/execution/link.ld`，由
`programs/build.rs` 读取 `DEP_EXECUTION_LINKER_SCRIPT`。

### QEMU 九景

全部在最终源码上重跑；QEMU 阶段取消 `CARGO_TARGET_DIR`，走默认
`source/target`，`CARGO_NET_OFFLINE=true`，每景独立 `TRACE_OUT`，显式
`QEMU_SMP`，输入脚本/EOF 独立。九景 qtest raw exit 均为 0，capture status 0。
`product` 4/8 核 console 分别报 4/8；其余四核景报 4。九景的 entry 均唯一为
`boot: entry task=1`，entry 的唯一 reason0 与各自完成句匹配；九景都自然
`task: all tasks exited, system halted`。

| 景 | SMP | 输入 | exit | capture | frames |
| --- | ---: | --- | ---: | ---: | --- |
| `product` session | 4 | terminal/session.py | 0 | 0 | 85 → 85 |
| `product` session | 8 | terminal/session.py | 0 | 0 | 153 → 153 |
| `product` shell | 4 | shell/smoke.py | 0 | 0 | 85 → 85 |
| `accept` session | 4 | terminal/session.py | 0 | 0 | 85 → 85 |
| `system-fault` | 4 | EOF | 0 | 0 | 85 → 85 |
| `iommu-product` | 4 | EOF | 0 | 0 | 85 → 85 |
| `iommu-product-fault` | 4 | EOF | 0 | 0 | 85 → 85 |
| `iommu-product-start-before` | 4 | EOF | 0 | 0 | 85 → 85 |
| `iommu-product-start-after` | 4 | EOF | 0 | 0 | 85 → 85 |

四个 fed 景的独立 driver result 均为 `ok`。`iommu-product-fault` 的现场保留
第二代 `faults=127 last_did=8`、`sentinel_held=true status_held=true`、
`release ... frames=0 stopped=true`，以及真实停链；没有把保护性终止当作
DMA 清场证明。`start-before` 没有已发布行，`start-after` 有真实已发布行；
两者都在 `settled after abort` 后收场。九景都没有放宽 timeout/CPU/回收，
没有跑 1024 代长测，也没有使用历史结果冒充本轮源码。

## 未验证与边界

- 未创建 execution 新仓，未做 Stage 4 源码快照迁出，不宣称总体完成。
- 未跑 1024 代 soak、默认 health 面、debug IPI 自检或其他已知边界；
  本轮 QEMU 覆盖范围是上述九个 release 景。
- 未跑全树 `cargo fmt`，未改冻结的 baseline/历史 docs。
- 未改已发布 env/resource/unit/wire/mold 的 Git rev；PCI lock 只新增
  execution 及其传递依赖，没有升级既有 pin。
- 最终源码保持未提交，交由 root review；本轮没有 `git commit` / `git push`。
