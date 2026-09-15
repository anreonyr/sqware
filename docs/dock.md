# dock — Pole 的 runtime 封装（Dock · View）

> 路径约定：`文件:行` 相对仓根。数据面三件套在 [mail.md](mail.md)，权柄模型在
> [pie.md](pie.md)，设备怎么用这段内存见 [driver.md](driver.md)，
> Hole 的封装见 [port.md](port.md)，Nole 的封装见 [bell.md](bell.md)。
>
> **状态：全部已实现，门绿**（默认档 17 步 / 23 marker，另两档 21 步 / 28、29）。
>
> **落地位置**：`crates/runtime/src/core/dock.rs`（`Dock` + `View`）；
> ABI 的改动在 `crates/env/src/fid.rs`（`UnsealPole { size }`、`Open → (VirtAddr, usize)`）；
> 内核侧 `kernel/src/work/mail/pole.rs` 与 `kernel/src/runtime/switcher/envcall/pie.rs`；
> 消费者 `programs/src/uart.rs`、`programs/src/plic.rs` 与其装配点。

## 1 · 语义定位

`Dock` 不是内核对象，是**镜像侧的用法**：把一枚 Pole 门闩**借映进本域**，得到一段视图。
内核里它仍是一枚 Pole（`AnyPie::Pole`，没有第四种资源）：页块、`Open`/`Shut`/`Narrow`
都在内核，"怎么用"封装在这一层。

| 管 | 不管 |
|---|---|
| 借映（`open`）、撤图（`shut`）、**视图的两半**（`base` / `size`） | 页块本身与权柄判定（内核）、设备寄存器怎么读（驱动）、共享内存的布局与同步（上层协议） |

**Dock 不认识设备**：`docs/driver.md` §8 裁过"不是设备类框架"——给一段内存，`Uart` 逐字节
读、`Plic` 按 u32 读，两台各写各的，只保证同形。

## 2 · 三件各加一样东西（这是"正交"的具体含义）

```text
Port   两枚 Hole   加**配对**   往哪推、从哪收、对面是谁（+ 来源校验，绕不过）
Dock   一枚 Pole   加**成对**   起点与长度成对的一段内存
Bell   一枚 Nole   加**约束**   三拍 ring / wait / hush（签名少了"方向"这个参数）
```

三件都持自己的门闩（`Port` 持两枚、`Dock` 与 `Bell` 各持一枚），因为**它们的动词就是权柄
动词**：`Dock` 的 `open`/`shut` 与 `Bell` 的 `wait`/`hush`/`ring` 一样，都要过一次
`resolve(token, …)`。

**本层不加动词**：`open` / `shut` 就是 `PieCall::Open` / `Shut`，与 `PolePie` 同名同形。
多出来的只有一件事——**把"起点 + 长度"成对地带回来**。`Dock` 因此很薄；这不是缺陷，
`docs/bell.md` §5 已经把这条立成规矩：同一个动作多一层命名与一条约束，就是这一层的全部内容。

**零新 ABI、零新 envcall、零新错误码**。唯一的 ABI 改动是 §4 那一条。

## 3 · 为什么视图是**另一个类型**

映射与视图的**可复制性相反**：

- 映射不该被复制——撤图（`shut`）要消费它，`Dock` 因此不 `Copy`（与 `Port` / `Bell` 同）；
- 视图**必须**能被复制——`supervisor/uart.rs:70` 的 `static UART: Lock<Option<Uart>>` 配
  `Uart: Copy`，两个线程各取一份（同一张页表，不需要锁）。这是**既有的用法**，不是新需求。

一个类型兼任两件事，就必然要在一个方向上撒谎。故拆成两个：

```rust
pub struct Dock { pie: PolePie, view: View }   // 映射：不 Copy，能撤
#[derive(Clone, Copy)]
pub struct View { base: usize, size: usize }   // 视图：两半成对，可复制
```

