# bell — Nole 的 runtime 封装（空载荷门铃）

> 路径约定：`文件:行` 相对仓根。权柄模型在 [pie.md](pie.md)，数据面三件套在
> [mail.md](mail.md)，Hole 的通讯协议在 [port.md](port.md)，中断面在 [driver.md](driver.md) §3.2。
>
> **状态：已实现。** 自检与端到端判据见 §9（`scripts/examine.nu` 的三档全绿）。

## 1 · 语义定位

`Bell` 不是内核对象，是**镜像侧的封装**：把**一枚 Nole** 当门铃用。

```rust
// crates/runtime/src/core/bell.rs
pub struct Bell { pie: NolePie }
```

内核里**只有一种 Nole**——`AnyPie` 不加变体、不新增资源种类。Nole 长出"听者面"（§2），
`Bell` 是它在镜像侧的用法。与 Hole → `Port` 同构：**种类归内核，用法归 runtime**。

| 管 | 不管 |
|---|---|
| 等铃（`wait`）、应铃（`hush`）、响铃（`ring`）、内核那一位的置与清 | 谁该被叫醒（内核只知道"有外部中断"）、线号（PLIC 的活，[driver.md](driver.md) §12 甲）、载荷（**没有载荷**） |

**为什么必须有它**：今天中断门铃是"1 字节的孔，内容恒为零"。[driver.md](driver.md) §3.2.3 的
裁决写的是**空载荷**，实现只能退让成"1 字节的 0"——**因为 `mtu` 最小是 1**。那个字节不是
内容，是一个信号；**让"有载荷的类型"去承载"没有内容的信号"，正是 `Payload::Signal` /
`meta_reserved` / `reserve` 那一串草稿的病根**（§7）。Bell 让"空"第一次**字面成立**。

## 2 · 操作与结构

| 操作 | 谁 | 说的是什么 |
|---|---|---|
| `ring` | 内核（`devices.rs::raise_irq`，**trap 上下文**） | 有外部中断了 |
| `ring` | 持铃的域（自检 / 自己叫自己） | 同上，只是响者不同 |
| `wait(millis)` | 持铃的域 | 等铃（有界 / 无界） |
| `hush` | 持铃的域 | 应铃：清掉"有待取之事" |
| `seal` / `accord` | 既有代数，一字不改 | 拆铃 / 授出 |

由这四个操作倒推，`NoleMeta` 长三个字段（对照 `hole.rs:66-83`）：

```rust
pub struct NoleMeta {
    state: SpinLock<NoleState>,   // 既有：Live / Dead（seal）
    owner: usize,                 // 既有
    id:    NoleId,                // ← wait：等待键的身份（同 HoleId：单调、永不复用）
    life:  Arc<Life>,             // ← wait：站点的寿命（键自然判死）
    ring:  SpinLock<bool>,        // ← ring / hush：有待取之事
}
```

`NoleMeta::new(owner)` **仍然无参**：参数没有增加，增加的是"能等、能响"这一面
（对照 `hole::meta(mtu, owner)` 的大小、`pole::allocate(bytes, owner)` 的字节数——那两个还是参数）。

### 2.1 `ring` 这一位为什么不可省

驱动循环是**排空 → 挂起**。一枚中断完全可能落在它**正排空的那一刻**：此刻没有站点挂在这枚
Nole 上，若"响"只是"唤醒站点"，这一枚就**丢了**，驱动睡到超时——而 20 ms 的有界等待正是
这道门铃当初要消灭的东西。

有这一位：内核发现 `ring` 已置 ⇒ 返 `Busy` ⇒ **本 hart 闸门保持关着**；驱动排空完、`hush`
⇒ 闸门重开 ⇒ PLIC（电平）立刻再陷入 ⇒ 内核再响一次，**这一次驱动在挂起，叫得醒**。

于是 `ring` 位的含义与孔槽里"有东西"同义：**本 hart 闸门关着 = 有待取之事**。`Busy` 不是
错误，是"先把上一件取走"。

