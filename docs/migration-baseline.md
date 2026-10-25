# Shuyra 初步迁移基线（阶段 0–1，target ownership 返工）

> 项目显示名：**Shuyra**。本文件登记阶段 0 基线、阶段 1 主要产物更名和 target
> ownership 合同；不进入阶段 2。

## 1. 冻结源、分支与 Worker

| 项 | 值 |
| --- | --- |
| 项目 | Shuyra（源仓旧名 `sqware`） |
| 源仓 | `git@github.com:anreonyr/sqware.git` / `master` |
| 冻结 SHA | `623fb15843cc61617ab27f3be4f145ff9cbb9d35` |
| 目标组织 | `shuyra-arche`（公开仓库数 0） |
| 分支 | `codex/shuyra-initial-baseline`；HEAD = ref = 冻结 SHA；root 已解决 Git 引用写权限 |
| 当前 worker | `session-c77a3ed4-7d67-4c8f-b5b2-06d88741202c`（`opencode-go` / `deepseek-v4.1-flash`，仅清单/文档返工） |
| 前 worker 检查点 | `session-6ad50d30-0cc5-4597-8e01-596fe99aea8d`（`deepseek-official` / `deepseek-flash`）；QUOTA 退出，已停止 |
| 检查点 | `target/migration-evidence/dsh-worker-checkpoint/` |
| rustc | `rustc 1.100.0-nightly (a69a63265 2026-09-03)` |
| cargo | `cargo 1.100.0-nightly (b2e9d5f9d 2026-09-02)` |
| toolchain | `nightly-2026-09-04`，root 正式安装 7 个官方组件；正式结果无 `RUSTUP_HOME`/`RUSTUP_TOOLCHAIN` 覆盖 |

分支由 root 创建；当前 `HEAD` 与 `refs/heads/codex/shuyra-initial-baseline` 都是
`623fb15843cc61617ab27f3be4f145ff9cbb9d35`。没有 reset、没有提交/推送/PR。前 worker 的源码、新文件和生成器
封存在 `target/migration-evidence/dsh-worker-checkpoint/`；本 worker 没有重置或解包覆盖当前源码。

## 2. Target ownership 覆盖率

每个对象都有 `target_repository`（owner/repo 完整值）、`target_path`（目标仓内路径）和
`role`。当前 source 保留在 `source`，不拿它冒充未来路径。

| 对象 | 行数 | 完整 | 缺失 |
|---|---|---|---|
| binaries | 89 | 89 | 0 |
| units | 85 | 85 | 0 |
| deployments | 36 | 36 | 0 |
| test_manifests | 72 | 72 | 0 |
| root_test_targets | 16 | 16 | 0 |
| checks | 27 | 27 | 0 |
| fixture_directories | 18 | 18 | 0 |
| fixture_files | 175 | 175 | 0 |
| dtb | 2 | 2 | 0 |
| feed_scripts | 55 | 55 | 0 |
| shell_fixtures | 2 | 2 | 0 |
| cross_directory_sources | 350 | 350 | 0 |

`target_repository` 共 32 个：

`anreonyr/sqware`, `shuyra-arche/account`, `shuyra-arche/cat`, `shuyra-arche/daemon`, `shuyra-arche/emit`, `shuyra-arche/env`, `shuyra-arche/execution`, `shuyra-arche/fail`, `shuyra-arche/hub`, `shuyra-arche/image-builder`, `shuyra-arche/iommu`, `shuyra-arche/ipc`, `shuyra-arche/kernel`, `shuyra-arche/login`, `shuyra-arche/mold`, `shuyra-arche/pci`, `shuyra-arche/pipe`, `shuyra-arche/resource`, `shuyra-arche/router`, `shuyra-arche/rtc`, `shuyra-arche/schedule`, `shuyra-arche/shell`, `shuyra-arche/shuyra`, `shuyra-arche/spin`, `shuyra-arche/stream`, `shuyra-arche/system`, `shuyra-arche/terminal`, `shuyra-arche/uart`, `shuyra-arche/upper`, `shuyra-arche/virtio-blk`, `shuyra-arche/wire`, `shuyra-arche/workers`

产品 UnitFile 归各产品 repo 的 `declaration/src/lib.rs`，运行实现才在 `server/`；
image 工具及其 tests/fixtures 归 `shuyra-arche/image-builder`；env/execution/resource/
schedule/wire 的测试各归对应库；Lisp 目标保留在 `anreonyr/sqware` 的
`crates/lisp/tests/language.rs`。覆盖率只证明字段齐全，不代表每个映射都已语义审核通过。

### 2.1 Assertion 审计

`assertion_targets` 只登记含真实断言的测试模块或测试函数。生产实现、纯声明、
薄 main 以及 Deployment 成员关系不再冒充断言，统一改记在 `dependency_sources`；
后者只说明依赖 owner，不声明断言。

| 项 | 值 |
|---|---|
| 真实 assertion_targets | 94 |
| 按 scope | whole-suite=68, whole-host-tool=1, function-split=4, module-split=5, whole-target=16 |
| 含 assertion_targets 的行 | 87 |
| selectors | 151（source_module=144, source_test=6, source_rust_module=1） |
| exclude_selectors | 5（source_rust_module=1, source_module=3, source_test=1） |
| all selectors | 156 |
| 唯一测试模块 | 140 |
| fixture_sources | 6 |
| dependency_sources | 280 |
| 无效 selector | 0 |
| 283 -> 94 的构成 | 运行依赖移除 267 组；manifest 16 -> 78 组；根目标 16 组 |

人工核对依据：只保留含 #[test] 或 assert 的测试模块/函数；生产实现、纯声明、薄 main 和 Deployment 成员依赖全部移入 dependency_sources。整套保留的 suite 用 assertion_scope=whole-suite，精确拆分用 function-split/module-split；source_rust_module 会按 source 文件核对同名 Rust 模块；exclude_selectors 只标记迁出项；根目标用 whole-target。

精确/混合拆分只保留三处：

- `device-api` 的 3 组：terminal 2 个真实 `source_test`、router 2 个真实
  `source_test`、Integration 的 Mark 冲突 1 个真实 `source_test`。
- `shell` 的 pipe：`pipe_registry_authorizes_owner_and_keeps_roles_exclusive`；
  Shell whole-suite 用 `exclude_selectors` 明确排除该函数，迁出后不保留副本。
- `retention` 的 mixed：IOMMU 只保留 ordering.rs 与三份夹具；binding/execution/
  enrollment/protocol 归 Hub，IOMMU 用 `exclude_selectors` 明确排除且不留副本。

## 3. 89 个 programs binary

产品二进制归同名 repo 的 `server/src/main.rs`；六个命令二进制归各自命令 repo 的
`command/src/main.rs`，`composition_owner` 是 `shuyra-arche/shell`，没有为命令伪造 UnitFile。
完整 accept 与组合 guests 归 `shuyra/tests/accept`；跨 Cluster 通用 guests 归
`shuyra/tests/fixtures`；只有实际 driver 协作才进 `tests/clusters/driver`。
运行产物本身不列 `assertion_targets`；它们的实际断言由测试 suite 承载，
`dependency_sources` 只记录运行依赖，不写成断言。

