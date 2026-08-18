# SQware

<!--toc:start-->
- [SQware](#sqware)
- [Workspace Layout](#workspace-layout)
- [Call Interface Symmetry](#call-interface-symmetry)
<!--toc:end-->

## Design Principal

This is a Orthogonal and Dual directs micro-kernel

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