## 3 · 内核侧：与孔一族同形

```rust
// kernel/src/work/mail/nole.rs
pub(crate) fn ring(meta: &NoleMeta) -> Result<(), GateError>;   // 已响 → Busy（闸门不重开）
pub(crate) fn hush(meta: &NoleMeta) -> Result<(), GateError>;   // 应铃：清位（**不看 alive**）
pub(crate) fn wait(meta: &NoleMeta, millis: usize) -> Result<bool, GateError>;   // 先探后挂
pub(crate) fn seal(meta: &NoleMeta);                            // += wipe：唤醒听者
```

与 `hole::{try_push, try_take, wait, seal}` 一一对应：`ring` ↔ `try_push`、`wait` ↔ `wait`、
`hush` ↔ `try_take`、`seal` ↔ `seal`。站点表（`Site { pend, head, tail, life }`）原样复用，
只多一个键：

```rust
// kernel/src/work/room/messenger/wait/site.rs（WakeKey + fold）
WakeKey::Nole { id: usize }     // 同 `Hole { hole: usize }`：裸整数，mail → room 单向依赖
```

**`seal` 多一个动作**：唤醒听者（`nole.rs:76-77` 今天写着"不唤醒——没人会等一个没有数据的
东西"；门铃有听者了）。

## 4 · ABI

| 项 | 变 |
|---|---|
| `MailCall::Hush { token }` | **新增**：应铃 |
| `MailCall::Ring { token }` | **新增**：自响（自检要**确定性**的一次响；没有它只能等真中断） |
| `MailCall::Wait { token, dir, millis }` | **复用**：Bell 只认 `dir = Pull`，其它值返 `Denied`（**不静默忽略**——ABI 不留白填的字段） |
| `PieCall::UnsealNole` | 不变（S 态铸币门不变） |
| `PieCall::UnsealBell` | **不立**：门铃是内核给的（`devices.rs`），与设备门闩同一条思路 |
| 配对块 | **零改动**：记录只有 `名字 + token`（`devices.rs:203`）⇒ 门铃不需要任何新的交付路径 |

权限位照旧问"对端能做什么"：

| 动词 | 要的位 |
|---|---|
| `Wait` / `Hush` | `READ`（听与应都在"取"这一侧） |
| `Ring` | `WRITE` |

于是 root 给 plic 授铃时**只授 `READ` 就够**——今天那句 `read_write()` 顺带收紧：驱动不需要
自响，而**内核响它不经过任何门闩**（持源实体，`devices.rs:58`）。

## 5 · runtime 侧

```rust
impl Bell {
    pub fn wait(&self, millis: usize) -> EnvResult<bool>;   // 等铃；**不清**
    pub fn hush(&self) -> EnvResult<()>;                    // 应铃：清
    pub fn ring(&self) -> EnvResult<()>;                    // 自响
}
```

- **`wait` 与 `HolePie::wait(dir, millis)` 同动词、少一个参数**：签名自己说出"门铃只有一条
  方向"，不必在文档里解释一条方向。
- **`wait` 不清**：闸门不变量是"关着 ⇔ 有待取之事"，清必须与"取完"同一刻 ⇒ 显式 `hush`
  （与今天 `wait` + `pull` 两拍同形）。
- plic 的循环因此是三拍：`wait(IRQ_WAIT_MS)` → 排空 → `hush()`。
- `Bell` **不是场所名**：场所（Hole → `Port`、Pole → `Dock`）说的是"一次往返的形状"；
  门铃没有往返，它是一个**用法**（裸 `NolePie` 的用法是建域权，见 §6）。

## 6 · 建域权：门铃成了 Nole，判据为什么不改

三条既成事实：门一 = 调用方表里有一枚**活着的 Nole**（`work/unit/gate/right.rs:27`）；
门二 = 调用方是 **S 态**域（`runtime/switcher/envcall.rs:426-434`）；而 root 把 irq 门铃
**交给 plic 子域**（`programs/src/bin/supervisor/root/main.rs:419`）。于是 plic 表里多了一枚
活着的 Nole ⇒ 门一过；plic 是 S 态 ⇒ 门二过 ⇒ **驱动能建域**。

