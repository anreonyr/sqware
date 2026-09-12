# abi — ABI 面与分层

> 路径约定：`文件:行` 相对仓库根。内核侧入口见 [switcher.md](switcher.md) §4。

## 1 · 语义定位

五层单向，外加一条侧边：

```text
kernel → env → runtime → protocol → programs
   └──→ sbi                    （S → M 的另一张 ABI 面）
```

| crate | 一句话 |
|---|---|
| `kernel` | 只认 ABI 与固件：`kernel/Cargo.toml:21-24` 的依赖表里只有 `env` 与 `sbi`，**没有** `runtime`/`protocol` |
| `crates/env` | ABI 的线格式与调用骨架：slot 编码、载荷 codec、线类型、唯一汇编入口；U 态任务与 S 态域任务共用（`env/src/lib.rs:2-3`） |
| `crates/runtime` | 镜像侧机制：`env` 薄转发 + `core` 组合封装（`runtime/src/lib.rs:5-12`） |
| `crates/protocol` | 用户态协议语义（目录 / 控制台），用 runtime 的机制（`protocol/src/lib.rs:4-18`） |
| `programs` | 装配层 bin：机制来自 runtime、语义来自 protocol（`programs/src/lib.rs:5-6`） |

**「内核零引用 protocol」不是纪律而是构造**：`kernel/src` 全树对 `protocol` 零命中，唯一命中是
`kernel/build.rs:107` 的 `rerun-if-changed=../crates/protocol`（initrd 重打包 watch，不是代码
引用）。反方向也各自声明过：`runtime/src/core/mod.rs:3-4`「本模块对 protocol 零引用」、
`runtime/src/lib.rs:11-12`「不认识任何协议」。

## 2 · manifest 里写下的裁决

| crate | manifest 裁决 |
|---|---|
| 根 `Cargo.toml` | 7 个成员、`default-members = ["kernel"]`（`:12`）；`profile.dev panic = "abort"`（`:17-19`——正是它让工作区里的 `cargo test` 建不起来）；`profile.harden` ＝ release + `debug-assertions`（`:21-27`，「只多开这一个开关」）；`opt-level = 2` 与 `WaitKey::compose` 的 mask 多算 +1 规避（`:29-38`） |
| `crates/env` | `test/bench/doctest = false`（`env/Cargo.toml:6-9`，无理由注释）；依赖只有 bitflags / erra / envmacros |
| `crates/envmacros` | `proc-macro = true`：`derive(Envcall)` 生成 `slot` / `pack` / `from_wire` / `*Ret` / `call` |
| `crates/runtime` | 「薄/厚的判据不是行数」＋目标依赖方向（`runtime/Cargo.toml:6-8`） |
| `crates/protocol` | `test = false` 的**实测**理由：`no_std` + riscv64 **编不出 libtest**（`:6-11`）；`→ runtime` 这条边是「机制在运行时、语义在协议」的编译期形态（`:19-21`）；`anstyle-parse` 只服务服务侧的 VTE 解码（`:24-26`） |
| `kernel` | `semihosting` 与 `framework` 都**非默认**（`kernel/Cargo.toml:6-30`，两个门的分工见该文件的门注） |
| `programs` | `INITRD_BINS` 是**唯一**声明特权级的地方（`programs/Cargo.toml:6-8`） |

## 3 · ABI 面

七个 class（`crates/env/src/fid.rs:13-15`），按**操作的归属轴**分。
**原 class 3（`IO`）已删**——设备面搬出内核（`docs/driver.md` §10 第三步），号段空着不补：

