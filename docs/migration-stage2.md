# Shuyra 阶段 2：组合解耦检查点

本文件记录阶段 2 返工后的最终边界、真实验证、限制与检查点。基线为 PR1
`fdc8c3c26d70c6a7f66ae9fce772e7cce81c0de4`（PR24）；首轮检查点已封存在
`target/migration-evidence/pr2/first-pass-checkpoint`。本文保存阶段 2 提交前的验收记录。

## 1. 最终边界

### 1.1 供给传输唯一化（system-api）

`SupplyFrame`、`descriptor_mark`、`IMAGE_MARK` 在
`programs/src/system/api/src/supply/{frame,marks}.rs`。字节长度仍为
`2 + MAX_DESCRIPTION`，salt 与旋转位数未改；所有消费者引用 `system-api::supply`。
`Grant`/`Subject`/`NamespaceKey`/`Ops`/`Permit`/`supply::Binding` 仍是 system-api 原类型，
无复制、包装或布局变化；system-api 依赖仍只有 `env`/`wire`/`mold`。

### 1.2 纯 `unit` 包

`crates/unit`（`#![no_std]`，依赖 `env`、`system-api`）承载 UnitFile 配置类型、Deployment、
排序/guard/validation，以及 **`ImageBuild`**。

- `ImageBuild` 在 `crates/unit/src/image_build.rs`：image/manifest/package/bin/features/
  target/profile/default_features，全是 static 字符串/slice；无 std、无 `PathBuf`、无
  composition 依赖。
- 每个数据类型独立文件；查询显式接收声明 slice：
  `select_deployment(deployments, units, name, entry)`、`Deployment::find`、
  `Deployment::entry_image(units)`、`Deployment::guarded(units)`、`order_scene`。
- 不包含 `PROGRAMS`/`DEPLOYMENTS`/`IMAGE_BUILDS` 全表，不依赖 execution/server/client。

### 1.3 声明值与 image 构建值

`crates/composition`（no_std）只持有剩余实际值：

- `units/`：实际 UnitFile 声明与 `PROGRAMS`；`deployments.rs`：`DEPLOYMENTS`。
- `image_builds.rs`：`IMAGE_BUILDS: &[ImageBuild]`，`image` 是唯一镜像名，`bin` 由
  `concat!("prog-", image)` 在同一处派生；不再有 `Package`/`PACKAGES` 第二份 image/bin 表。
- Shell / IOMMU / PCI 的声明值仍归各自声明包；Hub/router 常量归各自 API；运行失败号归
  runtime。命令镜像仍是独立供给，没有补造 UnitFile。
- `fixturetarget-api` 与 `rtc-api` 是所属 host-safe no_std 纯常量包（原 `api.rs` 常量原样
  移入；旧 `api.rs` 仅 `pub use ...::*`）。fixturetarget 声明使用
  `fixturetarget_api::ENTRY_MARK`/`ENTRY_NAME`；RTC 声明使用 `rtc_api::ENTRY_NAME`，binding
  mark 仍为原 `router_api::ENTRY_MARK`，不新增 RTC ENTRY_MARK。两个包均登记为 root workspace
  member，programs 与 composition 正常依赖。

### 1.4 System 显式输入

- System 入口适配层只领取一次 `Boot`，按 catalog 选择并校验 `Deployment`，把 Boot、借用
  声明 slice、Deployment 交给 server 主体；`execute::run`、`install::resources`、
  `scene::programs`、`config`、`operator`、`supplies` 均显式接收同一份输入。
- `assembly::input` 与动态构建只保存 `Declarations` 引用；`Jobs` 不复制全表、不加锁。
- `assembly/static` 三处全局查询读取当前 `Operation` 携带的声明引用。
- `bootstrap` 不引用 `unit::system::E_BOOT`；boot 失败号归 server。
- `Supplies::new` 遍历 `Catalog::entries` 时，遇到未通过 `Catalog::new` 校验的记录沿
  `"image manifest entry"` 错误传播，不再 `Err => continue` 静默跳过。InitReceipt 的
  source.shutdown → release authority → roots retry 顺序与既有 lease/授权行为不变。

### 1.5 execution 启动与链接

- `execution` 唯一提供 `_start`、TLS bootstrap、无分配 panic；`programs/src/entry.rs` 已删。
- `mold` 的 `#[entry]` 展开调用 `execution::boot::entry`。
- `crates/execution/link.ld` 由 execution 拥有；`links = "execution"` + build metadata
  `linker_script`，programs 与 fixture 的 build.rs 读取 `DEP_EXECUTION_LINKER_SCRIPT` 给自身
  ELF 传 `-T`。库自身不发 `rustc-link-arg`。