净增量，老实列：

| 它拿到的 | 是不是新的 |
|---|---|
| 一件**机制**：内核代办的 `Build`（分配空间/页表/任务、进内核账、可 `accord` 门闩、按血缘级联回收） | 是 |
| 一类**权**：新域能拿到的门闩只能是它自己有的那些（PLIC 寄存器、dtb、铃） | **不是**——`Refer` 写属主只有 root 能写（[driver.md](driver.md) §12 甲） |
| 一种**能力**：S 态域本来就是"沙箱外"的身份（`switcher/context.rs:154` 以 `SPP = Supervisor` 回去），能自开页表、自起 U 态活 | **不是** |
| **内核资源**：空间、页表、任务条目 | 是——**DoS 面，不是越权面** |
| 收场：`Doom` 按血缘，root 是全体域的祖先 | **不是**——plic 一死，它生的域一起被级联收掉 |

**结论：两个门本来就问两件事**——门一问"你有没有存在权"，门二问"这份存在**够不够**伸到
沙箱外"。**资源类型答第一问，门二答第二问**；让资源类型也去答第二问（另立 `AnyPie::Bell`）
是同一件事两个判据。

## 7 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| `AnyPie::Bell` 第四种资源 | **否**（用户裁决） | Bell 是 Nole 的 **runtime 封装**，内核一种 Nole。够不够建域由**门二**回答，不由种类回答——加第四种是把门二的活搬到类型上 |
| 拿 `owner != 0` 当建域权判据 | **否** | 用"资源的来历"代理"是不是建域权"，绕；门二已回答 |
| 门铃继续用 1 字节的孔 | **否** | §3.2.3 的"空载荷"只能退让成"1 字节的 0"（`mtu` 最小 1）；那个字节是信号不是内容 |
| `Payload::{Signal, Borrowed(&'static [u8])}` | **否** | `'static` 只为躲开 `HoleMeta` 的生命周期污染；信号该由"没有数据面"的类型承担 |
| `reserve` / `meta_reserved` / `Push::{Own, Borrow}` / pull 侧回填 | **否** | 全是"让有载荷的类型承载无内容信号"逼出来的补丁；Bell 一出，一串全销 |
| `ring` 位放内核设备对象（而非 `NoleMeta`） | **否** | 等待路径是 pie → meta → site；位不在 meta 就得在 meta 里放指针回指设备对象——绕一圈仍是 meta 的字段 |
| Bell 另立 meta（"没数据的孔"） | **否** | 回到"内核靠标记认它"那条路（`nole.rs:24-26` 已否） |
| 门铃留给 root、plic 等别的东西 | **否** | 等的人必须持闩；多一跳，且 root 的正事是"等 shell 死" |
| `listen` | **改**：定为 `wait` | 与 `HolePie::wait` 同族同动词，少一个参数就说清了"只有一条方向" |
| 全局上限 `HOLE_MSG_MAX` | **否** | `UnsealHole` 是**预分配**（空孔白占内存）⇒ 真正的界是分配器的 `try_reserve` → `OoM`（见 [port.md](port.md) §5） |

## 8 · 已知边界

1. **建域权与门铃共用一个种类**：门一不看"是哪一枚"，只看"有没有一枚活着的"。plic 若不降到
   U 态，它就在自己不需要的那一级上——`kernel/build.rs:23-24` 的待办因此从"最小特权偏好"
   变成"**降了才自动挡住**"。
2. **`hush` 不看 `alive`**：拆铃（`seal`）已把听者唤醒，但已置起的 `ring` 位仍要由 `hush`
   清——不清也不会**关死**闸门（闸门是**零状态**的：`trap.rs:202` 每个 timer tick 无条件
   `set_sext`、`fetch.rs:150` 取活前也重开），只是让本 hart 退化成 ≤100 ms 的轮询。
   `hush` 里那次 `set_sext` 是让重开**立即**发生，不是唯一路径。内核那枚铃 `IRQ` 是
   `OnceLock` **永久持源**，死不了（`devices.rs:52-66` 的既有例外）。