| class | 名 | 轴 |
|---|---|---|
| 0 | Room | 调度词族：`Starve` `Park` `Reap` `Wait` `Wake` `Doom`。<br>`Reap { reason, note, len }`：`reason` = 退出原因码（数据，内核只记不解），`note` = 域自己带的一句话（`VirtAddr(0)`+`len=0` = 无话，内核在入口 `copy_in` 至多 `env::NOTE_MAX` 字节并**自己打印**——不依赖任何服务活着） |
| 1 | Unit | 执行单元：`Spawn` `SelfId` `Sire` `HeirCount` `Heir` `Build` `Hatch` `Join` |
| 2 | Memory | `Allocate` `Deallocate` `Mmap` `Munmap` `Mprotect` |
| ~~3~~ | ~~IO~~ | 已删（`Put` `Get` 随设备面搬出内核；号段空着，见下） |
| 4 | Chrono | `Ticks` `Clock` |
| **5** | **Mail** | **数据轴**：`Push` `Pull` `Wait`（传**内容**） |
| 6 | Control | `Backtrace` |
| **7** | **Pie** | **权柄轴**：`Unseal*` `Seal` `Open` `Shut` `Accord` `Narrow` `Revoke` `Collect` `Reserve` `Release`（传**许可**） |

**拆轴的判据写在臂的调用集合里**：数据轴的臂从不调 `gate` 的权柄函数，权柄轴的臂从不搬运
载荷——原先 12 个操作同居 class 5，是两条轴的混合（`fid.rs:17-26`）。class 7 复用的是原
`ServiceCall` 腾出来的号位。

```text
slot = (class << 32) | index      index = 变体在枚举里的**声明顺序**
```

编码由 derive 现算（`envmacros/src/lib.rs:144-157` 的 `( #class << 32 ) | #i`），解码
`class = slot >> 32`、`index = slot & 0xFFFF_FFFF`（`:277`）。**index 是声明顺序判别号 ⇒
重排 variant 就是改 ABI**（`fid.rs:6-7`），没有版本号、没有冗余校验。用户侧不再构造
`EnvCall`（`fid.rs:372-376`）；内核侧唯一解码入口是 `EnvCall::from_wire`（`fid.rs:391-404`），
调用点在 `kernel/src/runtime/switcher/envcall.rs:186`。

## 4 · codec

- `Wire::pack/unpack`（`env/src/wire/mod.rs:25-31`），失败域
  `Decode{BadSlot, Overflow, Invalid}`（`:34-42`）。
- **分派**：class 不匹配 → `from_wire` 末尾 `_ => Err(BadSlot)`（`fid.rs:402`）；域内 index
  不匹配 → derive 的 `_ => Err(BadSlot)`（`envmacros/src/lib.rs:279-281`）；字段多于 `a0..a5`
  → `Overflow`（`wire/mod.rs:52`）；非法位 → `Invalid`。
- **`FromPair` 的职责**：把内核回写的 `(a0, a1)` 蒸馏成 `#[ret(T)]` 的载荷，由 `call()` 的
  非负路径调用（`wire/frompair.rs:28-35`）；错误路径归 `EnvError::from_raw`
  （`envmacros/src/lib.rs:298-308`）。
- **口径分工**（`frompair.rs:7-24`）：输入面（用户可控的 `a0..a5`）**拒绝**；回写面**按契约
  取位**（例：`Collect` 的 `v1` 低 32 位是 permission、高 32 位是 vestor，`:81-89`）。
- **曾经唯一「纯靠类型收窄、无契约背书」的地方（`FromPair for u8`）已随 `IOCall` 删除**：
  那个收窄只为 `IOCall::Get` 存在（成功时 `a0 ∈ 0..=255`，靠内核实现承载），而 `Get` 在
  `docs/driver.md` §10 第三步随设备面一起删了 ⇒ 该 impl 与它的 debug 断言一并删除。
  于是本 crate 现在**没有**"只靠类型收窄"的字段：每个 `FromPair` 都有契约或位打包背书。
  **class 3 号段空着不补**（判别号 = 声明顺序，挪号只制造无意义的 ABI 位移）。

## 5 · 用户侧的薄与厚

- **薄的边界**：一次调用一个函数、零业务逻辑（`runtime/src/env/mod.rs:1-2`；`env/mail.rs:33`
  自称「裸函数层」）。
- **厚的边界**：组合与封装（`runtime/src/core/mod.rs:11-13` 明写「`HolePie` 是薄句柄；
  `Channel` 是厚生命周期封装」）。

同一个 `Push` 在三层的样子：

