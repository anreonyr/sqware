# mail — 数据面（Hole · Pole · Nole）

> 路径约定：`文件:行` 相对 `kernel/src/`。权柄模型在 [pie.md](pie.md)：**数据面不感知 rights**，
> 判权在 envcall 入口。

## 1 · 语义定位

三者按「**有没有数据面**」分类（`work/mail/mod.rs:17-18`）：Hole 有槽、Pole 有页、
Nole **什么都没有**。内核在 mail 管两件事：**资源实体**（`HoleMeta`/`PoleMeta`/`NoleMeta`）
与**数据面动作**（push / pull / map / unmap）；还有一层的用户空间拷贝（`copy_in`/`copy_out`）。

| 管 | 不管 |
|---|---|
| 槽、页、身份、存活、阻塞语义的**载体** | 权限判定（在 envcall）、调度与等待队列（在 messenger）、协议语义（在 `crates/protocol`） |

**没有全局资源表**：资源寿命 ＝ 能力寿命（`mod.rs:11-12`）。`gate → mail` 单向依赖。

## 2 · 结构

| 文件 | 职责 |
|---|---|
| `work/mail/mod.rs` | 共享层：`whole()` 整段区间校验 + `copy_in`/`copy_out` + `HOLE_MTU_MAX = 4096`（`mod.rs:34`） |
| `work/mail/hole.rs` | 单槽管道：`HoleId` / `Slot` / `HoleMeta`、`try_push` / `try_pull` / `ready` / `key` / `seal` |
| `work/mail/pole.rs` | 页地基：页块 + 视图登记（键 ＝ per-pie token）、`open` / `shut` / `narrow` / `seal` |
| `work/mail/nole.rs` | 无载荷载体：`state` + `owner`、`seal` |
| `runtime/switcher/envcall/mail.rs` | class 5 数据轴入口：判权 → 长度校验 → **锁外**暂存 → `try_*` → 盖章 |
| `runtime/switcher/envcall/pie.rs` | class 7 权柄轴：`Unseal*` / `Open` / `Shut` / `Seal` / `Narrow` / `Release` 的编排 |
| `crates/runtime/src/env/mail.rs` | 用户侧裸函数层 + `HolePie`/`PolePie`/`NolePie` + `AnyPie` + `pull_timeout` |

| | Hole | Pole | Nole |
|---|---|---|---|
| 载荷 | 单槽 `Vec<u8>`（capacity = mtu） | 页块 + 视图表 | **无** |
| 自有 id | `HoleId`（等待键身份，单调不复用） | 无；映射键 ＝ per-pie `token` | **无 id** |
| 存活单元 | `Arc<Life>`（两个方向的键都指它） | 无 | 无 |
| 开辟者 / 状态 | `owner`；`Live`/`Dead` | `owner`；`Live`/`Dead` | `owner`；`Live`/`Dead` |

**代码里没有 `ResourceId` 类型**：身份由 `HoleId`（孔的等待键身份）与 per-pie `token`
（门闩句柄 / Pole 映射键）分担。

## 3 · Hole

- **单槽**：`len()` 既是「消息在不在槽」也是实际字节数——Push `set_len`、Pull `clear`，
  零额外分配（`hole.rs:75-76,189-199,221-225`）。
- **mtu**：`UnsealHole` 时定，取值 `1..= HOLE_MTU_MAX`；**唯一校验点** `hole.rs:283-286`。
  用户侧另有一份同值常量（`crates/runtime/src/env/mail.rs:24`），靠内核兜底。
- **方向**：Push 需 `W`、Pull 需 `R`（`envcall/mail.rs:175-178`）；`ready(Pull)` ＝ 槽非空、
  `ready(Push)` ＝ 槽空（`hole.rs:119-130`）。
- **盖章**：`from` ＝ 推者 task id，Push 时内核写、**与消息同锁同写**（`envcall/mail.rs:61-62,86`）；
  Pull 一并交回 `(len, from)`（`hole.rs:224-228`）。
