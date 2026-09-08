# supervisor — S 态域（Supervisor 域）

> 一个**一等公民的 S 态执行单元**：有自己的 Team / Space / 页表 / ASID，从 ELF
> 装载、可调度、经 Pie 通信——而不是寄生在内核空间里的闭包任务。
>
> 本文同时记录这次改造的**结构裁决**（SpaceKind 收窄 + Asid 独立）与两条被
> 验证逼出来的 ABI 事实（`ebreak`、`sepc` 前进量）。

## 1 · 定位与信任模型

```text
                    特权级           页表            身份
  内核            S 态         KERNEL_TEAM 的空间    Asid(0)
  supervisor 域   S 态         域自己的空间          Asid(1..=65535)
  用户任务        U 态         域自己的空间          Asid(1..=65535)
```

**v1 的 Supervisor 域是可信域，不是沙箱。** S 态与内核同特权：可写 `satp` /
`stvec` / `sstatus`，可直接发 SBI 调用（`HSM::Start` 的入口是物理地址）。唯一
的硬件隔离机制是 PMP，而 PMP 只能由 M 态写——本机 OpenSBI 域配置为
`Region07: 0x0-0xffffffffffffffff S/U: (R,W,X)`，S 态对整个地址空间全权。

因此本次设计**只做结构**（域所有权、陷阱路径、入口、销毁、诊断），不做权限门。
「为驱动准备」的含义是**不设计成死角**：设备授权不进能力面（见 §7），但
`borrow_map`、Pie、Mail 都能原样复用。

## 2 · 结构：三条轴分开

`SpaceKind` 今天同时承担三件事，本次把其中两件拆出去：

| 轴 | 载体 | 服务的操作 |
|---|---|---|
| 特权模式（S / U） | `SpaceKind::{Supervisor, User}` | SPP、`tp` 约定、窗口 U 位 |
| 空间身份 | `Asid`（独立字段） | TLB 标记、`WaitKey`/`fence` 键命名空间 |
| 「是不是内核空间」 | `Asid::is_kernel()` | 陷阱路由、`persist`、闭包任务、诊断展开、`Drop` |

```rust
pub enum SpaceKind { Supervisor, User }   // sum：穷尽分派
pub struct Asid(usize);                   // 值：唯一性不变量（私有构造）

pub struct Space {
    inner: RelLock<SpaceInner>,
    kind: SpaceKind,
    asid: Asid,
}

impl SpaceBuilder {
    pub fn kernel() -> Self;      // Supervisor + Asid::kernel()（0，全局唯一）
    pub fn supervisor() -> Self;  // Supervisor + allocate()
    pub fn user() -> Self;        // User       + allocate()
}
```

- 分配器恒发 `1..=65535`（`BitmapAllocator::new(1, 65536, 1)`）；`Asid::kernel()`
  是唯一的 0 铸造点。
- `Space::drop` 无条件归还：`!is_kernel()` 时先 `fence::retire(asid)` 销账、再
  `asid::deallocate(asid)`（顺序契约：ASID 复用后键即换主）。
- 内核空间退化为 **`KERNEL_TEAM` 的所有权事实**，不再是枚举变体。
- 域 = `Team` + Supervisor Space：**不新增 `Domain` / `TaskKind`**。

### 为什么 ASID 不能和 SpaceKind 合并成枚举

`SpaceKind::Kernel => 0` 那种形状把约定伪装成一个值。两条代码级证据说明 ASID
不是「用户空间才有的标记」，而是**每个空间的身份键**：

1. `WaitKey::compose(asid, key)`（`envcall.rs`）——ASID 是 wait/wake 的命名空间。
   两个空间共享 ASID ⇒ 同一个 key 产生同一个 `WaitKey` ⇒ `Wake` 能唤醒别的域。
2. `fence::key(asid, va) = (asid << 44) | (va >> 12)` 且 `retire(asid)` 注销该
   ASID 名下**全部**用户堆账——共享 ASID ⇒ 键碰撞，且销毁一个域会连坐另一个域。