- 启动/panic/allocator 限 riscv64；宿主执行库测试无重复 provider。

### 1.6 image-builder core / CLI

`crates/image` 是纯 host library：

- 正常依赖只有 `env`、`unit`、`system-api`、`elf`、`serde_json`；**不含 composition**。
  `composition` 仅在 `[dev-dependencies]` 作为既有集成断言的测试数据。
- 正常代码没有 `scenes`/`deployment_of`/`bins_for`/默认 build，也不从
  `CARGO_MANIFEST_DIR/../..` 猜 Integration 根。
- `build_with(root, work, output, declarations, deployment, builds)` 与
  `executables(root, work, builds)` 显式接收构建根、nested target 目录/输出路径、UnitFile
  slice、已选 Deployment、ImageBuild slice。
- 闭包库存以 `ImageBuild.image` 识别镜像；不再依赖 `Package`/`PACKAGES`。
- 嵌套 Cargo 使用 `--locked`；按 `cargo metadata` 的真实 `package_id`、Cargo JSON 的
  `target.kind`/`target.name`、非 test、唯一 `executable`、ELF magic 选择产物。
- 已删除两段误拒绝：由 `release`/`debug` 名字推断 `debug_assertions`，以及 requested
  features 逐字 subset 匹配 artifact.features。Cargo 参数仍是请求权威；resolved 的
  package/bin/target/profile/features/executable 打印记录。

Integration CLI 在 `tools/shuyra-image`（独立 host 包，依赖 builder + composition）：

- 选择 scene/profile、解析 Integration build root、把 `IMAGE_BUILDS` 的 profile 用 struct
  update 改成请求档、调用 core。
- `.cargo/config.toml` 的 `cargo image` alias 指向
  `tools/shuyra-image/Cargo.toml`；保留 `cargo image scene profile` 与最后
  `ok <绝对路径>`。

### 1.7 fixture 与缓存

- 多包链接 fixture 从 `crates/execution/tests/link` 移到
  `crates/image/tests/fixtures/link-workspace`，属于 image-builder 测试资产；正常 path 依赖
  `execution`/`mold`/`env`，没有跨包源码投影或复制运行实现。
- fixture 含 `first`、`second`、`link-dep`；`second` 的 `mold` 入口返回
  `link_dep::NOTE`，`link-dep` 按 `loud` feature 给出真实 `Report.note` 使用的不同
  rodata 常量；workspace 定义 `[profile.release] debug-assertions=true`，并保留
  `release-check`（inherits release）。独立无 programs 入口 ELF 证明保留。
- `.gitignore` 增加 `**/target/`，覆盖深层独立测试/fixture 工作区的 target；实际
  `Cargo.lock`、源码和证据保留。

### 1.8 产物覆盖缺陷与 group 隔离

Root 用真实 Cargo 复现：两轮 raw exit 均 0，但 plain/loud 同 package/bin/profile/target、
不同 features 返回同一 `second` executable 路径；先 plain sha256 `4b29ddce...`，后 loud
覆盖为 `3ac9708d...`，`overwritten=true`（`target/migration-evidence/pr2/variant-probe/results.json`）。

最小修复：`executables` 与 `build_with` 两处 `groups(&picked)` 都 `enumerate`，每个 group
用 `work.join(format!("group-{index}"))` 作自己的 nested target-dir；仍以 Cargo JSON 的真实
`compiler-artifact.executable` 为准。父 Cargo target 与 nested target 仍分离，不猜路径、不建
global registry/lock/framework。

新增真实回归：

- `same_bin_with_different_features_keeps_both_real_el_artifacts`：同一次 `executables`
  调用放两个不同 image 名、同 manifest/package/bin/profile/target，features 分别为空与
  `link-dep/loud`；全部构建后分别读两份 ELF，断言 plain 只含 `link-dep-plain-note`、loud
  只含 `link-dep-loud-note`，且 bytes 不同。
- `build_with_packs_each_feature_variant_from_its_own_artifact`：同一 `build_with` 打包
  `first` 与两个 variant 资源镜像，读回 manifest 条目核对各自专属 bytes。