- **封印**：置死 + `wipe` 两个方向的键，**不回收内存**（`hole.rs:269-273`）；写/读完槽各唤醒
  对侧（`:200,227`）。
- **`Drop`**：置死 + wipe 两个方向全部等待者（`hole.rs:139-145`）——调用方的义务是**在锁外**
  drop 门闩。

## 4 · Pole

- `unseal` 取页对齐帧并清零（`pole.rs:52-68`）。
- **`Open`/`Shut` 就是 map/unmap**：`open_into` 做 `allocate(Seg::User, bytes)` +
  `borrow(va, pa, bytes, flags)`（`:106-117`），登记 `(token, Weak<Space>, Span)`。
  **键取 per-pie token 而非 per-space**：同一空间里多任务各有一条独立 PTE，`narrow` 只动
  自己那条——`cap ⊆ 页表` 不被共享 PTE 击穿（`pole.rs:37-39`）。`shut_from` 幂等 +
  `space.release(span)`；空间已死则映射随之消失（`:149-165`）。`Drop` 逐视图 release + 还帧。
- **所有权闸**：`SpaceInner::protect` 是全树唯一的 flags 写点，借入页的新 flags 必须是
  **当前 PTE flags 的子集**，否则 `WidenDenied`（`space/core.rs:329-346,375-398`）——
  「加宽无路可走」正是 `narrow` 的 `cap ⊆ 页表` 契约的地基；`open` 在 map 之后还会强制
  `protect` 一次确认（`pole.rs:200-202`）。
- Pole 的 subset **必须含 `READ`**（RISC-V 无 `R=0` 的合法数据叶，`envcall.rs:55-65`）。

## 5 · Nole

名字 ＝ no + -ole：与 Hole/Pole 同族而说「没有」（`nole.rs:5-12`）。它是存在权的唯一合法载体：

- **因为空，所以不可能被当成资源来使唤**（`:26`）——没有槽就没有背压，没有页就没有映射。
- **不做成 `Permission` 的一位**：位与资源同轴，`READ|WRITE|BUILD` 一旦能出现，
  「这是不是那枚」就答不出来（`:21-23`）；它也不是「没数据的 Hole」（`:25-26`）。
- **连 id 都没有**：id 是等待键的身份，而没人会等一个没有数据的东西（`:48-50`）——
  故 `seal` 没有 `wipe` 那一步（`:76-77`）。
- **消费者 ＝ 建域权**：按 token 在**调用方自己表里**找一枚活着的 Nole（`gate/right.rs:22-33`）；
  铸币权（`UnsealNole`）收在 S 态（`envcall/pie.rs:129-135`）。

## 6 · 拷贝契约

共用前置 `whole()`：区间**每一个**段都在、权限含 `need`、段长之和恰为 `len`（中途未映射 ⇒
`Segments` 提前终止 ⇒ 和不等于，`mod.rs:49-58`）。契约是「**要么全读，要么 `dst` 一个字节
都不动**」/「**要么全写，要么用户缓冲一个字节都不动**」——先整段验完，再动第一个字节。

旧版是边写边判权限，失败路径上前面几页已经写脏（`mod.rs:90-92` 自述）。代价是区间多走
一遍 `Segments`；两遍之间映射可能变（他核 unmap），自认是**既有**窗口（`:44-48`）。

**锁序**：handler 先把数据拷进栈/堆暂存，再进 `try_push`/`try_pull`——槽(`L3`) 与
`Space.segments`(`L2`) 不得嵌套（`hole.rs:17-19`、`envcall/mail.rs:82-89`）。