| image | binary | source | target_repository | target_path | role | composition_owner | dependency_sources |
|---|---|---|---|---|---|---|---|
| hub | `prog-hub` | `programs/src/service/hub/main.rs` | `shuyra-arche/hub` | `server/src/main.rs` | product-server | - | - |
| daemon | `prog-daemon` | `programs/src/service/daemon/main.rs` | `shuyra-arche/daemon` | `server/src/main.rs` | product-server | - | - |
| daemon-rtc | `prog-daemon-rtc` | `programs/src/harness/probe/daemon_rtc/root_main.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-rtc/main.rs` | daemon-test-fixture | - | `shuyra-arche/rtc` |
| daemon-ctl-rtc | `prog-daemon-ctl-rtc` | `programs/src/harness/probe/daemon_rtc/control_main.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-ctl-rtc/main.rs` | daemon-test-fixture | - | - |
| daemon-target | `prog-daemon-target` | `programs/src/harness/probe/daemon_target/root_main.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-target/main.rs` | daemon-test-fixture | - | - |
| daemon-ctl-target | `prog-daemon-ctl-target` | `programs/src/harness/probe/daemon_target/control_main.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-ctl-target/main.rs` | daemon-test-fixture | - | - |
| fixturetarget | `prog-fixturetarget` | `programs/src/harness/probe/daemon_target/target_main.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/fixturetarget/main.rs` | daemon-test-fixture | - | - |
| daemon-stop | `prog-daemon-stop` | `programs/src/harness/probe/daemon_stop/root_main.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-stop/main.rs` | daemon-test-fixture | - | - |
| daemon-ctl-stop | `prog-daemon-ctl-stop` | `programs/src/harness/probe/daemon_stop/control_main.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-ctl-stop/main.rs` | daemon-test-fixture | - | - |
| pipe | `prog-pipe` | `programs/src/service/pipe/main.rs` | `shuyra-arche/pipe` | `server/src/main.rs` | product-server | - | - |
| account | `prog-account` | `programs/src/service/account/main.rs` | `shuyra-arche/account` | `server/src/main.rs` | product-server | - | - |
| terminal | `prog-terminal` | `programs/src/user/terminal/main.rs` | `shuyra-arche/terminal` | `server/src/main.rs` | product-server | - | - |
| login | `prog-login` | `programs/src/user/login/main.rs` | `shuyra-arche/login` | `server/src/main.rs` | product-server | - | - |
| shell | `prog-shell` | `programs/src/user/shell/main.rs` | `shuyra-arche/shell` | `server/src/main.rs` | product-server | - | - |
| cat | `prog-cat` | `programs/src/user/cat/main.rs` | `shuyra-arche/cat` | `command/src/main.rs` | shell-command | `shuyra-arche/shell` | - |
| emit | `prog-emit` | `programs/src/user/emit/main.rs` | `shuyra-arche/emit` | `command/src/main.rs` | shell-command | `shuyra-arche/shell` | - |
| upper | `prog-upper` | `programs/src/user/upper/main.rs` | `shuyra-arche/upper` | `command/src/main.rs` | shell-command | `shuyra-arche/shell` | - |
| fail | `prog-fail` | `programs/src/user/fail/main.rs` | `shuyra-arche/fail` | `command/src/main.rs` | shell-command | `shuyra-arche/shell` | - |
| spin | `prog-spin` | `programs/src/user/spin/main.rs` | `shuyra-arche/spin` | `command/src/main.rs` | shell-command | `shuyra-arche/shell` | - |
| workers | `prog-workers` | `programs/src/user/workers/main.rs` | `shuyra-arche/workers` | `command/src/main.rs` | shell-command | `shuyra-arche/shell` | - |
| probe-terminal | `prog-probe-terminal` | `programs/src/harness/probe/probe_terminal/main.rs` | `shuyra-arche/terminal` | `tests/probes/probe-terminal/main.rs` | terminal-test-probe | - | - |
| guest | `prog-guest` | `programs/src/harness/guest/guest/main.rs` | `shuyra-arche/shuyra` | `tests/fixtures/guests/guest/main.rs` | integration-fixture-guest | - | - |
| passer | `prog-passer` | `programs/src/harness/guest/passer/main.rs` | `shuyra-arche/shuyra` | `tests/fixtures/guests/passer/main.rs` | integration-fixture-guest | - | - |
| lodger | `prog-lodger` | `programs/src/harness/guest/lodger/main.rs` | `shuyra-arche/shuyra` | `tests/fixtures/guests/lodger/main.rs` | integration-fixture-guest | - | - |
| router | `prog-router` | `programs/src/driver/router/main.rs` | `shuyra-arche/router` | `server/src/main.rs` | product-server | - | - |
| uart | `prog-uart` | `programs/src/driver/uart/main.rs` | `shuyra-arche/uart` | `server/src/main.rs` | product-server | - | - |
| rtc | `prog-rtc` | `programs/src/driver/rtc/main.rs` | `shuyra-arche/rtc` | `server/src/main.rs` | product-server | - | - |
| sleeper | `prog-sleeper` | `programs/src/harness/guest/sleeper/main.rs` | `shuyra-arche/shuyra` | `tests/accept/guests/sleeper/main.rs` | accept-integration-guest | - | - |
| subject | `prog-subject` | `programs/src/harness/guest/subject/main.rs` | `shuyra-arche/shuyra` | `tests/accept/guests/subject/main.rs` | accept-integration-guest | - | - |
| member | `prog-member` | `programs/src/harness/guest/member/main.rs` | `shuyra-arche/shuyra` | `tests/accept/guests/member/main.rs` | accept-integration-guest | - | - |
| system | `prog-system` | `programs/src/system/main.rs` | `shuyra-arche/system` | `server/src/main.rs` | product-server | - | - |
| accept-system | `prog-accept-system` | `programs/src/harness/probe/accept/main.rs` | `shuyra-arche/shuyra` | `tests/accept/main.rs` | accept-integration-entry | - | - |
| probe-denied | `prog-probe-denied` | `programs/src/harness/probe/probe_denied/main.rs` | `shuyra-arche/system` | `tests/probes/probe-denied/main.rs` | system-test-probe | - | - |
| system-fault | `prog-system-fault` | `programs/src/harness/probe/system_fault/main.rs` | `shuyra-arche/system` | `tests/probes/system-fault/system-fault/main.rs` | system-fault-probe | - | `shuyra-arche/hub`, `shuyra-arche/pipe`, `shuyra-arche/account`, `shuyra-arche/router`, `shuyra-arche/uart`, `shuyra-arche/rtc` |
| system-fault-unit | `prog-system-fault-unit` | `programs/src/harness/probe/system_fault/fault_unit.rs` | `shuyra-arche/system` | `tests/probes/system-fault/system-fault-unit/main.rs` | system-fault-probe | - | - |
| system-dependent | `prog-system-dependent` | `programs/src/harness/probe/system_fault/dependent.rs` | `shuyra-arche/system` | `tests/probes/system-fault/system-dependent/main.rs` | system-fault-probe | - | - |
| system-child | `prog-system-child` | `programs/src/harness/probe/system_fault/child.rs` | `shuyra-arche/system` | `tests/probes/system-fault/system-child/main.rs` | system-fault-probe | - | - |
| memory-transfer | `prog-memory-transfer` | `programs/src/harness/probe/memory_transfer/main.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer/main.rs` | kernel-test-probe | - | - |
| memory-transfer-unit | `prog-memory-transfer-unit` | `programs/src/harness/probe/memory_transfer/unit.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-unit/main.rs` | kernel-test-probe | - | - |
| memory-transfer-tlb | `prog-memory-transfer-tlb` | `programs/src/harness/probe/memory_transfer_tlb/main.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-tlb/main.rs` | kernel-test-probe | - | - |
| memory-transfer-fresh | `prog-memory-transfer-fresh` | `programs/src/harness/probe/memory_transfer_fresh/main.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-fresh/main.rs` | kernel-test-probe | - | - |
| memory-transfer-race | `prog-memory-transfer-race` | `programs/src/harness/probe/memory_transfer_race/main.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-race/main.rs` | kernel-test-probe | - | - |
| memory-transfer-race-unit | `prog-memory-transfer-race-unit` | `programs/src/harness/probe/memory_transfer_race/unit.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-race-unit/main.rs` | kernel-test-probe | - | - |
| iommu-probe | `prog-iommu-probe` | `programs/src/harness/probe/iommu/main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-probe/main.rs` | driver-integration-probe | - | - |
| iommu-dma | `prog-iommu-dma` | `programs/src/harness/probe/iommu/dma/main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-dma/main.rs` | driver-integration-probe | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| iommu | `prog-iommu` | `programs/src/driver/iommu/main.rs` | `shuyra-arche/iommu` | `server/src/main.rs` | product-server | - | - |
| pci | `prog-pci` | `programs/src/driver/pci/main.rs` | `shuyra-arche/pci` | `server/src/main.rs` | product-server | - | - |
| virtio-blk | `prog-virtio-blk` | `programs/src/driver/virtio/blk/main.rs` | `shuyra-arche/virtio-blk` | `server/src/main.rs` | product-server | - | - |
| device-work | `prog-device-work` | `programs/src/harness/device/work_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-work/main.rs` | driver-integration-workload | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-work-before | `prog-device-work-before` | `programs/src/harness/device/work_before_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-work-before/main.rs` | driver-integration-workload | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-work-after | `prog-device-work-after` | `programs/src/harness/device/work_after_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-work-after/main.rs` | driver-integration-workload | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-ctl | `prog-device-ctl` | `programs/src/harness/device/control_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl/main.rs` | driver-integration-controller | - | - |
| device-ctl-fault | `prog-device-ctl-fault` | `programs/src/harness/device/control_fault_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-fault/main.rs` | driver-integration-controller | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-ctl-before | `prog-device-ctl-before` | `programs/src/harness/device/control_before_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-before/main.rs` | driver-integration-controller | - | - |
| device-ctl-after | `prog-device-ctl-after` | `programs/src/harness/device/control_after_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-after/main.rs` | driver-integration-controller | - | - |
| device-ctl-ruin | `prog-device-ctl-ruin` | `programs/src/harness/device/control_ruin_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-ruin/main.rs` | driver-integration-controller | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-ctl-forced | `prog-device-ctl-forced` | `programs/src/harness/device/control_forced_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-forced/main.rs` | driver-integration-controller | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-ctl-guard | `prog-device-ctl-guard` | `programs/src/harness/device/control_guard_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-guard/main.rs` | driver-integration-controller | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-ctl-soak | `prog-device-ctl-soak` | `programs/src/harness/device/control_soak_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-soak/main.rs` | driver-integration-controller | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-ctl-dual | `prog-device-ctl-dual` | `programs/src/harness/device/control_dual_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-dual/main.rs` | driver-integration-controller | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| device-ctl-confirmation | `prog-device-ctl-confirmation` | `programs/src/harness/device/control_confirmation_main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-confirmation/main.rs` | driver-integration-controller | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| iommu-confirmation | `prog-iommu-confirmation` | `programs/src/harness/probe/device_confirmation/main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-confirmation/main.rs` | driver-integration-probe | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| iommu-ruin | `prog-iommu-ruin` | `programs/src/harness/probe/device_ruin/main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-ruin/main.rs` | driver-integration-probe | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| iommu-guard | `prog-iommu-guard` | `programs/src/harness/probe/device_guard/main.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-guard/main.rs` | driver-integration-probe | - | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| probe-owner | `prog-probe-owner` | `programs/src/harness/probe/probe_owner/main.rs` | `shuyra-arche/system` | `tests/probes/probe-owner/main.rs` | system-test-probe | - | - |
| probe-rule | `prog-probe-rule` | `programs/src/harness/probe/probe_rule/main.rs` | `shuyra-arche/system` | `tests/probes/probe-rule/main.rs` | system-test-probe | - | - |
| probe-rule-other | `prog-probe-rule-other` | `programs/src/harness/probe/probe_rule_other/main.rs` | `shuyra-arche/system` | `tests/probes/probe-rule-other/main.rs` | system-test-probe | - | - |
| probe-watch | `prog-probe-watch` | `programs/src/harness/probe/probe_watch/main.rs` | `shuyra-arche/system` | `tests/probes/probe-watch/main.rs` | system-test-probe | - | - |
| probe-watch-after | `prog-probe-watch-after` | `programs/src/harness/probe/probe_watch_after/main.rs` | `shuyra-arche/system` | `tests/probes/probe-watch-after/main.rs` | system-test-probe | - | - |
| probe-watch-gone | `prog-probe-watch-gone` | `programs/src/harness/probe/probe_watch_gone/main.rs` | `shuyra-arche/system` | `tests/probes/probe-watch-gone/main.rs` | system-test-probe | - | - |
| probe-lease | `prog-probe-lease` | `programs/src/harness/probe/probe_lease/main.rs` | `shuyra-arche/system` | `tests/probes/probe-lease/main.rs` | system-test-probe | - | - |
| probe-bound | `prog-probe-bound` | `programs/src/harness/probe/probe_bound/main.rs` | `shuyra-arche/system` | `tests/probes/probe-bound/main.rs` | system-test-probe | - | - |
| probe-rack | `prog-probe-rack` | `programs/src/harness/probe/probe_rack/main.rs` | `shuyra-arche/ipc` | `tests/probes/probe-rack/main.rs` | ipc-test-probe | - | `shuyra-arche/kernel` |
| probe-rack-mount | `prog-probe-rack-mount` | `programs/src/harness/probe/probe_rack_mount/main.rs` | `shuyra-arche/ipc` | `tests/probes/probe-rack-mount/main.rs` | ipc-test-probe | - | `shuyra-arche/kernel` |
| probe-rack-guest | `prog-probe-rack-guest` | `programs/src/harness/probe/probe_rack_guest/main.rs` | `shuyra-arche/ipc` | `tests/probes/probe-rack-guest/main.rs` | ipc-test-probe | - | `shuyra-arche/kernel` |
| probe-coalition | `prog-probe-coalition` | `programs/src/harness/probe/probe_coalition/main.rs` | `shuyra-arche/system` | `tests/probes/probe-coalition/main.rs` | system-test-probe | - | - |
| probe-control | `prog-probe-control` | `programs/src/harness/probe/probe_control/main.rs` | `shuyra-arche/system` | `tests/probes/probe-control/main.rs` | system-test-probe | - | - |
| probe-operator-gate | `prog-probe-operator-gate` | `programs/src/harness/probe/probe_operator_gate/main.rs` | `shuyra-arche/system` | `tests/probes/probe-operator-gate/main.rs` | system-test-probe | - | - |
| probe-operator-land | `prog-probe-operator-land` | `programs/src/harness/probe/probe_operator_land/main.rs` | `shuyra-arche/system` | `tests/probes/probe-operator-land/main.rs` | system-test-probe | - | - |
| churn | `prog-churn` | `programs/src/harness/bench/again/churn/main.rs` | `shuyra-arche/kernel` | `tests/bench/churn/main.rs` | kernel-benchmark | - | - |
| rig | `prog-rig` | `programs/src/harness/bench/rig/rig/main.rs` | `shuyra-arche/kernel` | `tests/bench/rig/main.rs` | kernel-benchmark | - | - |
| busy | `prog-busy` | `programs/src/harness/bench/load/busy/main.rs` | `shuyra-arche/kernel` | `tests/bench/busy/main.rs` | kernel-benchmark | - | - |
| park | `prog-park` | `programs/src/harness/bench/load/park/main.rs` | `shuyra-arche/kernel` | `tests/bench/park/main.rs` | kernel-benchmark | - | - |
| hang | `prog-hang` | `programs/src/harness/bench/rig/hang/main.rs` | `shuyra-arche/kernel` | `tests/bench/hang/main.rs` | kernel-benchmark | - | - |
| load | `prog-load` | `programs/src/harness/bench/load/load/main.rs` | `shuyra-arche/kernel` | `tests/bench/load/main.rs` | kernel-benchmark | - | - |
| beat | `prog-beat` | `programs/src/harness/bench/beat/main.rs` | `shuyra-arche/kernel` | `tests/bench/beat/main.rs` | kernel-benchmark | - | - |
| again | `prog-again` | `programs/src/harness/bench/again/again/main.rs` | `shuyra-arche/kernel` | `tests/bench/again/main.rs` | kernel-benchmark | - | - |
| waiter | `prog-waiter` | `programs/src/harness/bench/group/waiter/main.rs` | `shuyra-arche/ipc` | `tests/bench/waiter/main.rs` | ipc-benchmark | - | `shuyra-arche/kernel` |
| group | `prog-group` | `programs/src/harness/bench/group/group/main.rs` | `shuyra-arche/ipc` | `tests/bench/group/main.rs` | ipc-benchmark | - | `shuyra-arche/kernel` |

