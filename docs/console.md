# console — 控制台协议与服务

> 路径约定：`文件:行` 相对仓库根；`crates/protocol/src/console/` 简写为 `console/`。
> 与它同构的另一张协议见 [dispatch.md](dispatch.md)。

## 1 · 语义定位

控制台是**一个服务**（`prog-console`，S 态），不是每个程序自己读 UART：终端渲染、键盘解码、
行编辑都住在服务侧，客户端只说「我写了什么」和「给我读一行」（`console/mod.rs:3-4`）。

它是**唯一持 UART 的任务**（`programs/src/bin/supervisor/console.rs:3`）。写也由服务落到设备：
客户端的 `Write` 只是把字节推进请求门闩，**那次 `push` 的阻塞就是背压**——与旧 `io::put`
（同步写设备）语义同级（`console/mod.rs:13-14`）。

**「持有设备」已经是权限**（[driver.md](driver.md) 落地后）：UART 是一枚 `Pole` 门闩，
root 用 `Accord` 交给本服务，服务 `Open` 它即映射，此后 load/store 就是设备访问。
`IOCall` 已删——**每个字节都不再穿内核**。迁移前这句话写作：「持有设备是纪律不是权限：
设备走 `IOCall`，是环境给的调用，不占权限表」（旧 `console.rs:68-69`、
`crates/runtime/src/env/io.rs:5-6`，两处都已随设备面删除）。

**不负责**：命令解释与分发（shell）、名字与发现（目录）、命令历史与补全（`Up`/`Down` 无操作、
`Tab` 直插 `\t`）、OSC/DCS、多会话并发等读（**单读者**）。

## 2 · 操作集（4 个，闭环）

| 操作 | 谁调用 | 载体字段 | 失败 |
|---|---|---|---|
| `Open` | `Console::open`（`console/client.rs:86`） | `reply` ＝ 回信孔**在服务侧**的 token（0 非法） | 保留区非零 → `UnexpectedField`/`Reserved`；表满借 `NoSuchClient` |
| `Write` | `write`（`:122`）、`readline` 的 prompt（`:143`） | `client`、`len`、`payload[24]` | id 不认识 → `NoSuchClient`；非 UTF-8 → `Denied`；`len > 24` → `BadLen` |
| `ReadLine` | `readline`（`:150`） | `client`、`len`、`prompt[24]` | 已有等读会话 → `NoSuchClient`（单读者） |
| `Close` | `Console::close`（`:179`，**零调用点**） | `client` | `NoSuchClient` |

`Reply` ＝ `Ok{client}` / `Line{len, payload}` / `Eof` / `Interrupt` / `Denied` / `NoSuchClient`
（`console/wire.rs:112-129`，状态码 `70-75`）。`ProtocolError` ＝ `BadOp`（未知动词或状态码）/
`Reserved` / `UnexpectedField`（该动词下不该有的字段非零）/ `BadLen`（`wire.rs:79-88`）；
客户端一律折成 `denied()` ＝ `-1`（`client.rs:32-34`）。

## 3 · 线格式（64 字节）

请求与回复**共用一张表**，按 `op` 判读，不用哨兵（`wire.rs:5-16`）：

```text
[0]      op        u8
[1..8]   保留
[8..16]  reply     usize LE   Open：回信孔在**服务侧**的 token
[24..32] client    usize LE   会话 id
[32..40] len       u64 LE
[40..64] payload   Write 的字节 / Line 的整行（不含 \n）
```

`MSG_LEN = 64`、`PAYLOAD_LEN = LINE_MAX = 24`（`wire.rs:35-53`）。**会话 id 从 1 起，
`0` 恒表示「无会话」**——这是 id 值域约定，不是字段哨兵（`wire.rs:29-32`、`server.rs:227-232`）。

