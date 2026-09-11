# switcher — 陷阱 · 切换 · envcall 入口（含内核启动）

> 路径约定：`文件:行` 相对 `kernel/src/`。ABI 的声明侧见 [abi.md](abi.md)；
> 退场之后发生的事见 [task.md](task.md) §6。

## 1 · 语义定位

S 态**唯一入口**是 TRAMPOLINE 那一页（`layout.rs:56`）：`__alltraps` 只做「路由 → 存帧 →
切 satp/栈」，出口 `__restore` 只做「切表 → 恢复 → `sret`」（`switcher/trampoline.rs:38-221`）。
**一切策略在 Rust 侧的 `trap_handler`**（`trap.rs:76`）。

**切换的最小充分集**（没有软件上下文——现场全在帧页里）：

```text
帧内 6 个字    kernel_satp / kernel_sp / trap_handler / user_pa / user_satp / self_va
每核 tp        PerHart（machine.rs:99-127）
kernel_sp      每次上台**无条件重写**（scheduler/core/hart.rs:186）
出场登记        ASID（trap.rs:302、trampoline.rs:246）
```

## 2 · 结构

| 文件 | 职责 |
|---|---|
| `switcher/trap.rs` | `trap_handler`（唯一 Rust 入口，`:76`）、`persist`（`:38`）、`EXIT_FAULT`（`:312`） |
| `switcher/trampoline.rs` | `global_asm` 一页：`__alltraps` / `__task_trap:51` / `__core_trap:113` / `__restore:161`；`restore:241`、`alltraps_va:262`、`check_fits_page:270` |
| `switcher/context.rs` | `TrapContext:82`、`Gprs:29`、`init:123`、偏移编译期断言（`:162-173`） |
| `switcher/trap/stack.rs` | per-hart trap 栈窗口与 canary（`:27`）、`trap_stack_hart:67`、`trap_stack_guard_hart:79`、`init:96`、`arm_hart:200` |
| `switcher/envcall.rs` | `dispatch:161` / `dispatch_inner:168`、`instr_len:80`、`ret_err:68`、`subset_to_pte:55` |
| `switcher/envcall/mail.rs`、`pie.rs` | 两条轴的 dispatch（见 [mail.md](mail.md) / [pie.md](pie.md)） |
| `main.rs`、`boot.rs`、`machine.rs` | 启动六步与 root 装载（见 §6） |

## 3 · 帧与栈

帧布局（`context.rs:82-109`）：

```text
0x00 kernel_satp        0x08 kernel_sp        0x10 trap_handler
0x18 trap_stack_corrupt 0x20 user_pa(self_pa) 0x28 user_satp
0x30 gpr[32]            0x130 sstatus         0x138 sepc      0x140 self_va
```

`x0` 不存；`x2/x5/x6` 在 `__restore` 末尾经 `self_va` 收尾（`trampoline.rs:213-220`）。
**跨 asm/Rust 边界一律传物理地址**（`user_pa` ＝ 帧 PA）：`restore(frame_pa)`、`run()` 返回帧 PA；
`self_va` 只用于切表之后访问同一物理页。

- **trap 栈**：`TRAP_STACK_BASE + h·64 KiB` ＝ 首个 4 KiB guard（不映射）+ 60 KiB 栈体
  （`layout.rs:87-95`）；boot 一次块分配、**永不归还**（`stack.rs:123-139`）；canary 写栈底；
  `hart = (sp − BASE) >> 16` 零表反解（`stack.rs:67-75`）；入口 guard 特判**先于** canary
  （`trap.rs:108-116`）。
- **用户栈** ＝ `StackWindow` 的 slot（guard + 16 KiB，立即物化，`space/window/stack.rs:1-50`）；
  **任务帧** ＝ `FrameWindow::claim` 一页（S-only、`U=0`，否则首次陷阱即 storm，
  `space/window/frame.rs:29-48`）。