## 4. 85 个 UnitFile

UnitFile 的目标仓/路径与镜像分离；recovery 不在表中（不是独立 Unit），
virtio-blk 是 driver Unit，daemon 是普通 `supervised` Unit。

| Unit | source | target_repository | target_path | role | dependency_sources |
|---|---|---|---|---|---|
| `iommu` | `programs/src/driver/iommu/program.rs` | `shuyra-arche/iommu` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `pci` | `programs/src/driver/pci/program.rs` | `shuyra-arche/pci` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `router` | `programs/src/driver/router/program.rs` | `shuyra-arche/router` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `rtc` | `programs/src/driver/rtc/program.rs` | `shuyra-arche/rtc` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `uart` | `programs/src/driver/uart/program.rs` | `shuyra-arche/uart` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `virtio-blk` | `programs/src/driver/virtio/blk/program.rs` | `shuyra-arche/virtio-blk` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `again` | `programs/src/harness/bench/again/again/program.rs` | `shuyra-arche/kernel` | `tests/bench/again/unit.rs` | kernel-benchmark-unit | - |
| `churn` | `programs/src/harness/bench/again/churn/program.rs` | `shuyra-arche/kernel` | `tests/bench/churn/unit.rs` | kernel-benchmark-unit | - |
| `beat` | `programs/src/harness/bench/beat/program.rs` | `shuyra-arche/kernel` | `tests/bench/beat/unit.rs` | kernel-benchmark-unit | - |
| `group` | `programs/src/harness/bench/group/group/program.rs` | `shuyra-arche/ipc` | `tests/bench/group/unit.rs` | ipc-benchmark-unit | `shuyra-arche/kernel` |
| `waiter` | `programs/src/harness/bench/group/waiter/program.rs` | `shuyra-arche/ipc` | `tests/bench/waiter/unit.rs` | ipc-benchmark-unit | `shuyra-arche/kernel` |
| `busy` | `programs/src/harness/bench/load/busy/program.rs` | `shuyra-arche/kernel` | `tests/bench/busy/unit.rs` | kernel-benchmark-unit | - |
| `load` | `programs/src/harness/bench/load/load/program.rs` | `shuyra-arche/kernel` | `tests/bench/load/unit.rs` | kernel-benchmark-unit | - |
| `park` | `programs/src/harness/bench/load/park/program.rs` | `shuyra-arche/kernel` | `tests/bench/park/unit.rs` | kernel-benchmark-unit | - |
| `hang` | `programs/src/harness/bench/rig/hang/program.rs` | `shuyra-arche/kernel` | `tests/bench/hang/unit.rs` | kernel-benchmark-unit | - |
| `rig` | `programs/src/harness/bench/rig/rig/program.rs` | `shuyra-arche/kernel` | `tests/bench/rig/unit.rs` | kernel-benchmark-unit | - |
| `device-work` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-work/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-work-before` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-work-before/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-work-after` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-work-after/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-ctl` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl/unit.rs` | driver-integration-unit | - |
| `device-ctl-ruin` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-ruin/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-ctl-forced` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-forced/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-ctl-guard` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-guard/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-ctl-soak` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-soak/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-ctl-soak-long` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-soak-long/unit.rs` | driver-integration-unit | - |
| `device-ctl-dual` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-dual/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-ctl-dual-long` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-dual-long/unit.rs` | driver-integration-unit | - |
| `device-ctl-confirmation` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-confirmation/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-ctl-fault` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-fault/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `device-ctl-before` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-before/unit.rs` | driver-integration-unit | - |
| `device-ctl-after` | `programs/src/harness/device/programs.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-ctl-after/unit.rs` | driver-integration-unit | - |
| `guest` | `programs/src/harness/guest/guest/program.rs` | `shuyra-arche/shuyra` | `tests/fixtures/guests/guest/unit.rs` | integration-fixture-guest-unit | - |
| `lodger` | `programs/src/harness/guest/lodger/program.rs` | `shuyra-arche/shuyra` | `tests/fixtures/guests/lodger/unit.rs` | integration-fixture-guest-unit | - |
| `member` | `programs/src/harness/guest/member/program.rs` | `shuyra-arche/shuyra` | `tests/accept/guests/member/unit.rs` | accept-integration-guest-unit | - |
| `passer` | `programs/src/harness/guest/passer/program.rs` | `shuyra-arche/shuyra` | `tests/fixtures/guests/passer/unit.rs` | integration-fixture-guest-unit | - |
| `sleeper` | `programs/src/harness/guest/sleeper/program.rs` | `shuyra-arche/shuyra` | `tests/accept/guests/sleeper/unit.rs` | accept-integration-guest-unit | - |
| `subject` | `programs/src/harness/guest/subject/program.rs` | `shuyra-arche/shuyra` | `tests/accept/guests/subject/unit.rs` | accept-integration-guest-unit | - |
| `accept-system` | `programs/src/harness/probe/accept/program.rs` | `shuyra-arche/shuyra` | `tests/accept/unit.rs` | accept-integration-unit | - |
| `daemon-ctl-rtc` | `programs/src/harness/probe/daemon_rtc/control_program.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-ctl-rtc/unit.rs` | daemon-test-unit | - |
| `daemon-rtc` | `programs/src/harness/probe/daemon_rtc/root_program.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-rtc/unit.rs` | daemon-test-unit | `shuyra-arche/rtc` |
| `daemon-ctl-stop` | `programs/src/harness/probe/daemon_stop/control_program.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-ctl-stop/unit.rs` | daemon-test-unit | - |
| `daemon-stop` | `programs/src/harness/probe/daemon_stop/root_program.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-stop/unit.rs` | daemon-test-unit | - |
| `daemon-ctl-target` | `programs/src/harness/probe/daemon_target/control_program.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-ctl-target/unit.rs` | daemon-test-unit | - |
| `daemon-target` | `programs/src/harness/probe/daemon_target/root_program.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/daemon-target/unit.rs` | daemon-test-unit | - |
| `fixturetarget` | `programs/src/harness/probe/daemon_target/target_program.rs` | `shuyra-arche/daemon` | `tests/deployments/daemon/fixturetarget/unit.rs` | daemon-test-unit | - |
| `iommu-confirmation` | `programs/src/harness/probe/device_confirmation/program.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-confirmation/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-guard` | `programs/src/harness/probe/device_guard/program.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-guard/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-ruin` | `programs/src/harness/probe/device_ruin/program.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-ruin/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-dma` | `programs/src/harness/probe/iommu/dma/program.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-dma/unit.rs` | driver-integration-unit | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-probe` | `programs/src/harness/probe/iommu/program.rs` | `shuyra-arche/shuyra` | `tests/clusters/driver/iommu-probe/unit.rs` | driver-integration-unit | - |
| `memory-transfer` | `programs/src/harness/probe/memory_transfer/program.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer/unit.rs` | kernel-test-unit | - |
| `memory-transfer-unit` | `programs/src/harness/probe/memory_transfer/program.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-unit/unit.rs` | kernel-test-unit | - |
| `memory-transfer-fresh` | `programs/src/harness/probe/memory_transfer_fresh/program.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-fresh/unit.rs` | kernel-test-unit | - |
| `memory-transfer-race` | `programs/src/harness/probe/memory_transfer_race/program.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-race/unit.rs` | kernel-test-unit | - |
| `memory-transfer-race-unit` | `programs/src/harness/probe/memory_transfer_race/program.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-race-unit/unit.rs` | kernel-test-unit | - |
| `memory-transfer-tlb` | `programs/src/harness/probe/memory_transfer_tlb/program.rs` | `shuyra-arche/kernel` | `tests/probes/memory-transfer-tlb/unit.rs` | kernel-test-unit | - |
| `probe-bound` | `programs/src/harness/probe/probe_bound/program.rs` | `shuyra-arche/system` | `tests/probes/probe-bound/unit.rs` | system-test-unit | - |
| `probe-coalition` | `programs/src/harness/probe/probe_coalition/program.rs` | `shuyra-arche/system` | `tests/probes/probe-coalition/unit.rs` | system-test-unit | - |
| `probe-control` | `programs/src/harness/probe/probe_control/program.rs` | `shuyra-arche/system` | `tests/probes/probe-control/unit.rs` | system-test-unit | - |
| `probe-denied` | `programs/src/harness/probe/probe_denied/program.rs` | `shuyra-arche/system` | `tests/probes/probe-denied/unit.rs` | system-test-unit | - |
| `probe-lease` | `programs/src/harness/probe/probe_lease/program.rs` | `shuyra-arche/system` | `tests/probes/probe-lease/unit.rs` | system-test-unit | - |
| `probe-operator-gate` | `programs/src/harness/probe/probe_operator_gate/program.rs` | `shuyra-arche/system` | `tests/probes/probe-operator-gate/unit.rs` | system-test-unit | - |
| `probe-operator-land` | `programs/src/harness/probe/probe_operator_land/program.rs` | `shuyra-arche/system` | `tests/probes/probe-operator-land/unit.rs` | system-test-unit | - |
| `probe-owner` | `programs/src/harness/probe/probe_owner/program.rs` | `shuyra-arche/system` | `tests/probes/probe-owner/unit.rs` | system-test-unit | - |
| `probe-rack` | `programs/src/harness/probe/probe_rack/program.rs` | `shuyra-arche/ipc` | `tests/probes/probe-rack/unit.rs` | ipc-test-unit | `shuyra-arche/kernel` |
| `probe-rack-guest` | `programs/src/harness/probe/probe_rack_guest/program.rs` | `shuyra-arche/ipc` | `tests/probes/probe-rack-guest/unit.rs` | ipc-test-unit | `shuyra-arche/kernel` |
| `probe-rack-mount` | `programs/src/harness/probe/probe_rack_mount/program.rs` | `shuyra-arche/ipc` | `tests/probes/probe-rack-mount/unit.rs` | ipc-test-unit | `shuyra-arche/kernel` |
| `probe-rule` | `programs/src/harness/probe/probe_rule/program.rs` | `shuyra-arche/system` | `tests/probes/probe-rule/unit.rs` | system-test-unit | - |
| `probe-rule-other` | `programs/src/harness/probe/probe_rule_other/program.rs` | `shuyra-arche/system` | `tests/probes/probe-rule-other/unit.rs` | system-test-unit | - |
| `probe-terminal` | `programs/src/harness/probe/probe_terminal/program.rs` | `shuyra-arche/terminal` | `tests/probes/probe-terminal/unit.rs` | terminal-test-unit | - |
| `probe-watch` | `programs/src/harness/probe/probe_watch/program.rs` | `shuyra-arche/system` | `tests/probes/probe-watch/unit.rs` | system-test-unit | - |
| `probe-watch-after` | `programs/src/harness/probe/probe_watch_after/program.rs` | `shuyra-arche/system` | `tests/probes/probe-watch-after/unit.rs` | system-test-unit | - |
| `probe-watch-gone` | `programs/src/harness/probe/probe_watch_gone/program.rs` | `shuyra-arche/system` | `tests/probes/probe-watch-gone/unit.rs` | system-test-unit | - |
| `system-fault` | `programs/src/harness/probe/system_fault/program.rs` | `shuyra-arche/system` | `tests/probes/system-fault/system-fault/unit.rs` | system-fault-unit | `shuyra-arche/hub`, `shuyra-arche/pipe`, `shuyra-arche/account`, `shuyra-arche/router`, `shuyra-arche/uart`, `shuyra-arche/rtc` |
| `system-dependent` | `programs/src/harness/probe/system_fault/program.rs` | `shuyra-arche/system` | `tests/probes/system-fault/system-dependent/unit.rs` | system-fault-unit | - |
| `system-child` | `programs/src/harness/probe/system_fault/program.rs` | `shuyra-arche/system` | `tests/probes/system-fault/system-child/unit.rs` | system-fault-unit | - |
| `system-fault-unit` | `programs/src/harness/probe/system_fault/program.rs` | `shuyra-arche/system` | `tests/probes/system-fault/system-fault-unit/unit.rs` | system-fault-unit | - |
| `account` | `programs/src/service/account/program.rs` | `shuyra-arche/account` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `daemon` | `programs/src/service/daemon/program.rs` | `shuyra-arche/daemon` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `hub` | `programs/src/service/hub/program.rs` | `shuyra-arche/hub` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `pipe` | `programs/src/service/pipe/program.rs` | `shuyra-arche/pipe` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `system` | `programs/src/system/program.rs` | `shuyra-arche/system` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `login` | `programs/src/user/login/program.rs` | `shuyra-arche/login` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `shell` | `programs/src/user/shell/program.rs` | `shuyra-arche/shell` | `declaration/src/lib.rs` | product-unit-declaration | - |
| `terminal` | `programs/src/user/terminal/program.rs` | `shuyra-arche/terminal` | `declaration/src/lib.rs` | product-unit-declaration | - |