`View` 的两半**私有**、只能一起取走（`Dock::view()`）——这与 `To { peer, seed }` 是同一条
理由：起点与长度单独一个数拿出去没有意义。且**长度由内核给**（§4），调用方报不出来，
所以"用 `reg` 的 0x100 当 4096 用"这类越界不再有一个入口。

## 4 · ABI：`Open` 返两件

```
PieCall::UnsealPole { size }        解封（大小页对齐）
PieCall::Open { token } -> (VirtAddr, usize)   借映 → 起点 + 整段多大
```

**为什么长度必须一起返**：它只在内核手里。外来区（设备 MMIO）按页界向两侧撑开，故
UART 的 `reg` 只有 `0x100` 而映射出来是 4096——`reg` 的长度内核根本不知道。分开取就等于
把"这段有多长"留成调用方的猜测，而这个猜测错了就是越界踩到同页邻居。

**通道不用扩**：返回值恒为 `(a0, a1)` 两个寄存器（`crates/envmacros/src/lib.rs:299`），
`Collect` / `Reserve` 早就在用 a1。新增的只是
`crates/env/src/wire/frompair.rs` 里 4 行 `impl FromPair for (VirtAddr, usize)`。

**兼容性**：程序 ELF 由 `kernel/build.rs` 用嵌套 cargo 打包，内核与镜像恒同时重建，故不需要
过渡形状（与 `fid.rs` 头注里 `debug_assertions` 那条"会分叉的 ABI"是两回事）。

**顺带改的名**：这一族里的"字节数"统一叫 `size`（`UnsealPole { size }` /
`PolePie::unseal(size)` / `PoleMeta.size`）。`PoleMeta::region` 的参数则改叫 `reg`——那是
**设备树声明的那一段**（所有权粒度），与 `size`（映射粒度，页对齐、只大不小）不是同一个量，
此前两者同名是歧义。

## 5 · 签名

```rust
impl Dock {
    /// 映射：借映一枚已授权的共享页 → 视图。收下门闩。
    /// Errors: `Denied`（不在本任务表里 / 无 READ）、`Dead`（已封印）、`OoM`（备不出段）
    pub fn open(pie: PolePie) -> EnvResult<Dock>;

    /// 把视图拷一份交出去（`View` 是 `Copy`）。
    pub fn view(&self) -> View;

    /// 撤图（幂等）。**不 release 门闩**——它与 self 一起放下。
    /// Errors: `Denied`
    pub fn shut(self) -> EnvResult<()>;
}

impl View {
    pub fn base(self) -> usize;   // 视图起点
    pub fn size(self) -> usize;   // 视图长度 ＝ 映射的那一段（页对齐）
}
```

**`shut` 不 `release` 门闩**：与 `Port::shut` 逐字同一条理由——门闩的收尾是**策略**，不进结构
（[port.md](port.md) §8.2）。

**谁持有 Dock**：**要撤图的那个人**，即装配层，不是驱动。`Uart` / `Plic` 只持 `View`，
形状零变化（`Uart` 仍 `Copy`、仍住 `static`、仍两个线程各取一份）。

```rust
let dock = Dock::open(PolePie::from_token(dev.token()))?;  // 装配层：开一次
let uart = Uart::new(dock.view());                          // 驱动：只拷视图
```

## 6 · 不变量与失败域

| 不变量 | 违反会怎样 | 谁守 |
|---|---|---|
| 视图只可能来自 `Open` | 拿一段没映射的地址 load/store | **类型**：`View` 两半私有，只有 `Dock::open` 造得出来 |
| 起点与长度成对，且长度是内核给的 | 用 `reg` 的长度当 `size` 用（越界踩同页邻居） | 类型（一个值）+ 来源（`Open` 的 a1） |
| 映射的键是 **per-pie token**，不是 per-space | 同一域两个任务各映射同一枚 Pole ⇒ **两个不同的 VA**；往页面里放地址就成了错 | 内核（`pole.rs:53-55`）。故**页面里不许放地址** |