- **用户↔内核栈切换**：任务路径 `csrrw sp, sscratch, sp` 换到线程帧（`sscratch` 承担
  「用户 sp ↔ 帧 VA」），读帧内 `kernel_satp/kernel_sp/trap_handler` 后切表切栈；内核路径不切
  satp，帧由 `tp` 定位、栈切 trap 栈。理由是 trampoline 页在所有空间**同 VA 恒映射**（G 位），
  故跨页符号只能经帧内元数据取（`trampoline.rs:11-19`）。

## 4 · 时序：用户一次 envcall

1. 用户 `env::ecall::trap`：发 **`ebreak`**（不是 `ecall`），且**必须 `#[inline(never)]`**
   （`crates/env/src/ecall.rs:63-96`）。
2. `__alltraps`：`SPP=0` → `__task_trap`（`trampoline.rs:40-47`）→ **先存 x5 再读 `sscratch`**
   取用户 sp（`:54-59`）→ 存 gpr/sstatus/sepc → 读 `kernel_satp/kernel_sp/trap_handler/self_pa`
   → `csrw satp` + `sfence.vma` → `mv sp, kernel_sp` → `jalr trap_handler`（`:93-105`）。
3. `trap_handler`：由 sp 反解 hart 重建 `tp`（`trap.rs:84-89`）→ 入场 `set_asid(kernel)`
   （`:93`）→ guard/canary 双校验（`:110-129`）→ 按 `scause` 归类（`:153`）。
4. `envcall::dispatch(frame, ident)`（`:210`）：读 a7/a0..a5 → trace → **`sepc += instr_len`**
   （`:182`，按首字节低两位判 RVC 2/4 字节）→ `EnvCall::from_wire(slot, &regs)`（`:186`）。
5. 执行：`Starve` → `current().starve()` 换帧（`:191`）；`Park` → `park()`（`:231`）；
   `Wait` → 可能 `Handoff::Switch`（`:247-251`）；`Reap` → **返回空指针**（`:224`）；
   Mail / Pie 两轴回落（`:573-585`）。
6. 返回：出口再校 canary（`:291-296`）→ `set_asid(next)`（`:302`）→ 返帧 PA →
   `__restore`：清 SIE、写 `sepc`、复原 `sscratch`、恢复 gpr、切 satp、经 `self_va` 收尾
   `x2/x5/x6` → `sret`。

## 5 · 退场窄尾

**为什么不由 envcall 函数自己做**：退场 ＝ 上下文被切走，栈上的活引用随栈一起释放而
**永不递减计数**；`dispatch_inner` 的帧里带着它全部的临时值（`Vec`/`Arc`…），在那里
`quit()` 等于把这些引用计数一起丢掉（`envcall.rs:155-160`）。

**谁在最浅帧收尾**：`Reap { reason }` 只写退出原因 + `drop(ident)` + 返回空
（`envcall.rs:210-224`）；`trap_handler` 见到 `None` → `messenger::quit()`
（`trap.rs:212-215`）→ `swap` → `reap`（钩子 → `Reaped` → 躯壳）→ `bury`（归还栈 slot、trap 帧、
空间）→ `run` 取下一帧。故障隔离的三条杀点（`trap.rs:186-188,245-257,260-280`）与退场同款。

## 6 · 启动时序

```text
OpenSBI(M) → _start（main.rs:26-41：tp = PER_HART + hartid·64、清 SIE、
             sp = _kernel_edge + ROOT_STACK_SIZE、写 ROOT canary）→ main（:44-54）
```