### Unit 边界

- recovery：`programs/src/driver/virtio/blk/recovery/` 模块树；不是 UnitFile，没有
  `prog-recovery`，通过 virtio-blk driver Unit 使用。
- virtio-blk：独立 driver Unit，`programs/src/driver/virtio/blk/program.rs`。
- daemon：普通监督 Unit，`supervision = Some("supervised")`，`Ending::Resident`，
  依赖 `Bootstrap(Operator)` 和 `Target(supervised)`。
- 六个 Shell 命令没有 UnitFile；它们的 `composition_owner` 是 `shuyra-arche/shell`。

## 5. 36 个 Deployment（完整成员）

| Deployment | entry | start | targets | bootstrap | guarded | resources | sources | 目标 | role | dependency_sources |
|---|---|---|---|---|---|---|---|---|---|---|
| `iommu-fail-readback` | `system` | hub, pci, iommu, virtio-blk, device-ctl-fault | scene=[hub, pci, iommu, virtio-blk, device-ctl-fault] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-fail-readback/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-dual` | `system` | hub, pci, iommu, virtio-blk, device-ctl-dual | scene=[hub, pci, iommu, virtio-blk, device-ctl-dual] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-dual/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-dual-long` | `system` | hub, pci, iommu, virtio-blk, device-ctl-dual-long | scene=[hub, pci, iommu, virtio-blk, device-ctl-dual-long] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-dual-long/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-soak-long` | `system` | hub, pci, iommu, virtio-blk, device-ctl-soak-long | scene=[hub, pci, iommu, virtio-blk, device-ctl-soak-long] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-soak-long/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-fail-fence` | `system` | hub, pci, iommu, virtio-blk, device-ctl-fault | scene=[hub, pci, iommu, virtio-blk, device-ctl-fault] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-fail-fence/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-fail-reference` | `iommu-confirmation` | hub, pci, iommu, virtio-blk, device-ctl-confirmation | scene=[hub, pci, iommu, virtio-blk, device-ctl-confirmation] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-fail-reference/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-fail-invalidate` | `iommu-confirmation` | hub, pci, iommu, virtio-blk, device-ctl-confirmation | scene=[hub, pci, iommu, virtio-blk, device-ctl-confirmation] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-fail-invalidate/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-fail-reset` | `iommu-confirmation` | hub, pci, iommu, virtio-blk, device-ctl-confirmation | scene=[hub, pci, iommu, virtio-blk, device-ctl-confirmation] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-fail-reset/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-fail-release` | `iommu-confirmation` | hub, pci, iommu, virtio-blk, device-ctl-confirmation | scene=[hub, pci, iommu, virtio-blk, device-ctl-confirmation] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-fail-release/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-soak` | `system` | hub, pci, iommu, virtio-blk, device-ctl-soak | scene=[hub, pci, iommu, virtio-blk, device-ctl-soak] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-soak/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-forced` | `system` | hub, pci, iommu, virtio-blk, device-ctl-forced | scene=[hub, pci, iommu, virtio-blk, device-ctl-forced] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-forced/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-ruin` | `iommu-ruin` | hub, pci, iommu, virtio-blk, device-ctl-ruin | scene=[hub, pci, iommu, virtio-blk, device-ctl-ruin] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-ruin/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-guard-pci` | `iommu-guard` | hub, pci, iommu, virtio-blk, device-ctl-guard | scene=[hub, pci, iommu, virtio-blk, device-ctl-guard] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-guard-pci/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-guard-iommu` | `iommu-guard` | hub, pci, iommu, virtio-blk, device-ctl-guard | scene=[hub, pci, iommu, virtio-blk, device-ctl-guard] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-guard-iommu/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-guard-virtio-blk` | `iommu-guard` | hub, pci, iommu, virtio-blk, device-ctl-guard | scene=[hub, pci, iommu, virtio-blk, device-ctl-guard] | Operator, Identity | pci, iommu, virtio-blk | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-guard-virtio-blk/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `daemon-rtc` | `daemon-rtc` | hub, router, rtc, daemon, daemon-ctl-rtc | scene=[hub, router, rtc, daemon, daemon-ctl-rtc]; supervised=[rtc] | Operator, Identity | - | - | - | `shuyra-arche/daemon` / `tests/deployments/daemon-rtc/deployment.rs` | daemon-deployment | `shuyra-arche/daemon`, `shuyra-arche/rtc` |
| `daemon-target` | `daemon-target` | fixturetarget, daemon, daemon-ctl-target | scene=[fixturetarget, daemon, daemon-ctl-target]; supervised=[fixturetarget] | Operator, Identity | - | - | - | `shuyra-arche/daemon` / `tests/deployments/daemon-target/deployment.rs` | daemon-deployment | - |
| `daemon-stop` | `daemon-stop` | fixturetarget, daemon, daemon-ctl-stop | scene=[fixturetarget, daemon, daemon-ctl-stop]; supervised=[fixturetarget] | Operator, Identity | - | - | - | `shuyra-arche/daemon` / `tests/deployments/daemon-stop/deployment.rs` | daemon-deployment | - |
| `accept` | `accept-system` | hub, pipe, account, terminal, login, probe-terminal, guest, passer, lodger, router, uart, rtc, sleeper, subject, member, probe-denied, probe-owner, probe-rule, probe-rule-other, probe-watch, probe-watch-after, probe-watch-gone, probe-lease, probe-bound, probe-rack, probe-rack-mount, probe-rack-guest, probe-coalition, probe-control, probe-operator-gate, probe-operator-land | scene=[hub, pipe, account, terminal, router, uart, rtc, probe-terminal, guest, passer, lodger, sleeper, subject, member, probe-denied, probe-owner, probe-rule, probe-rule-other, probe-watch, probe-watch-after, probe-watch-gone, probe-lease, probe-bound, probe-rack, probe-rack-mount, probe-rack-guest, probe-coalition, probe-operator-gate, probe-operator-land] | Operator, Identity | - | - | - | `shuyra-arche/shuyra` / `tests/accept/deployment.rs` | accept-integration-deployment | - |
| `product` | `system` | hub, pipe, account, terminal, login, router, uart, rtc, daemon | scene=[hub, pipe, account, terminal, router, uart, rtc, daemon]; supervised=[rtc] | Operator, Identity | - | shell, cat, emit, upper, fail, spin, workers | shell, cat, emit, upper, fail, spin, workers | `shuyra-arche/shuyra` / `tests/clusters/product/deployment.rs` | product-integration-deployment | `shuyra-arche/system`, `shuyra-arche/hub`, `shuyra-arche/pipe`, `shuyra-arche/account`, `shuyra-arche/terminal`, `shuyra-arche/login`, `shuyra-arche/shell`, `shuyra-arche/router`, `shuyra-arche/uart`, `shuyra-arche/rtc`, `shuyra-arche/daemon`, `shuyra-arche/cat`, `shuyra-arche/emit`, `shuyra-arche/upper`, `shuyra-arche/fail`, `shuyra-arche/spin`, `shuyra-arche/workers` |
| `iommu-product` | `system` | hub, pipe, account, terminal, login, router, uart, rtc, daemon, pci, iommu, virtio-blk, device-ctl | scene=[hub, pipe, account, terminal, router, uart, rtc, daemon, pci, iommu, virtio-blk, device-ctl]; supervised=[rtc] | Operator, Identity | pci, iommu, virtio-blk | shell, cat, emit, upper, fail, spin, workers | shell, cat, emit, upper, fail, spin, workers | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-product/deployment.rs` | product-driver-integration-deployment | `shuyra-arche/system`, `shuyra-arche/hub`, `shuyra-arche/pipe`, `shuyra-arche/account`, `shuyra-arche/terminal`, `shuyra-arche/login`, `shuyra-arche/shell`, `shuyra-arche/router`, `shuyra-arche/uart`, `shuyra-arche/rtc`, `shuyra-arche/daemon`, `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk`, `shuyra-arche/cat`, `shuyra-arche/emit`, `shuyra-arche/upper`, `shuyra-arche/fail`, `shuyra-arche/spin`, `shuyra-arche/workers` |
| `iommu-product-fault` | `system` | hub, pipe, account, terminal, login, router, uart, rtc, daemon, pci, iommu, virtio-blk, device-ctl-fault | scene=[hub, pipe, account, terminal, router, uart, rtc, daemon, pci, iommu, virtio-blk, device-ctl-fault]; supervised=[rtc] | Operator, Identity | pci, iommu, virtio-blk | shell, cat, emit, upper, fail, spin, workers | shell, cat, emit, upper, fail, spin, workers | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-product-fault/deployment.rs` | product-driver-integration-deployment | `shuyra-arche/system`, `shuyra-arche/hub`, `shuyra-arche/pipe`, `shuyra-arche/account`, `shuyra-arche/terminal`, `shuyra-arche/login`, `shuyra-arche/shell`, `shuyra-arche/router`, `shuyra-arche/uart`, `shuyra-arche/rtc`, `shuyra-arche/daemon`, `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk`, `shuyra-arche/cat`, `shuyra-arche/emit`, `shuyra-arche/upper`, `shuyra-arche/fail`, `shuyra-arche/spin`, `shuyra-arche/workers` |
| `iommu-product-before` | `system` | hub, pipe, account, terminal, login, router, uart, rtc, daemon, pci, iommu, virtio-blk, device-ctl-before | scene=[hub, pipe, account, terminal, router, uart, rtc, daemon, pci, iommu, virtio-blk, device-ctl-before]; supervised=[rtc] | Operator, Identity | pci, iommu, virtio-blk | shell, cat, emit, upper, fail, spin, workers | shell, cat, emit, upper, fail, spin, workers | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-product-before/deployment.rs` | product-driver-integration-deployment | `shuyra-arche/system`, `shuyra-arche/hub`, `shuyra-arche/pipe`, `shuyra-arche/account`, `shuyra-arche/terminal`, `shuyra-arche/login`, `shuyra-arche/shell`, `shuyra-arche/router`, `shuyra-arche/uart`, `shuyra-arche/rtc`, `shuyra-arche/daemon`, `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk`, `shuyra-arche/cat`, `shuyra-arche/emit`, `shuyra-arche/upper`, `shuyra-arche/fail`, `shuyra-arche/spin`, `shuyra-arche/workers` |
| `iommu-product-after` | `system` | hub, pipe, account, terminal, login, router, uart, rtc, daemon, pci, iommu, virtio-blk, device-ctl-after | scene=[hub, pipe, account, terminal, router, uart, rtc, daemon, pci, iommu, virtio-blk, device-ctl-after]; supervised=[rtc] | Operator, Identity | pci, iommu, virtio-blk | shell, cat, emit, upper, fail, spin, workers | shell, cat, emit, upper, fail, spin, workers | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-product-after/deployment.rs` | product-driver-integration-deployment | `shuyra-arche/system`, `shuyra-arche/hub`, `shuyra-arche/pipe`, `shuyra-arche/account`, `shuyra-arche/terminal`, `shuyra-arche/login`, `shuyra-arche/shell`, `shuyra-arche/router`, `shuyra-arche/uart`, `shuyra-arche/rtc`, `shuyra-arche/daemon`, `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk`, `shuyra-arche/cat`, `shuyra-arche/emit`, `shuyra-arche/upper`, `shuyra-arche/fail`, `shuyra-arche/spin`, `shuyra-arche/workers` |
| `system-fault` | `system-fault` | hub, pipe, account, router, uart, rtc, system-dependent, system-child | scene=[hub, pipe, account, router, uart, rtc, system-dependent, system-child] | Operator, Identity | - | system-fault-unit | system-fault-unit | `shuyra-arche/system` / `tests/deployments/system-fault/deployment.rs` | system-deployment | `shuyra-arche/system`, `shuyra-arche/hub`, `shuyra-arche/pipe`, `shuyra-arche/account`, `shuyra-arche/router`, `shuyra-arche/uart`, `shuyra-arche/rtc` |
| `memory-transfer` | `memory-transfer` |  | scene=[] | Operator, Identity | - | memory-transfer-unit | memory-transfer-unit | `shuyra-arche/kernel` / `tests/deployments/memory-transfer/deployment.rs` | kernel-deployment | - |
| `memory-transfer-tlb` | `memory-transfer-tlb` |  | scene=[] | Operator, Identity | - | - | - | `shuyra-arche/kernel` / `tests/deployments/memory-transfer-tlb/deployment.rs` | kernel-deployment | - |
| `memory-transfer-fresh` | `memory-transfer-fresh` |  | scene=[] | Operator, Identity | - | - | - | `shuyra-arche/kernel` / `tests/deployments/memory-transfer-fresh/deployment.rs` | kernel-deployment | - |
| `memory-transfer-race` | `memory-transfer-race` |  | scene=[] | Operator, Identity | - | memory-transfer-race-unit | memory-transfer-race-unit | `shuyra-arche/kernel` / `tests/deployments/memory-transfer-race/deployment.rs` | kernel-deployment | - |
| `iommu-probe` | `iommu-probe` |  | scene=[] | Operator, Identity | - | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-probe/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `iommu-dma` | `iommu-dma` |  | scene=[] | Operator, Identity | - | - | - | `shuyra-arche/shuyra` / `tests/clusters/driver/iommu-dma/deployment.rs` | driver-integration-deployment | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/virtio-blk` |
| `rig` | `rig` |  | scene=[] | Operator, Identity | - | hang | hang | `shuyra-arche/kernel` / `tests/deployments/rig/deployment.rs` | kernel-benchmark-deployment | - |
| `load` | `load` |  | scene=[] | Operator, Identity | - | busy, park | busy, park | `shuyra-arche/kernel` / `tests/deployments/load/deployment.rs` | kernel-benchmark-deployment | - |
| `group` | `group` |  | scene=[] | Operator, Identity | - | waiter | waiter | `shuyra-arche/ipc` / `tests/deployments/group/deployment.rs` | ipc-benchmark-deployment | `shuyra-arche/ipc`, `shuyra-arche/kernel` |
| `beat` | `beat` |  | scene=[] | Operator, Identity | - | - | - | `shuyra-arche/kernel` / `tests/deployments/beat/deployment.rs` | kernel-benchmark-deployment | - |
| `again` | `again` |  | scene=[] | Operator, Identity | - | churn | churn | `shuyra-arche/kernel` / `tests/deployments/again/deployment.rs` | kernel-benchmark-deployment | - |

运行声明原样登记；没有改变 start、targets、guarded、bootstrap、resources 或 sources。
Deployment 成员只记在 `dependency_sources`，不再推导成 assertion_targets。
accept 是完整 Integration 部署，不拆到产品 repo；只有 iommu/device 等 driver 协作才
落在 `tests/clusters/driver`。

## 6. 独立 Cargo manifest（72）

70 个独立测试套件 + `crates/image`（image-builder host tool）+ 暂留的
`crates/alloc-probe`（宿主工具 crate）。

`daemon-control` 的生产 Control 授权/intent/恢复准入归 `system`；`daemon-runtime` 才归
`daemon`。`device-api` 是混合 suite，目标在 Integration 的 `tests/device-api`，其
terminal/router 断言按真实 `source_test` 拆到同名 repo，跨 provider Mark 冲突留在
Integration；`shell` 的 `pipe_registry_authorizes_owner_and_keeps_roles_exclusive` 归 `pipe`。
`retention` 也是 mixed：IOMMU 只留 ordering.rs 与夹具，binding/execution/enrollment/
protocol 归 Hub。除这些精确拆分外，其余 suite 整套保留：`assertion_scope=whole-suite`，
selectors 只列实际含断言的测试模块；原先由生产模块推导的拆分移入 `dependency_sources`。

| manifest | package | kind | target_repository | target_path | role | executed | assertion_scope | assertion_targets | dependency_sources |
|---|---|---|---|---|---|---|---|---|---|
| `crates/alloc-probe/Cargo.toml` | alloc-probe | host-tool | `anreonyr/sqware` | `crates/alloc-probe/Cargo.toml` | retained-host-tool | no | delegated-to-root-test-targets | - | - |
| `crates/execution/tests/Cargo.toml` | execution-tests | test-suite | `shuyra-arche/execution` | `tests/Cargo.toml` | execution-host-test-suite | no | whole-suite | `shuyra-arche/execution` | - |
| `crates/execution/tests/unit/Cargo.toml` | execution-unit-tests | test-suite | `shuyra-arche/execution` | `tests/unit/Cargo.toml` | execution-host-test-suite | no | whole-suite | `shuyra-arche/execution` | - |
| `crates/image/Cargo.toml` | image | host-tool | `shuyra-arche/image-builder` | `Cargo.toml` | image-builder-host-tool | yes | whole-host-tool | `shuyra-arche/image-builder` | - |
| `crates/ipc/tests/host/Cargo.toml` | ipc-rpc-host-tests | test-suite | `shuyra-arche/ipc` | `tests/host/Cargo.toml` | ipc-host-test-suite | no | whole-suite | `shuyra-arche/ipc` | - |
| `crates/ipc/tests/rack-reader-host/Cargo.toml` | ipc-rack-reader-host-tests | test-suite | `shuyra-arche/ipc` | `tests/rack-reader-host/Cargo.toml` | ipc-host-test-suite | no | whole-suite | `shuyra-arche/ipc` | - |
| `crates/ipc/tests/session-establish/Cargo.toml` | ipc-session-establish-host | test-suite | `shuyra-arche/ipc` | `tests/session-establish/Cargo.toml` | ipc-host-test-suite | no | whole-suite | `shuyra-arche/ipc` | - |
| `crates/ipc/tests/session-exchange/Cargo.toml` | ipc-session-exchange-host | test-suite | `shuyra-arche/ipc` | `tests/session-exchange/Cargo.toml` | ipc-host-test-suite | no | whole-suite | `shuyra-arche/ipc` | - |
| `crates/resource/tests/capability/Cargo.toml` | resource-capability-tests | test-suite | `shuyra-arche/resource` | `tests/capability/Cargo.toml` | resource-host-test-suite | no | whole-suite | `shuyra-arche/resource` | - |
| `crates/schedule/src/tests/Cargo.toml` | schedule-tests | test-suite | `shuyra-arche/schedule` | `tests/Cargo.toml` | schedule-host-test-suite | no | whole-suite | `shuyra-arche/schedule` | - |
| `crates/wire/tests/host/Cargo.toml` | wire-host-tests | test-suite | `shuyra-arche/wire` | `tests/host/Cargo.toml` | wire-host-test-suite | no | whole-suite | `shuyra-arche/wire` | - |
| `kernel/tests/console-line/Cargo.toml` | kernel-console-line-tests | test-suite | `shuyra-arche/kernel` | `tests/console-line/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/custody/Cargo.toml` | kernel-custody-tests | test-suite | `shuyra-arche/kernel` | `tests/custody/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/dma-confirmation/Cargo.toml` | dma-confirmation-tests | test-suite | `shuyra-arche/kernel` | `tests/dma-confirmation/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/export/Cargo.toml` | kernel-export-tests | test-suite | `shuyra-arche/kernel` | `tests/export/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/frame-runs/Cargo.toml` | kernel-frame-runs-tests | test-suite | `shuyra-arche/kernel` | `tests/frame-runs/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/pend/Cargo.toml` | kernel-pend-tests | test-suite | `shuyra-arche/kernel` | `tests/pend/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/scheduler-dispatch/Cargo.toml` | scheduler-dispatch-tests | test-suite | `shuyra-arche/kernel` | `tests/scheduler-dispatch/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/trace/Cargo.toml` | kernel-trace-tests | test-suite | `shuyra-arche/kernel` | `tests/trace/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/wait-block/Cargo.toml` | kernel-wait-block-tests | test-suite | `shuyra-arche/kernel` | `tests/wait-block/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `kernel/tests/weak-copies/Cargo.toml` | kernel-weak-copies-tests | test-suite | `shuyra-arche/kernel` | `tests/weak-copies/Cargo.toml` | kernel-host-test-suite | no | whole-suite | `shuyra-arche/kernel` | - |
| `programs/tests/bootstrap-carrier/Cargo.toml` | bootstrap-carrier-tests | test-suite | `shuyra-arche/system` | `tests/bootstrap-carrier/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/capability-import/Cargo.toml` | capability-import-tests | test-suite | `shuyra-arche/hub` | `tests/capability-import/Cargo.toml` | hub-test-suite | no | whole-suite | `shuyra-arche/hub` | `shuyra-arche/system` |
| `programs/tests/control-api/Cargo.toml` | control-api-tests | test-suite | `shuyra-arche/system` | `tests/control-api/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/control-instance/Cargo.toml` | control-instance-tests | test-suite | `shuyra-arche/system` | `tests/control-instance/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/control-lifetime/Cargo.toml` | control-lifetime-tests | test-suite | `shuyra-arche/system` | `tests/control-lifetime/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/control-publisher/Cargo.toml` | control-publisher-host-tests | test-suite | `shuyra-arche/system` | `tests/control-publisher/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/control-query/Cargo.toml` | control-query-tests | test-suite | `shuyra-arche/system` | `tests/control-query/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/control-rpc/Cargo.toml` | control-rpc-host-tests | test-suite | `shuyra-arche/system` | `tests/control-rpc/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/daemon-control/Cargo.toml` | daemon-control-tests | test-suite | `shuyra-arche/system` | `tests/host/daemon-control/Cargo.toml` | system-control-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/daemon-runtime/Cargo.toml` | daemon-runtime-tests | test-suite | `shuyra-arche/daemon` | `tests/daemon-runtime/Cargo.toml` | daemon-test-suite | no | whole-suite | `shuyra-arche/daemon` | - |
| `programs/tests/device-api/Cargo.toml` | device-api-tests | test-suite | `shuyra-arche/shuyra` | `tests/device-api/Cargo.toml` | integration-mixed-provider-suite | no | function-split | `shuyra-arche/terminal`, `shuyra-arche/router`, `shuyra-arche/shuyra` | - |
| `programs/tests/device-recovery/Cargo.toml` | device-recovery-tests | test-suite | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/Cargo.toml` | driver-integration-test-suite | no | whole-suite | `shuyra-arche/shuyra` | `shuyra-arche/pci`, `shuyra-arche/virtio-blk` |
| `programs/tests/guard-deployment/Cargo.toml` | guard-deployment-tests | test-suite | `shuyra-arche/system` | `tests/guard-deployment/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/hub-api/Cargo.toml` | hub-api-tests | test-suite | `shuyra-arche/hub` | `tests/hub-api/Cargo.toml` | hub-test-suite | no | whole-suite | `shuyra-arche/hub` | - |
| `programs/tests/hub-coordination/Cargo.toml` | hub-coordination-tests | test-suite | `shuyra-arche/hub` | `tests/hub-coordination/Cargo.toml` | hub-test-suite | no | whole-suite | `shuyra-arche/hub` | - |
| `programs/tests/hub-handoff/Cargo.toml` | hub-handoff-tests | test-suite | `shuyra-arche/hub` | `tests/hub-handoff/Cargo.toml` | hub-test-suite | no | whole-suite | `shuyra-arche/hub` | - |
| `programs/tests/identity/Cargo.toml` | identity-tests | test-suite | `shuyra-arche/system` | `tests/identity/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/identity-api/Cargo.toml` | identity-api-tests | test-suite | `shuyra-arche/system` | `tests/identity-api/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/identity-publication/Cargo.toml` | identity-publication-tests | test-suite | `shuyra-arche/system` | `tests/identity-publication/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/identity-rpc/Cargo.toml` | identity-rpc-tests | test-suite | `shuyra-arche/system` | `tests/identity-rpc/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/interface-marks/Cargo.toml` | interface-mark-tests | test-suite | `shuyra-arche/system` | `tests/interface-marks/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/iommu-pages/Cargo.toml` | iommu-pages-tests | test-suite | `shuyra-arche/iommu` | `tests/iommu-pages/Cargo.toml` | iommu-test-suite | no | whole-suite | `shuyra-arche/iommu` | `shuyra-arche/kernel` |
| `programs/tests/iommu-queue/Cargo.toml` | iommu-queue-tests | test-suite | `shuyra-arche/iommu` | `tests/iommu-queue/Cargo.toml` | iommu-test-suite | no | whole-suite | `shuyra-arche/iommu` | - |
| `programs/tests/lease-window/Cargo.toml` | lease-window-tests | test-suite | `shuyra-arche/system` | `tests/lease-window/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/loader-api/Cargo.toml` | loader-api-tests | test-suite | `shuyra-arche/system` | `tests/loader-api/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/loader-client/Cargo.toml` | loader-client-host-tests | test-suite | `shuyra-arche/system` | `tests/loader-client/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/login/Cargo.toml` | login-tests | test-suite | `shuyra-arche/login` | `tests/login/Cargo.toml` | login-test-suite | no | whole-suite | `shuyra-arche/login` | - |
| `programs/tests/namespace-installation/Cargo.toml` | namespace-installation-tests | test-suite | `shuyra-arche/system` | `tests/namespace-installation/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/namespace-layout/Cargo.toml` | namespace-layout-tests | test-suite | `shuyra-arche/system` | `tests/namespace-layout/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/namespace-retirement/Cargo.toml` | namespace-retirement-tests | test-suite | `shuyra-arche/system` | `tests/namespace-retirement/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/operator-api/Cargo.toml` | operator-api-tests | test-suite | `shuyra-arche/system` | `tests/operator-api/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/operator-client/Cargo.toml` | operator-client-host-tests | test-suite | `shuyra-arche/system` | `tests/operator-client/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/operator-connection/Cargo.toml` | operator-connection-tests | test-suite | `shuyra-arche/system` | `tests/operator-connection/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/operator-handoff/Cargo.toml` | operator-handoff-tests | test-suite | `shuyra-arche/system` | `tests/operator-handoff/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/operator-watch/Cargo.toml` | operator-watch-host-tests | test-suite | `shuyra-arche/system` | `tests/operator-watch/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/pci-placement/Cargo.toml` | pci-placement-tests | test-suite | `shuyra-arche/pci` | `tests/pci-placement/Cargo.toml` | pci-test-suite | no | whole-suite | `shuyra-arche/pci` | `shuyra-arche/kernel` |
| `programs/tests/pci-scan/Cargo.toml` | pci-scan-tests | test-suite | `shuyra-arche/pci` | `tests/pci-scan/Cargo.toml` | pci-test-suite | no | whole-suite | `shuyra-arche/pci` | - |
| `programs/tests/publication-admission/Cargo.toml` | publication-admission-tests | test-suite | `shuyra-arche/system` | `tests/publication-admission/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/retention/Cargo.toml` | retention-tests | test-suite | `shuyra-arche/iommu` | `tests/retention/Cargo.toml` | iommu-test-suite | no | mixed | `shuyra-arche/iommu`, `shuyra-arche/hub` | `shuyra-arche/hub`, `shuyra-arche/iommu` |
| `programs/tests/retirement-boundary/Cargo.toml` | retirement-boundary-tests | test-suite | `shuyra-arche/system` | `tests/retirement-boundary/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/router-handoff/Cargo.toml` | router-handoff-tests | test-suite | `shuyra-arche/router` | `tests/router-handoff/Cargo.toml` | router-test-suite | no | whole-suite | `shuyra-arche/router` | - |
| `programs/tests/shell/Cargo.toml` | shell-tests | test-suite | `shuyra-arche/shell` | `tests/shell/Cargo.toml` | shell-test-suite | no | mixed | `shuyra-arche/shell`, `shuyra-arche/pipe` | `shuyra-arche/pipe` |
| `programs/tests/shell-adapter/Cargo.toml` | shell-adapter-tests | test-suite | `shuyra-arche/shell` | `tests/shell-adapter/Cargo.toml` | shell-test-suite | no | whole-suite | `shuyra-arche/shell` | - |
| `programs/tests/shutdown-policy/Cargo.toml` | shutdown-policy-tests | test-suite | `shuyra-arche/system` | `tests/shutdown-policy/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/startup-authority/Cargo.toml` | startup-authority-tests | test-suite | `shuyra-arche/system` | `tests/startup-authority/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/supply/Cargo.toml` | supply-host-tests | test-suite | `shuyra-arche/system` | `tests/supply/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/system-shape/Cargo.toml` | system-shape-tests | test-suite | `shuyra-arche/system` | `tests/system-shape/Cargo.toml` | system-test-suite | no | whole-suite | `shuyra-arche/system` | - |
| `programs/tests/terminal/Cargo.toml` | terminal-tests | test-suite | `shuyra-arche/terminal` | `tests/terminal/Cargo.toml` | terminal-test-suite | no | whole-suite | `shuyra-arche/terminal` | - |
| `programs/tests/terminal-connection/Cargo.toml` | terminal-connection-tests | test-suite | `shuyra-arche/terminal` | `tests/terminal-connection/Cargo.toml` | terminal-test-suite | no | whole-suite | `shuyra-arche/terminal` | `shuyra-arche/ipc` |
| `programs/tests/virtio-blk-coord/Cargo.toml` | virtio-blk-coord-host-tests | test-suite | `shuyra-arche/virtio-blk` | `tests/virtio-blk-coord/Cargo.toml` | virtio-blk-test-suite | no | whole-suite | `shuyra-arche/virtio-blk` | `shuyra-arche/pci`, `shuyra-arche/iommu`, `shuyra-arche/ipc`, `shuyra-arche/kernel` |
| `programs/tests/virtio-caps/Cargo.toml` | virtio-caps-tests | test-suite | `shuyra-arche/virtio-blk` | `tests/virtio-caps/Cargo.toml` | virtio-blk-test-suite | no | whole-suite | `shuyra-arche/virtio-blk` | - |