**守不住的一条，如实记**：撤图之后不得再用。`View` 是 `Copy`，`base()` 是裸数，可以被复制、
存进 static、加偏移——`shut(self)` 消费掉的只是**我手里这份 `Dock`**。要真守住，地址就得从
不以裸值离开本层（闭包，或带界访问器），而那与驱动的形状（`&self` 方法跨调用持基址）冲突。
**故 `shut` 的身份是礼节，不是证明。**

**失败域**：`open` → `Denied` / `Dead` / `OoM`；`shut` → `Denied`（见 §7）。

**无核心/适配之分**：无表、无状态机、无失败中间态——与 `Bell` 同形，不适用那一刀。

## 7 · 裁决：`Shut` 不过存活闸

`Shut` 是**唯一**与 `Release` 并列的"不判存活"的操作，两处各改了一行：

| 处 | 改动 |
|---|---|
| `kernel/src/work/mail/pole.rs` | 删 `if !meta.alive() { return Err(Dead) }` |
| `kernel/src/runtime/switcher/envcall/pie.rs` | `shut` 不走 `resolve`（它含存活闸），改走 `find` + 判权 + 判「被关住」 |

**理由**：撤的是**调用方自己那张 PTE**，与资源活不活着无关。`Release` 的先例就在同一份 ABI
里（`fid.rs` 的 `Release` 那一格）：「**唯一不判存活的操作**——`Seal` 不摘表项，若它也判存活，
封印后的表项就永远摘不掉。语义 =「你总得能放下手里的东西」」。`Shut` 是同一句话：
**你总得能撤掉自己那张图。**

**证据本来就在手边**：内核唯一的自动撤图点 `kernel/src/work/unit/gate/cull.rs:114` 写的是
`let _ = pole::shut(&meta, token);`——那个 `let _ =` 正是在把这道闸产生的 `Dead` 主动丢掉，
调用方想要的语义与这道闸正好相反。（这条改动之后 `let _ =` 仍保留：`space.release` 还可能
`Denied`，而它是收尾路径。）

**权限位照旧要 `R`**：与 `Release` 的"不要任何权限位"不同——撤图仍是 Pole 的数据面动作。

`docs/mail.md` §10 第 1 条那条边界（「Pole 封印后借入映射撤不掉」）就此销账，
判据是 §9 的 `sealed-shut=1`。

## 8 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| `Dock` 该不该立 | **立**（用户裁决） | 三件正交的要求：`runtime::core` 里除了任务本地原语，Mail 之上只能有三件封装，各包一种 primitive |
| `Dock` 是"PolePie 换名"还是"映射出来的视图" | **视图** | 换名净增 0，三件里唯一白立的一个；视图则把"两半成对"变成类型，并消掉驱动侧两句 `// SAFETY: base 是本域已映射的设备页` |
| 视图与映射一个类型还是两个 | **两个**（`Dock` / `View`） | 可复制性相反：映射不该复制（`shut` 要消费它），视图必须能复制（`static UART` 两个线程各一份） |
| 视图长度从哪来 | **`Open` 一起返**（用户裁决 (b)） | 它只在内核手里；分开取 = 留成调用方的猜测。与 `Port::pull` 返"恰好那一帧"同一个动作 |
| `Dock::open` 收下门闩还是借一枚 | **收**（用户裁决） | 与 `Port` / `Bell` 同构；今天没有一处同时要"开视图"和"再授出"，故安全。代价：`Dock` 在手时 `accord`/`release` 够不着 |
| `shut` 立不立 | **立**（用户裁决） | 视图有自己的收尾；`Dock` 是它的天然归口。今天是自检与将来的跨域共享内存在用 |
| `Shut` 的存活闸 | **拿**（用户裁决） | 见 §7；与 `Release` 对齐，`cull.rs` 那行 `let _ =` 就是反证 |
| 一个数叫 `bytes` 还是 `size` | **`size`**（用户裁决） | `bytes` 是**单位**词冒充**量**名；`size` 才是那个量。`region` 的参数则改叫 `reg`（设备树声明的那一段），歧义一并拆掉 |
| 视图类型叫 `View` | **定** | `pole.rs` 与本文档一直叫它"视图"（"视图登记"/"视图清单"）；`Region` / `Span` 与内核既有类型撞名 |
| 驱动持 `Dock` 还是持 `View` | **持 `View`** | 持 `Dock` 就得砍掉 `Uart` 的 `Copy`，去换一个 §6 刚证明守不住的不变量——代价是实的，收益是虚的 |
| `narrow` 上不上 `Dock` | **不上** | 全仓零调用者；它确有映射副作用（同步降 PTE），将来有消费者时再议 |
| 视图携带读写性 | **不带** | 只读映射（DTB）内核已按 pie 权限降了 PTE，写会 fault——**失败是响的**；`View` 不区分 `*const`/`*mut` 这条代价记着 |
| `Dock` 做成设备/寄存器框架 | **否** | `docs/driver.md` §8 已裁：不是设备类框架，两台设备连寄存器宽度都不同 |