| 步 | 函数 | 做什么 |
|---|---|---|
| 1 | `console::init`（`console.rs:189`） | log 装配 |
| 2 | `machine::init(dtp)`（`machine.rs:256-287`） | 从 FDT 取 hart / hertz / dram / `free` / initrd |
| 3 | `allocator::init`（`memory/allocator/mod.rs:66`、`frame.rs:508`） | bump → hybrid → spare；initrd 成持久保留空洞 |
| 4 | `unit::init`（`work/unit/mod.rs:100-206`） | 探测 satp 模式 → 内核空间 → DRAM 双映射 → `.rodata` 只读 → TRAMPOLINE（`:172`）→ hart 帧页 → `satp::set` → `layout::validate` |
| 5 | `clock::init` / `trace::init` | 计时与事件环 |
| 6 | `trap::init`（`trap/stack.rs:96-193`） | per-hart 内存校验、trap 栈块分配 + 映射 + canary、`check_fits_page`、hart 帧元数据、`timer::beat`、`arm_hart` |
| 7 | `boot::banner`（`boot.rs:44-94`） | 打印 trap vector / trap stack / frames |
| 8 | `boot::init`（`:97-144`） | `scheduler::boot::init` → 钩子 / lockdep / health → **`spawn_root`**（`:189-238`）→ `conductor::rooted` → audit → `boot_harts`（`:244-272`，HSM），带 opaque ＝ 该 hart 栈顶 → ROOT canary 复审 → `restore(scheduler::trap::run())` |

**boot 只装一个域**：`root`。它的镜像是 initrd 里按打包期常量取出的那一份
（`initrd.rs:35`），initrd 区被**只读借映**进 root 的用户段，VA 与长度经启动参数交付——
**清单的解释权在 root 程序**，内核不含清单格式。

## 7 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 帧布局即 ABI | 汇编按错偏移读写 ⇒ 现场错乱 | `context.rs:162-173` 编译期断言 ↔ 汇编偏移 |
| 汇编段落在一页内 | TRAMPOLINE 只映射一页 ⇒ 取指缺页 | `check_fits_page`（`trampoline.rs:270`），boot 调（`stack.rs:164`） |
| trap 栈不溢出 | 覆写邻核或元数据 | guard 特判（`trap.rs:111-116`）+ canary 前后双校（`:121-129,291-296`） |
| 内核态 `tp` 恒 ＝ 本 hart 的 PerHart | `hart_id()` 与帧定位全错 | `main.rs:32`、`boot.rs:29-31`、`trap.rs:84-89`（汇编 `ld 0x08/0x10(tp)`） |
| `kernel_sp` 每次上台重写 | 跨核偷取/迁移后陷阱跑在别的核栈上 | `scheduler/core/hart.rs:186` + `trap.rs:132-148` |
| 内核代码只在内核空间执行（ASID=0 ⇔ 内核） | 路由判据失效 | 路由 `trampoline.rs:9-11,43-47`；出场登记 `trap.rs:302` / `trampoline.rs:246` |
| 帧 PA 恒等映射可写、归属独占 | 写他人已归还的帧 | `persist` 的双闸（`trap.rs:44-52`） |
| 退出原因单一出口 | 事件重复或丢失 | `set_exit_reason`（`envcall.rs:222`、`trap.rs:255,278`）→ `quit` 发 `RoomEvent::Exit` |

## 8 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| 用 `ebreak` 不用 `ecall` | `ebreak`（`scause=3`） | S 态 `ecall` 是 SBI 调用、进 M 态；两类任务共用一个入口（`ecall.rs:73-83`） |
| 路由判据 | `SPP=0` **或** `satp.ASID ≠ 0` → 任务 | 域任务的 SPP 也是 Supervisor，`SPP` 单独不能区分（`trampoline.rs:7-11`） |
| 内核/任务栈 | 每核一条 trap 栈；任务栈是**用户**栈 | 「不能在自己正在用的栈上回收自己」；帧 PA 与栈解耦 |
| 退场收尾位置 | 由 `trap_handler` 的帧做 | 退场窄尾（`envcall.rs:155-160`） |
| 未知调用号 | `Denied` 并续跑，**绝不 panic** | a7 由 U 态完全控制，panic 即打死整机（`envcall.rs:183-189`） |
| `sepc` 前进量 | 按首字节低两位判 RVC | 固定 `+4` 曾跳过 `c.ebreak` 之后的指令（`envcall.rs:73-86`） |
| 只有 root 一个 spawn | 其余全由 root 派生 | `boot.rs:184-188`；清单解释权在 root（`initrd.rs:1-16`） |