消费侧也几乎不相交：只有 `context::init` 同时读 mode（拼 sstatus）与 asid
（拼 satp），且用途不同。若把二者焊进同一个枚举，要么两变体载荷相同（product
伪装成 sum），要么保留「supervisor 没有 ASID」这条已被证伪的不变量。

## 3 · 陷阱路由

`__alltraps` 是唯一 `stvec` 目标；路由判据是**硬件状态**，不是约定：

```asm
__alltraps:
    csrr  t0, sstatus
    andi  t0, t0, (1 << 8)      # SPP
    beqz  t0, __task_trap       # 来自 U 态 → 任务路径
    csrr  t0, satp
    srli  t0, t0, 44
    slli  t0, t0, 48            # 只留 satp.ASID 16 位
    bnez  t0, __task_trap       # SPP=1 但不在内核空间 → S 态域任务
    j     __core_trap           # 否则：内核态陷阱
```

- **契约**：`satp.ASID == 0` ⇔ 被中断者是内核。
- **前置不变量**：内核代码只在内核空间执行（ASID 0）。今天成立——内核只在
  陷阱切回 `kernel_satp` 后运行。
- `__task_trap` 服务两类被中断者（U 态任务 / S 态域任务），存帧与切表序列**逐字
  相同**（`sscratch` = 自身帧 VA → 存 GPR → 读帧内 `kernel_satp`/`kernel_sp`/
  `trap_handler` → 切内核 satp）。差别只在保存下来的 `sstatus.SPP`。
- `__core_trap`（原 `__strap`）不切 satp，用 `tp` 取本 hart 帧。
- `__restore` 的 `sscratch` 复原同样按帧内 `user_satp.asid()` 判别：0 → 本 hart
  帧 VA；≠0 → 线程帧 `self_va`。与入口判据同源。

### `from_task`：为什么不能用 SPP 判「是不是任务」

S 态域任务的 `sstatus.SPP` 也是 Supervisor，与内核陷阱无法区分。`trap_handler`
改用**帧身份**：

```rust
let from_task = (frame as *const TrapContext as usize) != machine::hart_frame().as_usize();
```

`__core_trap` 传本 hart 帧、`__task_trap` 传任务帧——这正是路由判据的产物。据此：

| 分支 | 内核（`!from_task`） | 任务（`from_task`） |
|---|---|---|
| S-timer | `persist` 后抢占；空闲则恢复原上下文 | 现场已在任务帧 → 直接 `run()` |
| 缺页 | 内核 bug → panic | 解析成功续跑，失败杀 task |
| 其它异常 | 内核 bug → panic | 杀 task（fault isolation） |
| `ebreak` | 内核自身 ebreak = bug → panic | 环境调用分发 |

## 4 · uABI：环境调用是 `ebreak`，不是 `ecall`

**RISC-V 的 `ecall` 语义随特权级变化**：

| 来源 | scause | 委派给 S 态？ | 结果 |
|---|---|---|---|
| U 态 | 8 (UserEnvCall) | 是（`medeleg` 位 8） | 进内核 envcall 分发 |
| S 态 | 9 (SupervisorEnvCall) | **否**（OpenSBI 清零位 9） | **进 M 态固件，被当 SBI 调用** |

本机 `medeleg = 0x0000_0000_00f4_b509`：位 3（Breakpoint）为 1、位 9 为 0。
所以域任务用 `ecall` 发环境调用会**静默变成一次失败的 SBI 调用**（返回值当错误
码，陷阱根本不进内核）——表现为 `tls_bootstrap` 的 `alloc().expect()` 失败、
panic、最后落在 `room::exit` 的 `unimp` 上。

**决定：`warpper` 的陷阱指令改为 `ebreak`**（`crates/ubi/src/ucall.rs`）。
`ebreak` 在 U 态与 S 态都被委派给 S 态，两类任务共用同一入口，无需按模式分派
wrapper。内核侧 `trap_handler` 处理 `Exception::Breakpoint`。

> 语义名仍然成立：它就是一个"环境调用"陷阱；`fid.rs` 的 `#[derive(Envcall)]`
> codec、a7=slot、a0..a5 参数、负值即错误码（D1）全部不变。

## 5 · `sepc` 前进量：`c.ebreak` 是 2 字节