- `release_with_debug_assertions_and_dependency_feature_request` 现使用字面
  `profile="release"` 且 `features=["link-dep/loud"]`；真实 Cargo artifact profile 为
  `release` + `debug_assertions=true`，features 列表为空（依赖 feature 请求）。该测试成功
  同时防回归旧的“release 必须 debug_assertions=false”和“requested feature 必须逐字出现在
  artifact.features”两段误拒绝。

## 2. 实际验证（返工后源码）

命令均在当前工作树执行；QEMU 使用 `CARGO_NET_OFFLINE=true`，nested Cargo 继承同一环境。
证据：`target/migration-evidence/pr2/`（`commands-rework.log`、`dependency-trees-rework.txt`、
`variant-regression.log`、`api-packages.log`、`qemu-variant-final/`、`judge-variant.log`）。

| 验证 | 结果 |
| --- | --- |
| `cargo check --workspace --locked` | exit 0 |
| `cargo check -p unit -p composition`（riscv，`--locked`） | exit 0 |
| `cargo check -p unit -p composition --target x86_64-unknown-linux-gnu --locked` | exit 0 |
| image core host `build --locked` | exit 0；正常依赖树无 composition |
| CLI host `build --locked` | exit 0 |
| image core tests | 18 lib + 3 catalog + 4 second-package = 25 passed |
| `cargo image product debug`（新 CLI） | exit 0，最后 `ok .../debug/initrd.img` |
| `cargo image product release`（正常准备锁/缓存） | exit 0，最后 `ok .../release/initrd.img` |
| `cargo test -p mold -p stream`（首轮，未改代码） | mold 11、stream 3 passed |
| system-shape host tests（Supplies 改动后重跑） | 22 passed |
| guard-deployment / interface-marks（首轮，未改断言） | 16 / 5 passed |
| execution host / unit（首轮，未改启动代码） | 6 / 1 passed |
| `cargo check -p programs --bins --locked` | exit 0 |
| `cargo build -p programs --bins` debug / release（`--locked`） | 均 exit 0 |
| `cargo build -p kernel` debug / release（`--locked`） | 均 exit 0 |
| `prog-rtc` / `prog-fixturetarget` debug+release | 实际链接为 RISC-V ET_EXEC，非零入口 |
| API 唯一性 | 旧 `api.rs` 仅 re-export；`fixturetarget/entry` 与 `rtc/entry` 常量只在所属 API 包 |
| fixture `first`/`second`（`--locked`；second `release` + `link-dep/loud`） | exit 0，RISC-V ELF；artifact profile `release` + `debug_assertions=true` |
| `judge.sh` | `JUDGE_EXIT=0` |

依赖树判定：`unit`/`composition` 无 execution、programs、各 client；image **正常**依赖树无
composition/execution/programs/CLI，composition 只在 dev 边出现。

### 2.1 最终源码 QEMU（实际 Hart 与独立判定）

两个共享 API 包完成后，root 在 `CARGO_NET_OFFLINE=true` 下集中复验：

| 场景 | 实际 Hart | 原始退出 / 判定 |
| --- | --- | --- |
| product + terminal/session | 4 | 0 / ok，自然停止，85→85 |
| product + terminal/session | 8 | 0 / ok，自然停止，153→153 |
| product + Shell smoke | 4 | 0 / ok，自然停止，85→85 |
| accept、system-fault | 各 4 | 各 0，自然停止，85→85 |
| iommu-product / fault / start-before / start-after | 各 4 | 各 0，自然停止，85→85 |

以上九项在 `target/migration-evidence/pr2/final-verified/` 保存命令、原始退出、
console、capture status、交互 driver 结果及逐项 `result.json`。判定读取固件实际 Hart，
并核对唯一入口成功退出、自然停止、原内存基线相等；不由目录名推断核数。

另外针对共享 fixturetarget API，使用原测试要求的 `QEMU_SEMI=1` 与
`--features semihosting` 验证 daemon-target：原判定器核实旧 task/team/helper 在新实例前
回收，退出 0，实际 4 Hart，85→85，相关 lock 前后 hash 一致。证据为
`target/migration-evidence/pr2/daemon-target-traced-final/`。

原始失败与误标记录均保留：旧 `qemu-variant-final/product-session-8` 实际 4 Hart，
不计八核验收；额外 daemon-target 首次未开事件导出，运行与内存检查通过但原判定器缺文件，
整体退出 1；随后误传 cargo-qtest 不支持的 `--locked`，未启动机器。最终按工具真实接口
开启导出后通过，未删除事件判据。cargo-qtest 不接受 `--locked`，因此运行采用 offline，
另核对锁文件前后不变；正常构建检查使用 `--locked`。