## 9 · 已知边界

1. **历史坑（已修，注释即现场记录）**：`__utrap` 保存顺序错——`csrr t0, sscratch` 排在
   `sd x5, 0x58(sp)` 之前，把用户 `t0` 就地覆盖成用户 `sp` 再当「用户 t0」存帧；现象是每次
   用户陷阱返回后 `t0` 变成栈地址（`pc=0x4`、栈数据当返回地址一类随机崩溃）。修法是先存
   `x5` 再用 `t0` 做 scratch（`trampoline.rs:54-57` 的注释即此）。
2. ~~**疑似缺陷（高置信，未实机验证）**~~ —— **已修（本轮）**：判据右侧改用**本 hart 帧的
   `user_pa`**（新增 `hart_frame_pa()`），两侧从此同口径；修后 `persist` 与「S 态空闲恢复
   原上下文」两支路转为可达，完整门 `examine 5/5` 通过。原始现场记录保留如下：

   `trap.rs:99` 的 `from_task` 判据两侧不同源——
   `__core_trap` 传的 `a0` 是帧的**物理**地址（`trampoline.rs:154`、`context.rs:91`），
   而 `hart_frame()` 给的是**虚拟**地址（`machine.rs:199-210`、`layout.rs:79`）。两者永不相等
   ⇒ `from_task` 恒 `true` ⇒ `persist` 与「S 态空闲恢复原上下文」分支是死代码；后果是内核侧
   的同步异常会被当作**任务**故障、误杀无辜任务。测试看不见它（内核态恒 SIE=0，且内核任务面
   已删）。
3. ~~**注释残留旧名**~~ —— **已修（本轮）**：`establish_tp` → 「`trap_handler` 第 0 步」、
   `__strap` → `__core_trap`／「trap 入口」。原记录：`establish_tp`（`trampoline.rs:101,109`、`stack.rs:64`）与 `__strap`
   （`machine.rs:71,88,103,226`、`stack.rs:167,203`）在代码里不存在，真身是 `trap_stack_hart`
   与 `__core_trap`。
4. ~~**`context.rs:85` 与代码不符**~~ —— **已修（本轮）**（改为「两者恒为**执行核**的 trap 栈顶；
   任务没有自己的内核栈」）。原记录：说「用户帧的 `kernel_sp` ＝ 任务内核栈顶」，实际 `prepare`
   无条件写**执行核的 trap 栈顶**（`hart.rs:186`）——任务没有自己的内核栈。
5. ~~**`envcall.rs` 头部写「每个调用后 `sepc += 4`」**~~ —— **已修（本轮）**（改为「按实际
   指令长度前进，见 `instr_len`」）。原记录：它与同文件 `instr_len`（`:80-86`）
   的 RVC 判定矛盾。
6. ~~**`reap.rs:57` 称调用点之一为「envcall 的 `Exit`」**~~ —— **已修（本轮）**（改为 `Reap`）。
7. **`BadSlot` 无独立诊断码**：`from_wire` 的三种 `Decode` 在门面统一落成 `GateError::Denied`
   （`envcall.rs:186-189`），用户侧分不出「不存在的调用号」与「合法但被拒」。

## 10 · 判据与验证

- **四条总判据**（`scripts/examine.nu:8-12`）：逐步 expect → **自退非 124** → **无 panic**
  （捕 `[panic] at`）→ marker 齐全。
- **与本文直接对应的 marker**：`badslot: 3/3 rejected, kernel alive`（未知 slot 只被拒、
  内核续跑）；`badslot: 1/1 abnormal exit reaped, kernel alive`（带原因退场＝调用方死、
  内核活，覆盖退场窄尾与 `RoomCall::Reap`）；`task: all tasks exited, system halted`
  （帧与栈全部回收后的自然停机）。
- **halt 处的断言**（`boot.rs:150-175`）：胜出核广播后等 `HALT_ARRIVED == hart_count` 屏障，
  再按 messenger 观测 → `scheduler::core::rip` → block flush → audit 基线的次序跑钩子。
