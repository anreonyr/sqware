# SQware

<!--toc:start-->
- [SQware](#sqware)
- [Workspace Layout](#workspace-layout)
- [Call Interface Symmetry](#call-interface-symmetry)
<!--toc:end-->

## Design Principal

This is a Orthogonal and Dual directs micro-kernel

## Kernel Layering Paradigm（内核分层范式）

内核代码统一按四层归位，依赖方向无环（功能 ⊥ 护栏；健康/诊断只读功能）：

| 层 | 位置 | 形态 | 命中后果 |
|---|---|---|---|
| **功能** | `kernel/src/memory/{allocator,manager}`、`work/`、`runtime/switcher` 等 | 实现本身（数据 + 操作） | — |
| **护栏** | `kernel/src/memory/allocator/fence/{checker,banker,ledger,audit}` | **in-path** 运行时不变量检查，钩子内嵌功能路径 | panic → halt（crash scene） |
| **健康** | `kernel/src/health/{mod,pt_reclaim,…}` | **out-of-path** 开机验收，boot 时一次性调用 | `expect!` → panic → halt |
| **诊断** | `kernel/src/runtime/diagnose/{trace,scene,halt,watch,export}` | 证词面：事件流水 / 崩溃现场 / 停机 / 看护 / 宿主导出 | 服务整个内核 |

规则（贴各层 mod 头）：
- **断言归位**：编译期/局部不变量断言（类型义务、函数前置）留功能内；可复用链式断言归 `fence::checker`；验收断言用 `health::expect!`。
- **验收归位**：任何 boot 时独立验收点 → `health`（可增分配压力、锁序回归等）。
- **护栏语义**：hook 恒编译、单行调用，release 空体零开销（`checker`）；gated 深度检查（`banker`/`ledger`/`audit`）debug + audit feature 双开。

## Workspace Layout

- `kernel` — 微内核（S-mode）：SBI 调用、调度、内存、trap 分发（envcall）
- `user` — 用户态程序（U-mode）：`env::*` 系统调用适配层 + 各 demo bin
- `crates/sbi` — S-mode → M-mode 的 SBI 调用封装（`ScallBuilder`；erra + fack）
- `crates/ubi` — U-mode → S-mode 的环境调用封装（`UcallBuilder`；erra）
  - `ubi::fid::Ucall`（调用号）是 kernel 分发与 user 编码共用的**单一事实源**

## Call Interface Symmetry

| 方向 | crate | 调用号 | 构建器 | 模块 |
|---|---|---|---|---|
| U → S（envcall） | `ubi` | `fid::Ucall` | `UcallBuilder` | `ubi::ucall` |
| S → M（SBI） | `sbi` | `fid::*` | `ScallBuilder` | `sbi::scall` |

错误统一走 `erra::Error`：`UResult<T> = Result<T, erra::Error<UError>>`、`SResult<T> = Result<T, erra::Error<SError>>`；`UError`/`SError` 均用 fack derive。

## User 运行时
- **共享入口** `user::entry`：`_start` 引导（保留 a0=arg 后 `call main`）+ `#[panic_handler]`；bin 只需写 `main`。
- **用户堆** `user::heap`：`env::heap_allocate`/`heap_deallocate`（`Ucall::HeapAllocate/HeapDeallocate`）走后端 `Space::heap_allocate/deallocate`（heap 窗口位图）；`#[global_allocator]` 提供 `alloc` 支持（聚合 fack 的 alloc 依赖）。