```text
薄   mail::push(token, msg, len)     一次 MailCall::Push，Busy 原样返回
中   HolePie::push                   在 Busy 上转 wait(Push, usize::MAX) 循环
厚   Channel::open / close           = unseal + accord(R|W) / revoke + release
```

同类：`core::unit::closure` ＝ `spawn` + `hatch` + `Completion` 两位置位仲裁
（`core/unit.rs:106-143`）；`core::heap::Heap` 直接装 `#[global_allocator]`（`heap.rs:52-53`）。

## 6 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| `a7` 是**输入**不是标识：未声明 class/index → `BadSlot` → 写负码并**续跑** | 用户一发 `ebreak` 打死整机 | `fid.rs:28-36`；`kernel/.../envcall.rs:183-189` |
| `trap` 必须 `#[inline(never)]` | 内联后读回的返回值错（实测 a0 恒 0） | `env/src/ecall.rs:63-67` |
| index ＝ 声明顺序 | 老程序打到新原语上 | `envmacros/src/lib.rs:144-157,277-282` |
| 非法位与超宽位必须拒，不得截断 | `0x1_0000_0002` 静默变成 `WRITE` | `wire/mod.rs:124-140` |
| 非法名不可表达（非空、≤ 31 B、无 NUL、UTF-8） | 填充与内容歧义、错误域错标 | `wire/name.rs:43-78` |
| `Release` 不过存活闸 | 封印后表项永远摘不掉 | `fid.rs:350-355` |
| 负值即错误、非负即成功（`-1..-6`） | 用户把错误当值用 | `ecall.rs:31-48` |
| 发送者由内核盖章，`owner` 不随转手改写 | 身份可伪造 / 认错对端 | `fid.rs:243-247,341-349` |

## 7 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| 协议搬出 ABI crate | 新建 `crates/protocol` | env 里躺着 284 行内核读不到的协议 |
| protocol 依赖 runtime 而非 env | 要用机制（`HolePie` / `Channel`） | 「机制在运行时、语义在协议」的编译期形态 |
| class 5 / 7 拆轴 | Mail ＝ 数据、Pie ＝ 权柄 | 正交判据在臂的调用集合里 |
| `Doom` 落 class 0（词族 doom） | 与 `Reap` 同族成对：**自杀 ↔ 他杀** | 追加在末尾——判别号 = 声明顺序，插中间即改 ABI（`fid.rs:78-94`） |
| 他杀**不进** `UnitCall` | 判据是血缘（domain 轴），不是执行单元 | `Join`/`Spawn` 那一轴管"产/放行/等"，"杀"与 `Reap` 同族 |
| 返回类型锁死 | 每 variant 一个 `#[ret(T)]` → 生成域 `*Ret` | `call()` 负值判译、非负蒸馏（`fid.rs:9-11`） |
| `from_pair` 不返 `Result` | 回写面按契约取位，纯收窄只在 debug 档查 | 内核违约不该塞进「内核→用户」的错误域 |
| `test = false` | 不是「不该测」 | `no_std` + riscv64 编不出 libtest |
| `default-members = ["kernel"]` | 裸 `cargo build` 只出内核 | 根 `Cargo.toml:12` |
| 负码空间所有权 | 内核 `-1..-6`；协议自 `-7` 起自取 | `ecall.rs:31-40`、`protocol/src/dispatch/client.rs:37-45` |

## 8 · 已知边界

1. ~~**「index 5 是空号」是错的**~~ —— **已修（本轮）**：`fid.rs` 三处注释与 `README.md` 的同一
   说法都已改写。原记录：`Build` 就坐在 index 5（声明顺序第 6 个），`Hatch` / `Join`
   在 6 / 7——`UnitCall` 的 index `0..=7` **连续无空号**。三处陈注释：`fid.rs:90-91`、
   `fid.rs:121`、`fid.rs:30-31`（把「class 1 的空号 index 5」当作 `BadSlot` 的例子）。
   机制上也必然：**index 是声明顺序，注释占不住槽位**——没有变体就没有号。本轮已从
   `README.md` 的 ABI 表里删掉同一说法（它此前照抄了那条注释）。