## 9 · 判据

`dock` 命令（`programs/src/bin/user/shell.rs` 的 `dock_probe`），四个数各有各的牙，
**三档都跑**：

| 读数 | 断言 | 牙（反向验证：去掉什么它会变） |
|---|---|---|
| `size=4096` | `Dock::open` 返的长度等于 `unseal` 报的 | `Open` 的 a1 填错（填 0、或填设备树 `reg` 的长度）⇒ 不等于 4096 |
| `rw=1` | **首字节与末字节**（`base + size - 1`）各写一个值、读回来相等 | 映射没真建起来 ⇒ 读回不符；长度报大了 ⇒ 末字节落在未映射页，本域当场 fault |
| `remap=1` | 撤图之后再开一次，重开的那段仍可读写 | 撤图与重开之间留下"登记还在、段已经还了"的半截状态 ⇒ 重开后写它就 fault |
| `sealed-shut=1` | **封印之后 `shut` 仍返 `Ok`** | §7 的存活闸加回去（两处任一处）⇒ 当场 `Dead` ⇒ 0 |

这一条同时是 **Pole 页数据面的第一条端到端断言**（`docs/mail.md` §10 第 2 条此前记着
"`PolePie::open`/`shut` 零调用者、门里无断言"，现已不成立）。

## 10 · 已知边界

1. **今天生产路径上没有人撤图**：映射活到域死（`PoleMeta::drop` 兜底），故 `Dock` 在生产
   路径上多半是**瞬时**的——开一次、取视图交出去。`shut` 的消费者是自检与将来的共享内存。
2. **撤图没有用户态观测量**：`shut` 之后写那个 VA 会 fault（本域当场死），而这是它唯一可
   观测的后果。故 `remap` 那一格的牙是"不留半截状态"，**不是**"映射真的没了"的证明。
3. **`UnsealPole` 仍只返 token**：开辟者要地址必须再发一次 `Open`（幂等，返同一个 VA）。
   一次交三件需要三个返回值，而通道只有两个寄存器——这条不齐治不了，列着。
4. **视图不携带读写性**，见 §8 倒数第三行。
5. **`narrow` 仍在 `PolePie`（`AnyPie`）上**，它降 PTE 时会绕过 `Dock` 把视图降权——今天零
   调用者；将来若要上 `Dock`，得先回答"视图降权之后那个 `View` 还算不算数"。
6. **页面里不许放地址**：同一域两个任务映射同一枚 Pole 会得到两个 VA（§6 第三条）。
   共享内存的布局若要在页内放"指针"，那是错的。