**第一版多开的那枚「数据孔」的下场**：字段已从 `Request::Open` 里删掉（客户端从不推、
服务从不读，从第一天就是死码），省下每次开会话一次 `Channel::open`（`wire.rs:25-27`）。
**但它的字节区没有收口**：`[16..24]` 既不在字段表里，也不在任何保留检查里——`decode` 只查
`m[1..8]`，另一半是**空区间、恒不触发**（`wire.rs:168-172`）。这 8 字节现在无文档、无检查。

## 4 · 结构

**客户端**：`Console { entry: HolePie, client: usize, reply: Channel }`（`client.rs:50-62`）；
`Channel` ＝「我这枚孔 + 它在对端的号（`at_peer`）」（`crates/runtime/src/core/channel.rs:19-36`）。
`Readline = Line(String) | Eof | Interrupt`（`client.rs:38-45`）。**生命周期是显式的**：
`HolePie` 没有 `Drop`，它是句柄值不是 RAII 守卫，所以资源由 `close` 显式放、两枚孔随进程退出
由内核回收（`client.rs:14-18`）。

**服务侧**：`State { slots: [Option<Slot>; 8], reading: Option<Reading>, pending, term }`
（`server.rs:176-182`）；`Slot` 只存 **token 值**不存句柄（token 是 `Copy`，`server.rs:70-77`）。

**两个线程各只持锁一小段**：

```text
请求线程   pull 请求孔 → Open/Write/Close/ReadLine → （该会话的）回信孔
输入线程   读 UART → VTE 解码 → 改行缓冲 → 重绘 → 回车时把整行**交给共享态**
```

## 5 · 时序：`Open → Write → ReadLine → Close`

1. **Open**：客户端 `Channel::open(server)`（unseal + `accord`）→ 服务 id 由
   `mail::reserve(entry).1`（`owner`）求得（`client.rs:69-76`）→ `push(Open{reply: at_peer})`
   → 服务建槽、回 `Ok{client = i+1}` 且记下 `to_client`（`server.rs:243-255`）→ 请求线程用该
   token `push`（`console.rs:167-174`）→ 客户端 `pull_timeout(1000ms)` 收（`client.rs:104-109`）。
2. **Write**：按 ≤ 24 B 分片，各一次往返 → 服务 `write`：若有会话正在等读，先 `\r\x1b[K`
   擦当前行、打印、再 `redraw`（`\r\x1b[K + prompt + 缓冲 + 光标定位`，`server.rs:286-316,
   346-359`）→ 回 `Ok` 即「已落屏」。
3. **ReadLine**：客户端先把 prompt **同步写一次**（提示符必须先落屏，`client.rs:138-148`）→
   `push(ReadLine{client, len, prompt})` → 服务**只登记** `reading`、不回（`server.rs:322-344`）
   → 客户端在回信孔上**无上界 `pull`**。
   回车那一下：输入线程 `io::try_get` → `Decoder::advance` → `State::on_key(Enter)` 写 `\r\n`、
   `reading.take()`、拼 `Reply::Line`（`server.rs:402-435`）→ `set_pending(client, reply)`
   （`console.rs:88-90`）→ 请求线程 `pull_timeout(IDLE_MS = 20)` 超时 → `take_pending()` →
   推**该会话的回信孔**（`console.rs:148-160`）→ 客户端 `pull` 返回 → `Readline::Line`。
4. **Close**：`push(Close)` → 服务清槽，若正是该会话在等读则一并清 `reading`
   （`server.rs:266-281`）→ 回 `Ok`。