### 6.1 shell 与 retention 的精确边界

**shell**：whole-suite 仍保留 Shell 的调用适配与组合断言，但 `exclude_selectors`
明确排除 `pipe_registry_authorizes_owner_and_keeps_roles_exclusive`；该函数只迁
`pipe/tests/shell/pipe_registry.rs`，迁出后从 Shell `lib.rs` 删除，不保留副本。

**retention**（`assertion_scope=mixed`）：

- IOMMU 只保留 `iommu/tests/retention/ordering.rs`；`accessor.rs`、`platform.rs`、
  `references.rs` 作为夹具归 `iommu/tests/retention/` 同相对路径。
- IOMMU 的 `exclude_selectors` 明确排除 `binding`、`execution.rs`、`enrollment.rs`、
  `protocol.rs`；迁出后 IOMMU 不保留任何 Hub 测试副本。
- Hub 接收 `tests/retention/binding.rs`（`source_module=lib.rs` +
  `source_rust_module=binding`，12 个 test）、`execution.rs`（11 个 admission/
  settlement test）、`enrollment.rs`（12 个注册/重试 test）、`protocol.rs`
  （Hub DmaWire），均落在 `hub/tests/retention/` 同相对路径。
- `enrollment/{boundary,effects,failure}.rs` 夹具归 `hub/tests/retention/enrollment/`；
  `lib.rs` 的 `hub_core` 与 `driver/iommu/hold` 只记 `production-dependency`。