原有授权、撤翻译 → 清 MASTER → reset → 关队列 → 释放与最终回收断言、场景 timeout 均未改。

### 2.2 13 个 host suite（测试入口/依赖返工）

修复 6 条指向已删除 `programs/src/unit` 的 `#[path]`：改为正常 `unit-declarations`
（`package = "unit"`）依赖；publication-admission/operator-api 只在测试模块使用，故放
`[dev-dependencies]`，其余按实际上下文使用该别名。pci/iommu 投影 API 的 6 个 host manifest
补 `pci-declaration`/`iommu-declaration`；control-lifetime 补 `system-api`，保留其独立
expected descriptor 算式与全部回收断言。

首次离线运行用于生成/更新缺的 lock，第二次在共用
`CARGO_TARGET_DIR=target/migration-host`、`CARGO_NET_OFFLINE=true` 下用 `--locked` 复跑。
两次均记录 raw exit；以下为第二次：

| suite | tests | exit |
| --- | --- | --- |
| startup-authority | 6 | 0 |
| publication-admission | 3 | 0 |
| operator-api | 25 | 0 |
| daemon-control | 14 | 0 |
| namespace-installation | 4 | 0 |
| control-publisher | 11 | 0 |
| pci-scan | 21 | 0 |
| pci-placement | 17 | 0 |
| device-recovery | 8 | 0 |
| virtio-blk-coord | 20 | 0 |
| iommu-pages | 53 | 0 |
| kernel/tests/dma-confirmation | 6 | 0 |
| control-lifetime | 8 | 0 |

共 196 passed。日志与 exit 在 `target/migration-evidence/pr2/host-suites/`
（`*.prepare.log`、`*.locked.log`、`*.exit`）。新增的 3 份 lock
（publication-admission/operator-api/daemon-control）保留，原有 lock 更新；对应本地
`.gitignore` 的 `/Cargo.lock` 已移除。13 suite 轮次本身只改测试代码/manifest；随后的
group 隔离修复曾按新源码重跑九景（旧日志保留）。`fixturetarget-api`/`rtc-api`
变化后，root 已在 `final-verified/` 完成实际 4/8 Hart 九景，并在
`daemon-target-traced-final/` 完成原事件判定。

## 3. 限制与未验证项

- worker 交接时未复验；root 随后完成最终 4/8 Hart 九景及 daemon-target 事件验收。旧
  `qemu-variant-final/product-session-8` 实际为 4 Hart，不计 8 Hart 验收。
- 不跑 1024 长测；SOAK/DUAL 长代数不在本轮。
- 阶段 3 公共 API 发布 SHA、阶段 4 pipe 独立真实验收未完成；仍是同仓 path 依赖。
- PR1 已知项保持披露、未擅修：default idle 两次失败（router NotReady / CPU 29.5>20）、
  `QEMU_SEMI=1` 不能替代、health 121 passed / 缺 initrd sentinel raw 101、裸 qtest 无 scene
  的 nothing 错误、identity 6!=9 冻结旧源码亦复现。故**不称全绿**。
- 未跑全树 `cargo test`；只跑合同相关 host tests 与短程 QEMU。
- `programs` 仍是一个 Cargo 包，所有 bins 与入口同包，因此 programs 依赖图含 composition；
  但 server 模块（除 `main.rs` 入口）无 `composition::` 引用，声明与 Deployment 由入口显式
  传入。按组件拆 Cargo 包留阶段 3/4。
- image core 的 composition 是 dev-dependency 测试数据；正常发布构建不需要实际组合。

## 4. 检查点

| 项 | 值 |
| --- | --- |
| 验收基线 | `fdc8c3c26d70c6a7f66ae9fce772e7cce81c0de4` |
| 分支 | `codex/shuyra-composition-decoupling` |
| 首轮检查点 | `target/migration-evidence/pr2/first-pass-checkpoint` |
| Worker | `opencode-go / deepseek-v4.1-flash`，`session-d030a784-ae59-4efd-8e01-c2a15233ac19` |
| 本轮 | 阶段 2 提交前验收；阶段 3 仓库尚未创建 |
| 证据 | `target/migration-evidence/pr2/`，最终选择见 `acceptance.json` |
| QEMU | 最终实际 4/8 Hart 九项通过，额外 daemon-target 原事件判定通过 |
| 独立判定 | `judge.sh` 为结构/ELF 检查；运行判定另核对实际 Hart、driver、自然停止与回收 |
| 旧 worker | 未恢复；本会话独立实施 |