2. **门的 `badslot` 探针第一发打的不是空号**：`programs/src/bin/user/shell.rs:1063` 发的
   `0x1_0000_0005` 解码成 `Build{...}`（6 个字段恰好占满 `a0..a5`），随后被**建域权门**拒。
   门只断言 `3/3 rejected`（`examine.nu:139,167`）⇒ **无法区分 `BadSlot` 与「合法但被门拒」**。
3. ~~**`crates/env/src/permission.rs:3-9` 两处陈旧**~~ —— **已修（本轮）**：改为 `Narrow` 口径 +
   「未申明的位一律拒绝」。原记录：`RESTRICT` / `restrict(...)` 这个术语在
   代码里不存在（ABI 动词是 `Narrow`）；「内核侧 `from_bits_truncate` 还原」与
   `wire/mod.rs:124-140` 的拒绝式 unpack 相反。
4. **`frompair.rs` 有三枚零消费者的 impl**：`u64`（`:54-58`）、`(PieToken, PieToken)`
   （`:66-70`）、`(PieToken, Permission)`（`:72-79`）——蒸馏面上没有生产侧的臂。
5. **`runtime/src/env/room.rs:7-10,32-43` 的 `starve`/`sleep`/`wait` 用 `let _ = ….call()`**
   吞掉结果恒返 `Ok(())`（`exit_with` 同），与「封域 Ret、零业务逻辑」的读法有张力；
   今日内核在这四条上从不写负码，故该路径不可达。
6. **`kernel/.../envcall.rs:573-585`**：两条轴的 dispatch 返 `None` 时，门面**落到函数末尾
   静默续跑**（不写 `a0`）。今日两个 match 都穷尽，故不可达，但没有编译期保证。
7. **`crates/sbi` 与 `env` 不同源**：手写 `#[repr(usize)]` FID + `as usize`
   （`sbi/src/fid.rs:1-4,194-219`），返回约定也不同（`a0` ＝ 错误码、`a1` ＝ 值）；
   env 侧两处注写作「sbi **未来**可复用同一 derive」，今日未复用。
8. **宿主侧单测无落脚点**：全仓 lib/bin 都是 `test = false`；一次性 `crates/testhost` 用完即删。
   `protocol/Cargo.toml:6-11` 明说「宿主侧单测需要一个宿主 crate——那是独立一步」。
9. ~~**`kernel/.../envcall/pie.rs:1,41` 说「十一个操作」**~~ —— **已修（本轮）**：改称十二个，并把
   「六个操作走 `resolve`」那句一并改成实情。原记录：`PieCall` 现有 **12** 个变体
   （`UnsealNole` 后加，晚于那条注释）。

## 9 · 判据与验证

- **`badslot`**（`shell.rs:1056-1070`）：不走 `EnvCall` 解码，直接进唯一汇编入口
  `env::ecall::trap`，三发 `[0x1_0000_0005, 0x8_0000_0000, usize::MAX]`，数 a0 < 0 ⇒
  `badslot: 3/3 rejected, kernel alive`；紧接一条对照（`:1082-1098`）用一次性子任务跑
  `room::exit_with(0x5A5A)`，**按终态不按钟**等它结束后打 `badslot: 1/1 abnormal exit reaped,
  kernel alive`。
- **门覆盖的六条 ABI 性质**：①非法 slot 被拒且内核活（`:167`）；②合法退场只终止本域
  （`:171`——它的头注说这是「域级退场曾实现成 `panic!`」留下的牙）；③数据轴往返
  `hole got "hi from shell`（`:163`）；④权柄轴 `seal` 唤醒等待者（`:185`）；⑤非法 id 的
  `Join` ⇒ `Denied`（`:186`）；⑥协议往返经 ABI `req echo -> "ifmmp…"`（`:161`）。
- **未覆盖**：7 个 class × 全部原语的逐条往返（class 3 已删，故不再是 8；**空号段**
  现在也无人探针——`a7 = 3<<32` 走的是同一条 `BadSlot` 拒码，与 `badslot` 的三发同类）；`BadSlot` 与 `Denied` 的区分；`Wire` 的
  `Invalid`/`Overflow` 拒绝面在真机日程里没有探针（只在已删的一次性宿主测试里出现过）。