3. **一响一应**：多 hart 同时取到 SEI 时，第二枚起返 `Busy`，**多个 ring 合成一位**——"门铃里
   还有几枚"不再是可数的事实。故 plic 的排空判据是"**PLIC 里还领得出线**"（`deliver` 领到 0
   就停，`plic.rs` 的 `claim` 恒有这条判据），不数门铃。这比旧的"推了几枚"更对：**线号的
   权威在 PLIC，不在内核的计数**。
4. **`Wait` 的 `dir` 对 Bell 是死的**：复用 `Wait` 的代价，靠"只认 `Pull`"兜住；不另立 variant。
5. **`Ring` 是给自检与"自己叫自己"的**：响者由持铃者决定；它响不动别人的站点（站点挂在这枚
   meta 的 `id` 上）。
6. **三个旧的"定义式"注释已照改**（账在此，落地时一并改，不再欠）：
   `nole.rs:28-29`「只承载**无状态**的存在权」→ **无载荷**的存在权，唯一允许的状态是一位
   门铃（没有"多少/哪个"，故不是数据面）；`nole.rs:48-50`「**没有 id**：没人会等一个没有
   数据的东西」→ 门铃有听者，`id`/`life` **因听者而存在**；`nole.rs:76-77`「不唤醒」→
   见 §3。`work/unit/gate/right.rs:13`「Nole 没有别的用途可被误用」的**理由句**换成"两种用途
   都是存在权，够不够建域由门二回答"（判据一字不改）。
7. **`docs/driver.md` §3.2.3 / §8.1.10 已改判**：门铃从"1 字节的孔、载荷恒 `[0]`"改成
   "一枚 Nole，空载荷字面成立"；§3.2.2 那格的闸门政策补上"用户应铃时也立即重开"。
8. **判据未立**的只剩一条：本地没有别的消费者，端到端只覆盖 `irq` 那一道门铃。

## 9 · 判据（已跑）

- **自检（新机制，一轮全测）**：`prog-root` 在**铸建域权之前**自铸一枚 Nole（`UnsealNole`
  走一遍），八个断言各答一个问题：`wait(0)` 未响返 `false` · `ring` 得动 · 再 `ring` 返
  `Busy` · 响着 `wait(0)` 返 `true`（**不清**）· `hush` 得动 · 应完 `wait(0)` 又返 `false` ·
  未响时 `hush` 返 `Busy` · `Wait { dir: Push }` 返 `Denied`。打完一行
  `bell: quiet=1 rung=1 twice=1 pending=1 hush=1 clear=1 empty=1 dir=1`，随后 `release`（铃随
  最后一份门闩消亡，走 `Drop` → seal + wipe）。**门断言这行字**
  （`scripts/examine.nu` 的 `MARKERS` 首条）。
  **它的牙**：把 `ring` 的"置位"去掉、只留唤醒站点，`pending` 那一格变 0——响在没人听的
  那一刻就丢。**实测过一次反向**：适配层把 `Handoff::Resume(false)` 丢掉、Bell 那一支恒答
  `true` ⇒ `quiet`/`clear` 两格当场变 0，门 0/4 拦下（三档都拦）。
- **端到端**：`irq` 门铃换成 Nole 之后，既有 marker 全部照旧 —— `plic: line 10 delivered`
  与 `uart: irq ok` **就是"内核响 → 驱动排空 → 投递 → 客户端取到线号"整链走通的证据**
  （不新造判据；全线只在真拿到东西时说话）。
- **不许退**：`spawn` / `dir` / `req echo` / `hole` / `badslot` / `stray` / `cascade` / `lend`
  / `kill` / `line` / 服务重启十一条既有自检全绿。
- **门**：默认档 + 框架档（默认跑）与 harden 档 `scripts/examine.nu` 绿；`cargo fmt --check`
  干净。