`ebreak` 有两种编码：标准 4 字节 `0x00100073` 与 RVC 压缩 2 字节 `0x9002`
（汇编器在开 RVC 时发后者）。旧的 `frame.sepc += 4` 会**多跳一条 2 字节指令**：

- release 档：`warpper` 的 ebreak 后面紧跟 `ld ra, 0x8(sp)`——被跳过的恰是它，
  而 `ra` 本就没被 `warpper` 改写（caller 的 `jalr` 设的值仍有效），于是**侥幸
  可用**；
- debug 档：跳过的是必需指令，控制流错位，表现为"6 字节分配失败"一类假象。

修法：按指令首字节低两位判长（`!= 0b11` ⇒ 2 字节），首字节经目标空间翻译后读：

```rust
fn instr_len(space: &Space, sepc: KVirt) -> usize {
    let b0 = space.translate(sepc)
        .map(|(pa, _)| unsafe { core::ptr::read_volatile(pa.as_usize() as *const u8) })
        .unwrap_or(0b11);
    if b0 & 0b11 == 0b11 { 4 } else { 2 }
}
```

## 6 · 装载：U 位是映射策略，不是 ELF 语义

- `parser` 只产 ELF 的 R/W/X（删掉原先硬编码的 `PteFlags::U`）。
- `loader` 按 `space.kind()` 决定 U 位：`User` ⇒ 带 U；`Supervisor` ⇒ 不带。
- 窗口同理（`stack` 删掉 `kernel: bool` 参数，`heap`/`share` 按 kind 推导）。

**为什么 S 态页不得带 U**：新任务 `sstatus` 起步为 0（`SUM=0`），S 态访问 U=1
的页会缺页——域任务一用堆/栈就崩。域空间整片 S-only 是最省心的选择（代价：域
不能在自己的空间里再起 U 态子任务，v1 不需要）。

`assemble(elf, sire, kind)` 是唯一装载入口：`Supervisor` ⇒
`SpaceBuilder::supervisor()`，`User` ⇒ `user()`。

## 7 · 设备授权不进能力面（Pie 冻结）

Pie 已冻结，`AnyPie` 仍 `Hole | Pole`。设备（MMIO 区间 + 中断号）**不新增
资源种类**，理由：

- `PoleMeta::allocate` 从帧分配器取零页、`Drop` 把帧还给分配器——设备内存既
  不该清零也不能归还，"Pole over MMIO"是语义污染；
- 可信域本就不需要能力门：设备访问权来自**建域时的映射供给**
  （`borrow_map(va, device_pa, size, flags)` 已是通用入口）。

**必须写进信任边界的后果**：设备权**不可转授、不可单独收回**，撤销 = 销毁域。

## 8 · 引导：initrd 小清单（临时机制）

`kernel/src/initrd.rs` 与 `machine` 同级（平台/引导供给层），**标注为临时机制**：

```text
initrd.img
  [0..4]  count     u32 LE  1..=8
  每条：
    [0..4]  name_len  u32 LE  1..=32
    [..]    name      ASCII，无 NUL
    [0..4]  len       u32 LE  >= 1
    [..]    bytes     ELF 原样字节
```

- 无 magic：旧格式（裸 ELF）前 4 字节 `0x464c_457f` 远超 `MAX_PROGRAMS`，被
  `TooMany` 当场拒掉。
- boot 按名取（`take(&programs, "shell")` / `"echo"`），顺序无关；未知名打印
  清单后 panic。
- `build.rs` 的 `INITRD_BINS` 是打包清单，与 boot 的供给表成对。
- **退出路径**：程序投递一旦有正式通道（运行期装载原语 / 设备发现），本模块
  连同打包端一起删除。
- `build.rs` 必须 `rerun-if-changed=../crates/ubi`——否则改 ubi 后 initrd 不重
  打包，内核重编而用户程序是旧的。

**域怎么拿到自己的入口门闩**：沿用根授予——boot 把入口 hole 的 Pie 放进域任务
权限表索引 0，域程序用 `Collect(0)` 取回（与 shell 取目录门闩同款）。uABI 无
任何新入口。