## 6 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 一条请求孔 + 一条回信孔（**没有数据孔**） | 开会话多一次 `Channel::open`；死码复归 | `console/mod.rs:18-20`、`wire.rs:25-27` |
| 交出去的必须是 `at_peer` | 服务 `find` 落空 → `Denied` → 客户端白等满 1s 后静默降级直连设备 | `client.rs:82-88` |
| 只有持 token 的 task 能推 ⇒ **只有请求线程碰孔** | 整行永远递不出去，客户端阻塞在无上界 `pull` 上（现象极具误导性：逐键重绘全对，回车之后什么都没有） | `console.rs:16-26`、`server.rs:32-33` |
| `Write` 同步（`Ok` ＝ 已落屏），且 prompt 先写后读 | 提示符憋在孔里，用户对着空行打字 | `client.rs:6-10,135-137` |
| 只在有会话等读时才碰设备；主线程等待不能无穷 | 抢走 shell 的字节（`spawn` 变 `sawn`）；或整行搁浅在共享槽里 | `console.rs:31-36,74-79` |
| 会话 id 从 1 起，`0` 恒无会话 | `Write`/`ReadLine`/`Close{0}` 一律 `NoSuchClient` | `wire.rs:29-32`、`server.rs:227-232` |
| 三个收尾键（回车 / Ctrl-C / Ctrl-D）一律写 `\r\n` | 下一轮 `\r\x1b[K` 回到本行行首 ⇒ 新提示符原地盖掉旧的 | `server.rs:402-411` |
| 单读者 + 行长上界（512 字符 / 交付 24 字节） | 第二个等读被拒；超 24 B 的行被静默截断；超 512 的插入无操作 | `server.rs:329-335,88-90,429-431` |

## 7 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| `ReadLine` 阻塞还是登记 | **只登记不阻塞** | 就地阻塞 ⇒ 等输入期间别人的 `Write` 排在请求孔里显示不出来；「别的程序能打印」正是控制台存在的理由（`server.rs:5-7,219`） |
| 整行怎么交付 | **经共享态交接**，只有主线程碰孔 | 跨 task 交接句柄两次都被拒；`Lock<State>` 本来就两线程共写，加一格不引新机制（`console.rs:16-29`） |
| 回信孔交哪个号 | **`at_peer`** | `push` 在**推者自己**的表里 `find`（`client.rs:82-88`） |
| 数据孔 | **删** | 客户端从不推、服务从不读（`wire.rs:25-27`） |
| 会话寿命 | **开一次活到进程结束** | 旧版每条输出 2 × `Channel::open` + 2 次往返，实测「输出延迟很高」（`shell.rs:91-97`） |
| 写缓冲 | **按行 / 128 字节合并** | 一条消息只带 24 B，`sq > ` 这种短串也要一次往返（`shell.rs:99-104`） |
| 提示符 | **随 `ReadLine` 带**（并先同步写一次） | 重绘 ＝ `\r\x1b[K + prompt + 缓冲`；服务不知道 prompt 就只剩输入串（`server.rs:319-321`） |
| 非 UTF-8 载荷 | `Denied` | 逐字节写会把转义串打成碎片（`server.rs:309-314`） |
| 表满 | 借 `NoSuchClient` | 协议里没有「稍后重试」这个码（`server.rs:257-262`） |
| 生命周期 | 显式 `close`，**不假装有 `Drop`** | `HolePie` 是句柄值（`client.rs:14-18`） |

## 8 · 已决 / 被否

- **客户端自己解码键盘 / 每程序读 UART** → 否：`Decoder` 必须住在输入线程（`Parser` 不是
  `Send`，进不了共享态），解码状态还要跨读行存活（`server.rs:445-451`）。
- **跨 task 交接句柄** → 否，两个方向都被拒：输入线程直接推回信孔、或自建事件孔 `Accord`
  给主线程（`console.rs:24-26`、`server.rs:33`，注：**别再试第三次**）。
- **一孔两用（数据 + 回信同一枚）** → 否：会让「服务写回信」与「客户端写字节」在同一枚孔里
  对撞。
- **`Render` 的 `clear` / `fg` / `reset`** → 删（零调用点）；shell 侧自留同名门面、经协议发
  转义串（`server.rs:56-60`、`shell.rs:228-241`）。
- **给 `IOCall` 加判据** → 否：`AnyPie` 只有 `Hole`/`Pole`/`Nole`，UART 指不过去——
  「权威不是收紧，是收口」（`kernel/src/work/unit/gate/pie.rs:102-108`）。**已按此收口**：
  不是给 `IOCall` 加判据，而是让 `IOCall` 消失（[driver.md](driver.md) §10 第三步）。