## 7 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 资源寿命 ＝ 能力寿命（无全局资源表） | 槽/帧/页泄漏或提前回收 | `gate/pie.rs:60-71`、`hole.rs:133`、`pole.rs:168` |
| 键身份 ＝ `HoleId`（单调不复用），不是堆地址 | 死孔的陈旧信标被同地址新孔继承 ⇒ 无关 `wait` 立刻「已唤醒」 | `hole.rs:35-45,156-169` |
| 就绪三者同源（`ready` / 挂起条件 / 唤醒点读同一份 `slot.len()`） | 就绪却没人唤醒，或唤醒后仍不满足 | `hole.rs:119-130` |
| 锁序：`slot`(L3) 不与 `Space`(L2) 嵌套；门闩在锁外 drop | 4→2 反向嵌套 / 3→3 自锁 | `hole.rs:17-19`、`envcall/mail.rs:82-85`、`cull.rs:9-11` |
| 要么全写、要么一个字节都不动 | 失败路径留半截数据（假契约） | `mod.rs:49-58,89-96` |
| `cap ⊆ 页表`；借入页只能收紧 | 按 VA 单方面扩大他人资源权限 | `pole.rs:126-147`、`space/core.rs:375-398` |
| 封印只置死 + 唤醒、不回收不摘表项 | 封印后表项永远摘不掉 | `pole.rs:222-227`、`release.rs:22-34` |
| 存在权铸币权收在 S 态 | 任何 U 域自铸 Nole 即自授建域权 | `envcall/pie.rs:129-135` |

## 8 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| Hole 单槽 vs 队列 | 单槽 | 长度是**参数**不是约定：Push 声明 len、Pull 声明 max（`hole.rs:3-4`） |
| 方向如何进键 | 枚举字段，不位打包 | 压低位曾逼出 `\|1` 拆句去躲 size 优化折叠 mask（`hole.rs:160-163`） |
| 键取 id vs 地址 | `HoleId` | 站点从不回收，地址会被分配器复用（`:35-38`） |
| Push 的发送者谁说了算 | 内核盖章 | syscall 上下文不可伪造（`:56-57`） |
| `Wait` 未就绪返什么 | `false`，**绝不返 `-3 Busy`** | 未就绪的答案就是 `false` |
| Pole 映射键 token vs space | per-pie token | 共享 PTE 会击穿 `cap ⊆ 页表`（`pole.rs:37-39`） |
| 借入页能加宽吗 | 不能，只能收紧 | 叶 PTE 是权限的权威（`space/core.rs:329-345`） |
| Nole 做位 vs 做类型 | 类型 | 位与资源同轴，会让任何资源顺带携带它（`nole.rs:21-23`） |
| 封印是否回收内存 | 不回收 | 寿命由引用计数，`Drop` 接管（`hole.rs:263-266`） |
| `pull_timeout` 的 `wait == false` 算超时吗 | 不算 | 可能是信标被消费或无关唤醒 ⇒ 按 deadline 循环（`env/mail.rs:336-341`） |
| Hole 的「开闩」是什么 | 就是 Push/Pull | 只有 Pole 有 `Open`/`Shut`（`fid.rs:274-275`） |

## 9 · 时序：`Push` → 对端 `Pull` 醒来

1. `HolePie::push`（`env/mail.rs:294-304`）→ 裸层封 `MailCall::Push`（`:65-76`）→ `ebreak`。
2. class 5 解码（`fid.rs:399`）→ 数据轴 dispatch（`envcall/mail.rs:41-43`）。
3. 判权与存活：按 token 在 `task.pies` 里找 `AnyPie::Hole`，需 `Need::Write`（`:63-75`）。
4. 长度校验 `len ∈ [1, meta.mtu]`（`:79`）。
5. **锁外**拷进暂存（`copy_in`，`:84-89`）。
6. `try_push`：槽非空 ⇒ `Busy`；否则拷进槽、`set_len`、盖 `from = me`（`hole.rs:189-199`）。
7. 放锁后 `wake(WakeKey::Hole{hole, dir:Pull}, &meta.life())`（`:200`）。
8. 站点表：有等待者 ⇒ 摘队首 + `void(票根)` + `rise`（`wait/mod.rs:287-310`）；无 ⇒ **置信标**
   `pend = true`（`site.rs:107-113`）。