## 9 · 已决 / 被否

| 提案 | 定论 |
|---|---|
| `SpaceKind::{Kernel, User{asid}}` | 拆开：kind 只答特权级，ASID 独立字段 |
| 新类型 `Domain` / `TaskKind` | 否决——`Team` 已是"围绕共同 Space 的 Task 集合" |
| 设备 = Pie 第三种资源 / Pole-over-MMIO | 否决（Pie 冻结 + Pole 语义不合） |
| `__strap_task` 独立入口符号 + 每上下文 `stvec` | 否决——改 `satp.ASID` 判别，复用 `__task_trap` |
| `sscratch` 哨兵（0 = 内核） | 否决——该 CSR 在嵌套陷阱下本就被文档标记为不可靠 |
| 内嵌 `include_bytes!` / `-device loader` + 引导参数 / 运行期装载 | v1 取小清单；其余见 §8 退出路径 |
| 用 `SPP` 判内核/任务 | 否决——S 态域任务 SPP 也是 Supervisor |
| `frame.sepc += 4` | 否决——见 §5 |

## 10 · 已知边界

1. **域态 echo 忙等**：用户态拿不到 hole 的 wait key（键 = `HoleMeta` 地址、
   命名空间 asid 0，见 `work/mail/hole.rs::pull_key`），故服务用短 spin 轮询而
   非 park，空闲时占满所在核。根治需暴露 hole 等待键或加 wait-on-hole 原语。
2. **域内自持陷阱未做**：v1 所有陷阱上交内核（`stvec` 仍是 `__alltraps`）。
   结构上留位——域改自己的 `stvec` 不需要动内核结构。
3. **设备/中断/DMA 未接入**：设备清单（`machine::Info.uart/plic/clint` 恒 0）、
   PA 可见性、中断路由均未做。
4. **建域仍限内核**：用户态建域是提权原语，Pie 冻结下没有门控位，v1 不开放。
5. **域不可转授设备权**：见 §7。

## 11 · 验证

```bash
$ ( sleep 3; printf 'dir\n'; sleep 2; printf 'req\n' ) | QEMU_TIMEOUT=22 cargo run --release
SQware shell
sq > dir
discover echo -> found
  echo
sq > req
req echo -> "ifmmp.tfswjdf…"        # 域态 echo（S 态页表 + 独立 ASID）逐字节 +1
```

debug 档同路径跑通，且 `health spare / pagetable / stress` 全 ok。

## 12 · 文件清单

```
新增  kernel/src/initrd.rs                    小清单解析（临时机制）
新增  user/src/bin/echo.rs                    域态 echo 服务
改写  kernel/src/work/unit/space/mod.rs       SpaceKind{Supervisor,User}
改写  kernel/src/work/unit/space/core.rs      Space.asid + 三构造器 + Drop 统一
改   kernel/src/memory/manager/asid.rs        Asid newtype（kernel/allocate/…）
改   kernel/src/work/unit/space/window/*      窗口 U 位随模式
改   kernel/src/work/unit/parser.rs           去 U 位
改   kernel/src/work/unit/loader.rs           按 kind 加 U
改   kernel/src/work/unit/mod.rs              assemble(elf, sire, kind)
改   kernel/src/runtime/switcher/trampoline.rs  __core_trap/__task_trap 路由 + restore
改   kernel/src/runtime/switcher/trap.rs      from_task 判据 + Breakpoint 分发
改   kernel/src/runtime/switcher/envcall.rs   instr_len（sepc 前进量）
改   kernel/src/runtime/switcher/context.rs   SPP 按 kind、satp 按 asid 字段
改   kernel/src/work/unit/task.rs             闭包断言按 is_kernel、栈窗去参
改   kernel/src/work/room/scheduler/core.rs   tp 约定按 is_supervisor
改   kernel/src/runtime/diagnose/{scene,frame}.rs  world 随新枚举
改   crates/ubi/src/ucall.rs                  ecall → ebreak
改   kernel/build.rs                          小清单打包 + watch ../crates/ubi
改   kernel/src/boot.rs                       装载两域 + 根授予 + 绑定
改   user/Cargo.toml                          +user-echo
```