## 7. 根 workspace 自动发现/显式测试目标（16）

| source | package | target | kind | target_repository | target_path | role | assertion_scope | executed | result |
|---|---|---|---|---|---|---|---|---|---|
| `crates/env/tests/manifest.rs` | env | `manifest` | root-workspace-auto-test | `shuyra-arche/env` | `tests/manifest.rs` | env-host-test | whole-target | no | - |
| `crates/env/tests/memory.rs` | env | `memory` | root-workspace-auto-test | `shuyra-arche/env` | `tests/memory.rs` | env-host-test | whole-target | no | - |
| `crates/env/tests/nole.rs` | env | `nole` | root-workspace-auto-test | `shuyra-arche/env` | `tests/nole.rs` | env-host-test | whole-target | no | - |
| `crates/env/tests/resource_domains.rs` | env | `resource_domains` | root-workspace-auto-test | `shuyra-arche/env` | `tests/resource_domains.rs` | env-host-test | whole-target | no | - |
| `crates/env/tests/resources.rs` | env | `resources` | root-workspace-auto-test | `shuyra-arche/env` | `tests/resources.rs` | env-host-test | whole-target | no | - |
| `crates/env/tests/signals.rs` | env | `signals` | root-workspace-auto-test | `shuyra-arche/env` | `tests/signals.rs` | env-host-test | whole-target | no | - |
| `crates/env/tests/tole.rs` | env | `tole` | root-workspace-auto-test | `shuyra-arche/env` | `tests/tole.rs` | env-host-test | whole-target | no | - |
| `crates/env/tests/unit.rs` | env | `unit` | root-workspace-auto-test | `shuyra-arche/env` | `tests/unit.rs` | env-host-test | whole-target | no | - |
| `crates/lisp/tests/language.rs` | lisp | `language` | root-workspace-auto-test | `anreonyr/sqware` | `crates/lisp/tests/language.rs` | retained-workspace-test | whole-target | no | - |
| `kernel/tests/embedded.rs` | kernel | `embedded` | kernel-explicit-test | `shuyra-arche/kernel` | `tests/embedded.rs` | kernel-embedded-test | whole-target | yes | formal health: 121 passed, 1 failed scene sentinel, raw 101; boot_error_reports_without_allocation ok |
| `crates/alloc-probe/src/lib.rs` | alloc-probe | `alloc_probe` | alloc-probe-test-target | `anreonyr/sqware` | `crates/alloc-probe/src/lib.rs` | retained-host-test | whole-target | no | - |
| `crates/alloc-probe/tests/heap.rs` | alloc-probe | `heap` | alloc-probe-test-target | `anreonyr/sqware` | `crates/alloc-probe/tests/heap.rs` | retained-host-test | whole-target | no | - |
| `crates/alloc-probe/tests/mt.rs` | alloc-probe | `mt` | alloc-probe-test-target | `anreonyr/sqware` | `crates/alloc-probe/tests/mt.rs` | retained-host-test | whole-target | no | - |
| `crates/alloc-probe/tests/pairing.rs` | alloc-probe | `pairing` | alloc-probe-test-target | `anreonyr/sqware` | `crates/alloc-probe/tests/pairing.rs` | retained-host-test | whole-target | no | - |
| `crates/mold/src/lib.rs` | mold | `lib` | root-workspace-lib-test | `shuyra-arche/mold` | `src/lib.rs` | mold-lib-test | whole-target | yes | 11 passed; root formal foundation-host-results |
| `crates/stream/src/lib.rs` | stream | `lib` | root-workspace-lib-test | `shuyra-arche/stream` | `src/lib.rs` | stream-lib-test | whole-target | yes | 3 passed; root formal foundation-host-results |