9. 对端 `pull` 的 Busy 循环（`env/mail.rs:307-317`）→ `hole::wait` **先探**就绪位，就绪即不挂起
   （`hole.rs:248-250`——先探不可省：对侧可能已写入并正等我们取）；否则走 `block` 的
   ①信标先探 ②离核 ③发票 + 票根 + `tock` ④入队（锁内判键死活、再查信标）。
10. 醒来：`rise` 入就绪复跑 → 重试 `pull` → `try_pull` 取消息、清槽、`wake(Push)`，返
    `(len, from)`（`envcall/mail.rs:150-157`、`hole.rs:212-228`）。

唤醒键恒为 `(HoleId, HoleDir)`；键的寿命由 `HoleMeta.life` 承担——孔死键即判死、站点当场删
（`hole.rs:101-107`、`site.rs:195-227`）。

## 10 · 已知边界

1. **Pole 封印后借入映射撤不掉**：`pole::shut` 自带 `alive` 闸（`pole.rs:206-212`），而
   `Release` 的撤映射走 `cull` 且 `let _ = pole::shut(...)` 吞掉 `Dead`（`cull.rs:81-84`）。
   Meta 仍活着（另有副本）时映射会留到 Space 或 Meta 死——与 `fid.rs:350`「Pole 同步
   unmap」的承诺不一致（若该 pie 是最后一份强引用，`PoleMeta::drop` 会补上，`:168-184`）。
2. **Pole 页数据面无端到端自检**：`PolePie::open`/`shut` 在 `programs/`、`crates/` 里零调用者；
   唯一消费者是 `reclaim` 的 `unseal(4096) + release × 40000`（`shell.rs:573-582`）。
   `narrow` 同步降页表、所有权闸都只有内核代码与注释，门里无断言。
3. ~~**`hole.rs:281` 注释与代码不符**~~ —— **已修（本轮）**（改为「**唯一校验点**」）。原记录：
   称 mtu「envcall 入口已校验，此处 defend」，但入口
   `envcall/pie.rs:109` 把 mtu 直交 `meta()`，没有第二次校验。
4. **用户侧 `HOLE_MTU_MAX` 是重复常量**（`env/mail.rs:24` ↔ `mail/mod.rs:34`），无编译期绑定。
5. **`NoleMeta` 未 re-export**：`mod.rs:3,28-29` 只把 Hole/Pole 记为资源实体
   （`gate/pie.rs:107` 走全路径），与同文件 `:9` 的「三面并列」不对称。
6. **Pull 缓冲下界是调用方义务**：`dst.len() < 消息长度` ⇒ `Denied` 且**不动槽**
   （`hole.rs:217-219`）；没有长度查询原语，收方须自备 ≥ mtu 的缓冲。
7. **`pull_timeout` 超时后该孔不再「干净」**：迟到的回复仍可能落槽，调用方应弃用会话
   （`env/mail.rs:335-336`）。
8. **门的字段数比内核打印少一个**：`examine.nu:24-27` 与旧 trace 记 5 个数，当前
   `audit.rs:482` 打 6 个（多了 `dead`）；门正则只锚 `[audit] sites ` 前缀，故仍成立。

## 11 · 判据与验证

- **门**：`hole` 步断言 `hole got "hi from shell`（`examine.nu:135,163`）；同一条命令内
  `seal_wake_probe` 以 seal 为界各等一次（前等满期限、后当场拿结论），audit 档断言
  `hole: wait-seal sealed=1 wake=seal`（`shell.rs:901-935`、`examine.nu:185,193`）——
  **封印不唤醒等待者就只会打出 `timeout`**。
- **附带断言**：`[audit] sites … by kind: space N hole N …` 要求全部任务退出后无孤儿、无活站点、
  无残留等待者；其中 `hole N` 就是孔等待键随资源消亡的直接观测量（`audit.rs:468-482`）。
- **`spoof`** 覆盖「Push 盖章不可伪造」（`shell.rs:687-799`）；**`reclaim`** 覆盖「随最后一份
  能力回收」与「封印只归开辟者」（`shell.rs:562-660`）。