## 9 · 已知边界

> 下面 1、2 两条的归宿已经走完：**所有权搬迁 → 接中断 → 删 `IOCall`**，见 [driver.md](driver.md)。
> 两条都**已办**，就地记为历史：

1. ~~**`IOCall` 的存废**~~ → **已删**：`Put`/`Get` 两个 fid 连同 class 3 一起消失
   （`crates/env/src/fid.rs`），shell 降级 5 处、用户态 panic 打印、root 的 `say` 三处调用方
   全部改道（分别是：删、改 `Reap`、改持设备/走会话）。**结果**：同一段输出在事件流里的
   `IO` 类 envcall **450 → 0**（`driver.md` §7.4）。
2. ~~**降级路径是临时护栏**~~ → **已删**：`fallback_readline` 与 `flush` 里那条 `io::put`
   都没了。**代价如实记**：任务侧现在**没有第二通路**，故 shell 的取舍写成
   `flush` 写丢不致命、`readline` 连不上就 `exit_with(NO_CONSOLE)` 收场
   （`programs/src/bin/user/shell.rs`）。
3. **`Close` 零调用点**：`Console::close` / `client()` 没有任何消费者（会话活到进程结束），
   门里也没有 `Close` 这一步。
4. **线格式 `[16..24]` 无主**（见 §3）。
5. **`Outcome` 的文档枚举不全**：注释说 `to_client: None` 只出现在「`Open` 失败、消息非法」，
   但 `write`/`readline` 的「id 不认识」分支也是 `None`（`server.rs:144-146` vs `:288-293,323-327`）。
6. ~~**`client.rs:67` 注释说 `Owned(entry).owner`**~~ —— **已修（本轮）**（改为 `Reserve(entry).owner`）。
   原记录：代码调的是 `mail::reserve`
   （`PieCall::Reserve`）——`Owned` 是旧名（`:70`）。
7. **`examine.nu` 头注称 stdin EOF 被当 Ctrl-D ⇒ shell 自退停机**，而 `Readline::Eof`
   只是 `continue`（`shell.rs:1200-1203`）；门里的自然停机实际由 `exit` 命令给出。
8. **`NoSuchClient` 双关**（表满 + id 不认识）；`LINE_CAP`(512 字符) 与 `LINE_MAX`(24 字节)
   不等 ⇒ 超长行静默截断；`Up`/`Down` 无操作；主线程 20/200 ms 轮询不是事件驱动。

## 10 · 判据与验证

- **引导第一步就是控制台**：门先 `expect "sq > "`（`examine.nu:419`）——**提示符即两条路
  （写与读）都通的证据**；随后逐步 expect 命令并核 marker，含自然停机。
- **交互输入就是门的传输**：每条命令都走 stdin → 输入线程 → 行编辑 → 共享槽 → 请求线程 →
  回信孔 → shell——**任一步通过即覆盖 §5 整条链路**。
- **逐键回显是诊断量、不是判据**：只在失败时写进 diag（`:351-354,370`）；`.sh` 时代用
  `^sq > ` 计数恒 0，因为提示符前恒有 ANSI 色码。
- **实测逼出来的六处性能修正**（每一处都曾让门判 0/3 或让用户当场看到）：①每条输出开关一次
  会话 → 4 次往返；②24 B 一往返 → 按行攒；③服务每 1 ms 轮询 → 「没事就阻塞」+ 两档等待；
  ④交 `mine` → 白等满 1 s 后静默降级；⑤回执丢在地上 → 三处改走该会话的回信孔；⑥Ctrl-C 不换行。
- **观测坑**：提示符在每次输入出现**两次**（写一次 + 重绘一次），故 `scripts/quick.sh` 不用它
  计数，改数 `clock <秒>` 行。