共 16 项：env 8、lisp 1、kernel embedded 1、alloc-probe lib+3、
mold lib 1、stream lib 1。已执行 3 项（kernel embedded、mold 11、stream 3）；
未执行 13 项（env 8、lisp 1、alloc-probe 4）。

每项有 1 组 `assertion_scope=whole-target` 的 assertion_targets，selector 指向实际
含断言的测试模块；alloc-probe 的 lib 目标 selector 是 `src/tests.rs`，不是 thin lib.rs。

## 8. Fixtures、checks 与跨目录源码包含

### Fixture 目录（18）

| source | 目标 | 文件数 |
|---|---|---|
| `crates/image/tests/fixtures` | `shuyra-arche/image-builder` / `tests/fixtures` | 1 |
| `kernel/tests/custody/fixture` | `shuyra-arche/kernel` / `tests/custody/fixture` | 1 |
| `kernel/tests/dma-confirmation/fixtures` | `shuyra-arche/kernel` / `tests/dma-confirmation/fixtures` | 5 |
| `kernel/tests/export/fixture` | `shuyra-arche/kernel` / `tests/export/fixture` | 5 |
| `kernel/tests/pend/fixture` | `shuyra-arche/kernel` / `tests/pend/fixture` | 5 |
| `kernel/tests/scheduler-dispatch/fixtures` | `shuyra-arche/kernel` / `tests/scheduler-dispatch/fixtures` | 3 |
| `kernel/tests/wait-block/fixture` | `shuyra-arche/kernel` / `tests/wait-block/fixture` | 7 |
| `programs/tests/control-publisher/fixtures` | `shuyra-arche/system` / `tests/control-publisher/fixtures` | 6 |
| `programs/tests/daemon-control/fixtures` | `shuyra-arche/system` / `tests/host/daemon-control/fixtures` | 22 |
| `programs/tests/device-recovery/fixtures` | `shuyra-arche/shuyra` / `tests/clusters/driver/device-recovery/fixtures` | 12 |
| `programs/tests/guard-deployment/fixtures` | `shuyra-arche/system` / `tests/guard-deployment/fixtures` | 27 |
| `programs/tests/iommu-pages/fixtures` | `shuyra-arche/iommu` / `tests/iommu-pages/fixtures` | 9 |
| `programs/tests/namespace-retirement/fixtures` | `shuyra-arche/system` / `tests/namespace-retirement/fixtures` | 3 |
| `programs/tests/pci-placement/fixtures` | `shuyra-arche/pci` / `tests/pci-placement/fixtures` | 8 |
| `programs/tests/pci-scan/fixtures` | `shuyra-arche/pci` / `tests/pci-scan/fixtures` | 2 |
| `programs/tests/retirement-boundary/src/fixtures` | `shuyra-arche/system` / `tests/retirement-boundary/src/fixtures` | 44 |
| `programs/tests/shutdown-policy/fixtures` | `shuyra-arche/system` / `tests/shutdown-policy/fixtures` | 3 |
| `programs/tests/virtio-blk-coord/fixtures` | `shuyra-arche/virtio-blk` / `tests/virtio-blk-coord/fixtures` | 12 |

Fixture 文件 175 个、DTB 2 个、QEMU feed 脚本
55 个、shell 夹具 2 个，
全部在 JSON `fixtures` 中逐项带 target。`crates/image/tests/fixtures` 归
`image-builder` 的 `tests/fixtures`。

### Checks（27）

| source | target_repository | target_path | role |
|---|---|---|---|
| `kernel/tests/critical/test_verify.py` | `shuyra-arche/kernel` | `tests/critical/test_verify.py` | verifier-negative-controls |
| `kernel/tests/critical/verify.py` | `shuyra-arche/kernel` | `tests/critical/verify.py` | independent-verifier |
| `programs/tests/daemon-recovery/test_daemon_rtc.py` | `shuyra-arche/daemon` | `tests/daemon-recovery/test_daemon_rtc.py` | host-python-test |
| `programs/tests/daemon-recovery/test_daemon_stop.py` | `shuyra-arche/daemon` | `tests/daemon-recovery/test_daemon_stop.py` | host-python-test |
| `programs/tests/daemon-recovery/test_daemon_target.py` | `shuyra-arche/daemon` | `tests/daemon-recovery/test_daemon_target.py` | host-python-test |
| `programs/tests/device-recovery/confirmation.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/confirmation.py` | independent-verifier |
| `programs/tests/device-recovery/dual_check.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/dual_check.py` | independent-verifier |
| `programs/tests/device-recovery/guard_check.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/guard_check.py` | independent-verifier |
| `programs/tests/device-recovery/hardware_check.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/hardware_check.py` | independent-verifier |
| `programs/tests/device-recovery/readback_check.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/readback_check.py` | independent-verifier |
| `programs/tests/device-recovery/ruin_check.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/ruin_check.py` | independent-verifier |
| `programs/tests/device-recovery/test_block.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_block.py` | host-python-test |
| `programs/tests/device-recovery/test_confirmation.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_confirmation.py` | host-python-test |
| `programs/tests/device-recovery/test_dual.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_dual.py` | host-python-test |
| `programs/tests/device-recovery/test_forced.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_forced.py` | host-python-test |
| `programs/tests/device-recovery/test_guard.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_guard.py` | host-python-test |
| `programs/tests/device-recovery/test_hardware.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_hardware.py` | host-python-test |
| `programs/tests/device-recovery/test_readback.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_readback.py` | host-python-test |
| `programs/tests/device-recovery/test_ruin.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_ruin.py` | host-python-test |
| `programs/tests/device-recovery/test_soak.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_soak.py` | host-python-test |
| `programs/tests/device-recovery/test_trace.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_trace.py` | host-python-test |
| `programs/tests/device-recovery/test_wrapper.py` | `shuyra-arche/shuyra` | `tests/clusters/driver/device-recovery/test_wrapper.py` | host-python-test |
| `programs/tests/memory-transfer/test_fresh.py` | `shuyra-arche/kernel` | `tests/memory-transfer/test_fresh.py` | host-python-test |
| `programs/tests/memory-transfer/test_observer.py` | `shuyra-arche/kernel` | `tests/memory-transfer/test_observer.py` | host-python-test |
| `programs/tests/memory-transfer/test_race.py` | `shuyra-arche/kernel` | `tests/memory-transfer/test_race.py` | host-python-test |
| `programs/tests/memory-transfer/test_tlb.py` | `shuyra-arche/kernel` | `tests/memory-transfer/test_tlb.py` | host-python-test |
| `scripts/qtest.nu` | `shuyra-arche/shuyra` | `tests/tools/qtest.nu` | qtest-harness |

这些 check 本身就是断言脚本，目标行即断言位置；没有从它们再推导 split-assertion。

### 跨目录源码包含（350）

全部 350 条都在 JSON `cross_directory_sources.items` 中带 source、target
repository/path 和 role；其中跨 target repo 107 条。

## 9. 品牌素材与产物

| 旧 | 新 | 说明 |
|---|---|---|
| `sqware` | `shuyra` | kernel binary, boot title |
| `sqware-image` | `shuyra-image` | image builder binary and usage text |
| `sqware Lisp Shell` | `Shuyra Lisp Shell` | Shell banner and terminal/session.py filter |
| `assets/sqware-logo.svg` | `assets/shuyra-logo.svg` | brand asset; only main SVG text changed to Shuyra |
| `assets/sqware-logo.png` | `assets/shuyra-logo.png` | re-exported from SVG at 1254x1254 with rsvg-convert 2.63.4 |

`assets/shuyra-logo.svg` 只把主文字 `sqware` 改为 `Shuyra`，几何与其他设计内容未动；
PNG 由 `rsvg-convert 2.63.4` 按 `1254x1254` 从该 SVG 重导出。旧历史记录未改；
除本迁移映射外，仓库内没有其他 `sqware-logo` 引用。

Shell banner renders Shuyra Lisp Shell; assets/shuyra-logo.svg/png were visually checked at 1254x1254 and differ only in the main wordmark.

保留：`prog-*`、Cargo package `kernel`/`image`、密码 `sqware`、`sqware.*` interface id、
`sqware-diagnose.jsonl`、`sqware-*.cap`、协议/Mark/initrd 格式、SBI、旧 docs 历史。

## 10. SBI 与工具

| 文件 | 字节 | SHA-256 | 类型 |
|---|---|---|---|
| SBI.bin | 277880 | c53d58aa9ab17f95fa1af159f0bf05fad5c5622ebc0f1084463435fa6e5752b6 | raw firmware blob |
| SBI.elf | 2430576 | 8673069a256a67b52a82932b734594d3e86811abb981573ae7b2368d81a7abc3 | ELF64 RISC-V static-pie, with debug_info, not stripped |

| 工具 | 版本 |
|---|---|
| qemu-system-riscv64 | QEMU emulator version 11.1.2 |
| nu | 0.116.1 |
| llvm-objcopy | LLVM 23.1.3 (compatible with GNU objcopy) |
| python3 | 3.14.8 |
| dtc | DTC v1.8.1 |
| git | 2.56.0 |
| rustup | 1.29.2 |
| cargo-qtest | cargo-qtest 0.3.2 |
| rsvg-convert | 2.63.4 |

## 11. 验证证据

### 11.1 root 正式结果（无 RUSTUP_HOME/RUSTUP_TOOLCHAIN 覆盖）

| 命令 | exit | 结果 | 证据 |
|---|---|---|---|
| `cargo check --workspace --locked --offline（no RUSTUP_HOME/RUSTUP_TOOLCHAIN override）` | 0 | workspace check passed; warnings only | `target/migration-evidence/root-formal/formal-check.log` |
| `cargo test --manifest-path crates/image/Cargo.toml --target x86_64-unknown-linux-gnu --locked --offline` | 0 | 16 lib + 3 catalog = 19 passed | `target/migration-evidence/root-formal/formal-image.log, formal-results.json` |
| `nu scripts/qtest.nu --package kernel --scene ""` | 101 | 122 tests: 121 passed, 1 failed; only embedded::tests::scene failed for the missing initrd sentinel; raw qtest exit 101; boot_error_reports_without_allocation exact assertion passed | `target/migration-evidence/root-formal/formal-health-empty-scene-result.json, formal-health-empty-scene.log` |
| `nu scripts/qtest.nu --package kernel（bare, no scene）` | 1 | Nushell 'nothing' type error before tests; no tests executed; not counted as a health run | `target/migration-evidence/root-formal/formal-health.log, formal-results.json` |
| `QEMU_SEMI=1 QEMU_SMP=4 python3 programs/tests/shell/idle.py` | 0 | strict idle passed: login 10.5%, shell 20.0%, all 9 checkpoints within thresholds | `target/migration-evidence/root-formal/formal-shell-idle-semi-result.json, formal-shell-idle-semi.log` |
| `cargo test -p mold --lib --target x86_64-unknown-linux-gnu --locked --offline` | 0 | 11 passed | `target/migration-evidence/root-formal/mold-host.log, foundation-host-results.json` |
| `cargo test -p stream --lib --target x86_64-unknown-linux-gnu --locked --offline` | 0 | 3 passed | `target/migration-evidence/root-formal/stream-host.log, foundation-host-results.json` |

正式工具链为 `nightly-2026-09-04`，rustc commit
`a69a63265cfd9e006d43137f98301b8d274ad4c9`。image 为 16 lib + 3 catalog = 19 passed；
health 的 122 项中 121 passed，唯一失败是缺 initrd sentinel 的
`embedded::tests::scene`，raw exit 101，boot 错误精确断言通过；裸无 scene 命令在
测试启动前因 `nothing` 类型错误退出，未计入 health 结果。

### 11.2 旧 worker 别名 toolchain 结果（分开记录，不冒充正式结果）

这些日志来自 `RUSTUP_HOME=target/rustup` 的符号链接别名，保留为 worker 证据；
本轮没有重跑。

| 命令 | exit | 结果 | 日志 |
|---|---|---|---|
| `cargo check --workspace --locked --offline（alias toolchain, before rework）` | 0 | pass; warnings only | `target/migration-evidence/renamed/check-workspace.log` |
| `cargo build -p kernel --locked --offline（alias toolchain）` | 0 | debug/shuyra exists; old debug/sqware absent | `target/migration-evidence/renamed/kernel-debug.log` |
| `cargo build -p kernel --release --locked --offline（alias toolchain）` | 0 | release/shuyra exists; old release/sqware absent | `target/migration-evidence/renamed/kernel-release.log` |
| `cargo build -p programs --bins --release --locked --offline（alias toolchain）` | 0 | 89/89 manifest prog-* binaries exist | `target/migration-evidence/renamed/programs-all-bins-release.log` |
| `cargo image product release（alias toolchain）` | 0 | shuyra-image; initrd 2998272 B, 17 programs | `target/migration-evidence/renamed/image-product-release.log` |
| `QEMU_SMP=8 product + terminal/session.py（alias toolchain）` | 0 | 1 passed, 4.08 s; previous run, not rerun | `target/migration-evidence/renamed/qemu-product-smp8-session.log` |
| `QEMU_SMP=4 accept + terminal/session.py（alias toolchain）` | 0 | 1 passed, 10.19 s; previous run, not rerun | `target/migration-evidence/renamed/qemu-accept-session.log` |
| `QEMU_SMP=4 system-fault（alias toolchain）` | 0 | 1 passed, 9.59 s; previous run, not rerun | `target/migration-evidence/renamed/qemu-system-fault.log` |
| `QEMU_SMP=4 short iommu-product --features semihosting（alias toolchain）` | 0 | 1 passed, 5.43 s; previous run, not rerun | `target/migration-evidence/renamed/qemu-iommu-product.log` |

### 11.3 返工后的 Shell 回归（正式 toolchain）

| 命令 | exit | 结果 | 日志 |
|---|---|---|---|
| `QEMU_SMP=4 product + terminal/session.py` | 0 | 1 passed, 3.88 s; no RUSTUP_HOME/RUSTUP_TOOLCHAIN override; Shuyra Lisp Shell filter | `target/migration-evidence/rework/product-smp4-session.log` |
| `QEMU_SMP=4 product + Shell smoke.py` | 0 | 1 passed, 13.04 s; no RUSTUP_HOME/RUSTUP_TOOLCHAIN override | `target/migration-evidence/rework/product-shell-smoke.log` |

这两次使用正常安装的 `nightly-2026-09-04`，没有 RUSTUP_HOME/RUSTUP_TOOLCHAIN 覆盖；
控制台实际出现 `Shuyra Lisp Shell`。本轮没有新增源改动，也没有重跑这些已通过测试。

### 11.4 idle 的全部结果

环境：default features, QEMU_SMP=4, no RUSTUP_HOME/RUSTUP_TOOLCHAIN override; the semi run adds QEMU_SEMI=1。

| 运行 | 命令 | exit | 结果 | 证据 |
|---|---|---|---|---|
| QEMU_SEMI=1 | `QEMU_SEMI=1 QEMU_SMP=4 python3 programs/tests/shell/idle.py` | 0 | strict idle passed at all 9 checkpoints; login 10.5%, shell 20.0% | `target/migration-evidence/root-formal/formal-shell-idle-semi-result.json, formal-shell-idle-semi.log` |
| 默认 features 首跑 | `python3 programs/tests/shell/idle.py` | 1 | first kernel rebuild with default features: router bond/NotReady before login; 'exit tid=7 reason=0x5 note: bond' then 'system: startup failed NotReady' | `target/migration-evidence/root-formal/normal-idle-results.json, shell-idle-default.log` |
| 默认 features 确认 | `python3 programs/tests/shell/idle.py` | 1 | re-confirmation: 'login: expected idle, got 29.5% of one host core' (threshold 20%) | `target/migration-evidence/root-formal/normal-idle-confirm-result.json, shell-idle-default-confirm.log` |
| 冻结旧源码默认 | `same idle.py on frozen source, default features, QEMU_SMP=4` | 0 | frozen source default main build passed, then the same idle command once passed; login 17.5%, shell 13.0%, all 9 checkpoints within thresholds | `target/migration-evidence/root-formal/baseline-normal-results.json, baseline-normal-idle.log` |

独立 reviewer 的结论：

- kernel：old/new kernel .text and .data byte-identical; .rodata differs only in the 4-byte boot-title change。
- product initrd：all 17 product initrd entries/programs share the same entry points; main execution segment/instruction sequence consistent。
- 缺陷定位：no specific migration logic defect located。
- 状态：default idle remains an unresolved verification limitation; scheduling, timeout and CPU threshold were not changed。来源：independent reviewer conclusion reported by root; no worker rerun。

**结论：** QEMU_SEMI=1 pass is not a substitute for the default-feature pass; regression has not been excluded; default idle remains unresolved.

### 11.5 identity-smoke 基线失败

- 命令：`sh programs/src/harness/probe/identity-smoke.sh on frozen source`
- exit：101
- 结果：reproduced the same 'all fixture programs must exist' assertion, left 6 right 9; confirmed pre-existing baseline failure
- 证据：`target/migration-evidence/root-formal/baseline-identity-smoke-result.json, baseline-identity-smoke.log`

## 12. 真实未验证项

- 70 个独立测试套件：0/70 执行；image host tool 另执行 19 passed。
- 根 workspace 13 个目标未执行：env 8、lisp 1、alloc-probe 4；全 PR 合计 83 个测试目标未执行（70 + 13）。
- kernel [[test]] embedded 经 qtest 执行：121 passed、1 failed（missing initrd sentinel），raw exit 101，不是全绿。
- 默认 features 的 idle 未通过：首次 router bond/NotReady，确认运行 login CPU 29.5% > 20%；QEMU_SEMI=1 通过不能替代，回归未排除。
- identity-smoke 当前与冻结源均 6!=9，属于确认的基线失败。
- 未跑 1024 代 iommu 长测、fmt --all、release debug-assertions 全套。
- ownership_coverage 只验证 target_repository/target_path/role 字段齐全，不等于语义审核通过。
- assertion_targets 是静态审计结果：只证明 selector 指向含 #[test]/assert 的测试代码；除已列执行项外，未逐个运行这些目标。
- dependency_sources 只表示依赖 owner 或 Deployment 成员关系，不表示断言存在或已执行。
- 分支 codex/shuyra-initial-baseline 未提交、未推送、未创建 PR。

## 13. 证据路径

- 机器可读清单：`docs/migration-baseline.json`
- 本文件：`docs/migration-baseline.md`
- root 正式结果：`target/migration-evidence/root-formal/`
- 返工 Shell：`target/migration-evidence/rework/`
- 早期别名证据：`target/migration-evidence/renamed/`、`target/migration-evidence/baseline/`
- 前 worker 检查点：`target/migration-evidence/dsh-worker-checkpoint/`

阶段 2 不进入；lisp/alloc-probe 暂留原仓；没有提交/推送/PR。
