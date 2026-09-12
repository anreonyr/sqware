# churn 下 OOM 的诊断记录

> 本文只写**有判据支撑**的部分。被推翻的假设也列出（避免下次重走）。

## 0. 结论摘要

- **核心诉求已达成**：OOM 以 `EnvError(-4)` 返回用户态，**不再 panic 内核**。
  `churn 500 2 2`（原始失败用例，原版死在第 185 轮）现于 **~1.5 s 跑完并干净停机**。
- **`freelist` ↔ `pagemeta` 的长期背离已定位并修复**（§10，第 10 轮）：两个根因都是
  一处写点的缺陷 —— ① `split_block` 写入**粗表项**（块首声明了不属于它的跨度）；
  ② `push_link`/`pull_link` 里 `x.read().prev = …` 是对**临时副本**赋值（静默失效的
  指针写，双向链表 `prev` 从未维护）。修后所有背离读数**恰好归零**，并升格为
  框架档用例的**绝对零**判据。
- 仍未修：`DuplicateMark`（§3.2，audit 档）、以及一处**偶发**的
  `[audit] leak: task 1`（§3.3，仅 harden/audit 档的关机审计，正在查）。

## 1. 判据工具（已落地）

| 工具 | 位置 | 说明 |
|---|---|---|
| 池水位一行 | `work/room/messenger/reap.rs` | 每 200 笔回收打 `walk/meta/held/step/cycles` |
| 闭环校准 | `shell.rs` 的 `calib [rounds]` | `alloc+free` 配平循环，净变化必须为 0 |
| 迭代驱动 | `scripts/fast.sh` | ~3.5 s 一轮（见 §4 的两条驱动纪律） |

**`walk` 与 `meta` 是两条独立读法**：
- `walk` = freelist 走链累加 `blocks << order`；
- `meta` = 只按 `pagemeta` 求和（`free=true` 的块）。
- `step` = 按块首**步进**扫 `pagemeta`（物理覆盖，已证恒定）。
- `meta + held ≈ step` 恒成立 ⇒ `pagemeta` 自洽。

## 2. 已修复（有实测判据）

| 缺陷 | 判据 |
|---|---|
| `SpaceInner::unmap` 在释放路径上重建整张映射表（`Map`=112 B，1024 条 = 114688 B → `alloc.rs:673` panic） | 改为原地 `retain_mut` 压实；`churn 500 2 2` 从死 185 轮 → 跑完 |
| 每个 spawn 的任务漏一页 TLS | `tls::free()` 接回；`held` 静息漂移显著下降 |
| `try_reserve_roster(id)` 用**单调全局 id** 当槽位（`HashMap` 容量与键无关）→ 预留量随时间线性膨胀 | 改 `try_reserve(1)`；`-4` 从 2220 轮推迟 |
| `roster()` 的 `collect()` 不可失败（`Spawn` 必经） | `try_reserve` + 空快照降级 |
| 退场钩子（`Hook = fn(usize)`，无错误通道）上的不可失败分配 | `cull::doom` / `snap::heirs` / `cull` 的 BFS 表全部可失败 |
| `pull_link` 拿坏 `Link` 直接索引 `pagemeta` → 越界 panic | 越界返回 `None` + 计数 |
| `PortalAllocator` 拿 `spare`（panic/日志打印专用预算）当通用兜底 | **已撤**：`spare` 只服务 hart trace 与 panic 打印 |
| `chain_len` 无步数上限（链成环即持锁死循环） | 加预算 + `cycles` 计数 |

## 3. 未修的深层缺陷

### 3.1 `walk` 与 `meta` 长期背离 —— **已结案（见 §10）**

> 本节以下内容是**定位过程**的记述，其中若干中间结论已被 §10 推翻（尤其"`pagemeta`
> 是一张容许重叠、不是函数的覆盖图"那一条：重叠是**一处写点**的产物，不是这张表的
> 性质）。保留的用处是看"缺自洽约束的探针如何持续产出可信的错答案"。

实测（64M，`churn 2 2`）：

```
wm reap=3400 walk=321 meta=11956 held=1842 step=13173 cycles=0
```

约 1.2 万帧被 `pagemeta` 标为空闲、而走链看不见。**harden 档（护栏全开：
`check_bounds` / `check_frame_free` / `check_not_in_chain`）跑到失败点零报警**
⇒ 链的局部操作合法，问题在"**哪些块压根没进链**"。未定位。

#### 已推翻的早期判断（勿重走）

| 早期结论 | 推翻依据 |
|---|---|
| "`pagedrain` 每轮净漏 90~570 帧，单调累积" | 那是**瞬时** `walk` 快照；释放期间内核自己也在从同一池取帧（`Vec` 扩容、页表、簿记），采样时刻并非静息。改用守恒量 `live`（累计分配 − 累计释放）后**每轮精确回到基线**（954→955→955）⇒ **没有帧经 `deallocate` 漏掉** |
| "`census` 显示 204 条空闲表项 vs 54 个链块" | `pagemeta[X] = Some(free=true)` 全仓只有 `push_link` 一处写（其自审零报告）⇒ 是我的**计数器 bug**（同位置覆盖时漏扣） |
| "`merge_census`：`meta=0` ⇒ 主因是 `in_freelist`" | 同一份数据自相矛盾：`MR_META`=0 而 `REJ_META[0]`=151（同一分支相邻两行各自自增）；且 `REJ_CHAIN` 各 power 之和 21 vs 总数 254 |
| "`frnet` 能证块进链又消失" | `frnet=83126` 而池子仅 32019 帧 —— 该量**不守恒**（一次 `pull_link` 配多次 `push_link`） |
| "幽灵表项覆盖跨度" | 加跨度清理后 `walk` 仍逐轮掉；且它在大 order 上遍历 `2^power` 槽，把运行拖慢约 50 倍 |

#### 仍然站得住的事实

- `step`（按块首步进扫 `pagemeta`）**恒定**，`pagemeta` 自洽
- harden 档护栏全开、跑到 `spawn failed` **零报警**
- `pagedrain` 的各种释放顺序（升序 / 降序 / 只放偶数）**都漏**、量级相同
  ⇒ 与释放顺序无关、与 `split_block`/`merge_block` 的链操作无关
  （两者自审均零报告）
- 帧的 alloc/free 调用点**布局对称**（逐点核对过）

### 3.2 `DuplicateMark`（`--features audit` 下）

```
DuplicateMark at 0x600000000034
  old(size=4096 kind=UserHeap site=0x80240ee0 marks=1)
  vs new(size=4096 kind=UserHeap site=0x802160c0)
```

`key = (asid << 44) | (va >> 12)`，故 `0x600000000034` = `asid 6` / 页索引 `0x34`。
同一 `(asid, 页索引)` 被 mark 两次而中间没有 unmark ⇒ **同一用户页被交付两次**
⇒ 两个所有者互相覆写，是下列症状的共同上游：

- `snap::find` 读到坏 `Weak`（`stval=0x1078`）
- `IllegalInstruction at sepc=<Vec<Weak<Task>>::from_iter>`
- freelist 节点 `addr` 变成 wait key（索引 `0x07011C7D01BB83C6`）
- 16 / 24 / 160 字节分配失败

**已排除的假设**：
- ~~残留 `free=true` 标记~~ —— 给 `push_link` 加"清跨度"后 `stale_cleared=0`、零报告；且该循环在大 order 上遍历 `2^power` 槽，把运行拖慢约 50 倍，已撤。
- ~~`Munmap` 漏注销~~ —— 该缺口**确实存在**（`ShareWindow::munmap` 校验的是 `holds(Seg::User, ..)`，与堆同段，却不走 `HeapWindow::deallocate`），但加了 `retire_range` 后**全部运行零次触发**、`DuplicateMark` 依旧，故已连同实现撤回。

### 3.3 `[audit] leak: task 1`（偶发，仅 audit/harden 档的关机审计）

**现状：出现 1 次，未复现。** harden 档整轮跑到关机审计时报 `Kind::Task` 未归零：

```
[audit] delta frame: total +0 avail +2 occ +190; block: occ +12; spare: total +0 occ +82048
[audit] sites 3 live 0 tomb 3 orphan 0 dead 0 waiters 0
[audit] leak: task 1
[audit]   task @ 0x841c2c00 size 152 site 0x8023a0e4
[audit]     <- Task block: strong 1, ident @ 0x0 (record not in this batch)
[integrity] AuditDivergence at 0x0: 1 object kinds leaked at shutdown
```

已确立的事实：

- **不是 seed 相关**：同 seed（26429988）+ 同 15 条命令，按提示符同步重跑不复现。
  两个 ELF（改前 / 改后）各跑 6 个 seed 共 12 轮，**全部 0 泄漏、全部干净停机**；
  故它是**时序相关**的偶发项。
- 泄漏的对象是 `ArcInner<Task>`（size 152 = 16 + 136），**不是** `TaskIdent`
  （48）也不是 `Life`（零尺寸）。
- `[audit] delta` / `[audit] sites` 两份读数与历史通过的 harden 轮**逐字一致**，
  差别只有这一条。

**关键线索（本轮修正的一处读数错误）**：`ArcInner<T> = { strong, weak, data }`，
strong 在 `+0`、weak 在 `+8`，而取证代码只读了 `+8` 却标成 `strong`。故
"strong 1" 实为 **weak 1**。若 `strong == 0 && weak == 1`，含义是：
载荷已析构、块却因**一枚存活的 `Weak<Task>`** 而未归还 —— 泄漏的是 152 B 的
`ArcInner` 外壳，不是活任务（站点墓碑 / 注册表里的一枚 `Weak` 即可造成，而
`tomb 3` 说明关机上确实留着墓碑）。已修取证读数（两个都读、都打），
下次复现即可一句话定案。

**待查方向**：谁持有那枚 `Weak<Task>`（`wait/site.rs` 的站点值？
`team` 的成员表？`heir`？），以及它为何在墓碑后仍未被回收。

### 3.4 关机偶发**挂住**（harden 档约 1/10 轮；**与本轮改动无关**）

**症状**：`exit` 之后内核开始关机，打印若干 `[audit] space asid N retired M user-heap
records`，然后**永不 `system halted`**；四个 hart 全部停在空闲 `wfi`
（`scheduler::core::fetch::wait`，见下），外接 timeout 180 s 才收场。

成功一轮与挂住一轮的日志对照（**缺的正是 asid 3**）：

```
成功: asid 1 → 2 → 3 → 4 → 6 → 5   task: all tasks exited, system halted
挂住: asid 1 → 2 →    4 → 5 → 6    ←—— 到此为止
```

**为什么判定与本轮改动无关**：改前那份 ELF（`git checkout 9fb61c4 -- kernel/src` 构建，
`strings | grep stale-rm` = 0 与改后区分）**同样挂住**，且日志签名逐字相同（同样缺
asid 3）。且它**不由 seed 决定**：同一 ELF + 同一 seed 重跑可以一次挂一次不挂
（`mine/101` 挂、`before/101` 通；`before/202` 挂、`mine/202` 通）⇒ 是**时序相关的竞态**。

**已排除的假象**：本轮用 gdb（QEMU `-gdb tcp::1234`）抓到过一例"四核全停在
`fetch::wait` 的 `wfi`"，但那份日志里 `exit` 被喂成了 `xit`
（`unknown: xit`）—— **是我的驱动吃了首字符**，属喂入失误而非内核挂住。同一脚本补上
`T_GAP=2` 的让出后连跑 8 轮全部正常停机。故"四核空闲在 `wfi`"这个现场**尚未**在真挂住
的轮次里取到。

**下一步（窄且明确）**：挂住时没有任何 hart 在跑，堆栈取不到"卡在哪个对象"，故要
**在关机路径上装进度信标**（只在 audit 档）：每个空间的 drop 进入/离开、每个任务
exit 的进入/离开、停机屏障已到达的 hart 数，各打一行带序号。挂住那轮的**最后一行**
就是卡点。若挂点在 `Ledger::retire` 的 `retain` 里（它持账本锁），则同时解释了
"5 个空间退完、第 6 个消失"的形态。

### 3.5 `churn <n> 3 3` 在第 1 轮就 `spawn failed with code -1`（既有，未查）

`churn 2000 3 3` 在**改前改后都**立刻失败：`churn: spawn failed with code -1 after 1/3
at round 1`（`-1` 而非 `-4`，故不是 OOM）。`churn 500 2 2` / `churn 1000 2 2` 都正常。
提示方向：`depth 3`（孙任务再产子）或 `fan 3` 撞上某条非内存的资源上限
（名册？房间容量？）。与本轮的帧分配器无关，留作独立一条。

## 4. 两条驱动纪律（非内核问题，但曾把时间烧光）

1. **stdin 不关，`boot.nu` 就不退出** —— 即便 guest 早已 `system halted`。
   `{ printf ...; sleep N; } | ...` 里的 `sleep N` 一直握着写端，于是**每轮精确耗时
   N 秒**（60/240/300 s），与 guest 里发生什么**毫无关系**。guest 本身是亚秒级。
2. **写完立刻关管道也不行** —— 送 EOF，guest 可能**没跑完那条命令就退**
   （实测 `churn` 静默消失、只剩 `system halted`）。
   正解：**看 transcript 决定何时撒手**。
   另有输入被吞：首字符会被启动期吃掉（`churn` → `unknown: hurn`），
   启动慢的档（harden / debug）需要更长的 settle。

## 5. 诊断探针的硬约束（血泪）

**探针不得在回收路径上做堆分配。** 两次实测：探针里一次 ~15 KB 的有界分配
（`Vec<bool>` / `Vec<u8>` 按 `pagemeta.len()`）⇒ `churn` 从 3.5 s 变成**卡住不返回**。
要用固定大小的静态数组，或一次性跑完就撤。

## 6. 正确的诊断档是 `harden`，不是 release

```toml
[profile.harden]
inherits = "release"
debug-assertions = true
```

release 档把 freelist 的护栏**全部条件编译掉了**：
```rust
check_bounds(...)       { #[cfg(any(debug_assertions, feature="audit"))] ... }
check_frame_free(...)   { #[cfg(any(debug_assertions, feature="audit"))] ... }
check_not_in_chain(...) { #[cfg(debug_assertions)] ... }
```
⇒ **release 档下"零报警"不构成任何证据**。`debug` 档因一个既有的锁序违规
（`Space` 低层级锁被持有时又取同层锁，`lock/depend.rs:112`）在开机阶段就 panic，
故用 `harden`（能开机 + 断言全开）。

## 7. 压测工具自身的失败必须现形（本轮修复）

`churn` 的用途就是把池子压到耗尽，而**耗尽恰好发生在"产生任务"这一步**。
旧版有两处 `.expect()` 把压测自己压死了：

| 位置 | 后果 |
|---|---|
| `unit::closure(...).expect("churn: outer closure spawn failed")` | 外层生不出来 ⇒ 用户态 panic ⇒ 整个 `shell` 进程退出 |
| `fn tree` 递归里用 `unit::closure`（失败即 panic） | 递归某一层生不出来 ⇒ 同上 |

实测现场（修复前）：
```
exit tid=3434 reason=0xffffff01 note: task spawn failed:
  Error { context: "envcall", source: EnvError(-4) } at crates/runtime/src/core/unit.rs:110:20
```
——**压测没给出任何结论就死了**，harness 只能干等到超时。

修复后：`tree` 全程改走 `try_closure`（返回 `Result<Vec<usize>, EnvError>`），
内层 `join()` 的错同样上抛，外层 `match` 报 `code`。现在撞墙是**一条读数**：

```
churn: spawn failed with code -4 after 0/2 at round 1009     （128M）
churn: spawn failed with code -4 after 0/1 at round 2225     （64M）
```

### 边界验证（本轮，六次运行）

| 内存 | 配置 | 结果 |
|---|---|---|
| 64M | `churn 800 1 1` | 跑完 + 干净停机 |
| 64M | `churn 1500 2 2` | `code -4` + 干净停机 |
| 64M | `churn 3000 1 1` | `code -4 after 0/1 at round 2225` + 干净停机 |
| 128M | `churn 800 1 1` | 跑完 + 干净停机 |
| 128M | `churn 1500 2 2` | `code -4 after 0/2 at round 1009` + 干净停机 |
| 128M | `churn 3000 1 1` | 跑完 + 干净停机 |

**零 panic** —— 这是原始诉求（"我不希望 OOM 错误把内核 panic 了"）的直接验证。

### 附：harness 的假超时

偶发 `fast.sh` 耗满整个 `FAST_TIMEOUT`（60/90 s），而 transcript 里
**有 `system halted`、guest 确实自退了**。这是 harness 轮询/关管道的竞态，
不是内核挂起（实测 transcript 末行是 `task: all tasks exited, system halted`）。
遇到时先看 transcript 而不是计时。

## 8. `walk`/`meta` 背离：已定位到"孤儿表项"，但**未修复**（第 6 轮）

### 已确立的事实（两个自带自洽约束的核对，口径不含自造累加）

```
chainblk=7   mismatch=0        ← 正向：链上每块，其 pagemeta 表项都是
                                 Some(free=true, power=桶号)
freeent=7764 orphan=7758       ← 反向：7764 条空闲表项里，7758 条不在任何链上
osample=[(0,1), (2,0), (3,0)]  ← 孤儿出现在帧索引 0、2、3（含索引 0）
```

`mismatch=0` 恒成立（多次采样）⇒ **链上留下的块全是对的**；
`orphan≈7758` ⇒ **大量空闲表项对应的块不在链上**。这就是 `walk=437` 而
`meta=11919` 的原因。

### 为什么前五个探针没能定位（方法教训）

`chain_meta_mismatch` 只验了**正向**（链 → 表）。孤儿表项（表 → 链）从这个缺口
**整片漏掉**。前五个探针（`overrun`/`census`/`frnet`/`merge_census`/`reachability`）
的共同毛病是**缺少自洽性约束**，所以总能产出一个看似有信息量的数，而我就照着讲。
补上反向核对后，问题立刻现形。**这是本会话唯一有效的方法论。**

### 已确证（本节收尾）

矛盾解开了，病因在 **`split_block` 的逐级下降**：它 `pull_link(k)` 之后 `k -= 1` 而
`index` **不变**，每降一级都在**同一个 `index`** 上 `push_link(buddy, k)`；于是 index
处留下一条**更大 order 的粗粒度表项**，而那个大小的块早就不存在了。

轨迹实测（索引 1280 的完整一生，共 15 件事）末尾：

```
clear_head idx=1280            ← 下降前撤（后加的修复）
push_link idx=1280 power=6     ← 最后一次写表
[orph] idx=1280 power=6 bucket_len=3 addr_present_in_bucket=false
```

此后**再无任何** `remove_link` / `pull_link` / `clear_head` 触碰 1280，可它并不在
`freelist[6]` 里（桶只有 3 个节点、且无环）。所以"表说空闲、链说不在"是**真的**：
表里留着一条声明"1280 处有一个 64 帧的空闲块"的条目，而那块早已被拆走。

**为什么修不动**：那条表项同时是另一个问题的答案。`free()` 的护栏
（`check_frame_held` 的读侧 `held(pa)`）靠"覆盖该页的块首"判"这帧在不在手"，而从大到小
扫的第一条覆盖正是这条粗粒度表项。三次修法逐一撞墙：

| 试法 | 结果 |
|---|---|
| 只撤 `merge_block` 下降头 | 孤儿数不变（9 → 9）—— 不是那里留的 |
| `split_block` 下降前撤粗粒度表项 | 当场 `freeing non-held frame`——`held()` 的答案被撤掉 |
| 入链前扫掉"被遮蔽的祖先" | 换处 `allocated non-free frame`——同样撤掉了答案 |

根因是**表示法允许重叠**：`pagemeta` 里同时存在覆盖同一段的多条块首条目（祖先与后代），
而两位读者各取所需 —— `held()` 读"任何一种覆盖"，`in_freelist` 读"链节点自己那条"。
要么让表在块缩小时**立刻唯一化**（并让 `held()` 改问"这个索引有没有自己的表项"），要么
让链侧容忍祖先。两者都动核心不变量，另有代价要算。

现状：`health/stress.rs::chain()` 把这条编成**增量哨兵**（起点 `orphan=9`，churn 后必须
不增长），挡住回归但**不假装旧账已清**。旧账 9 条仍在池里，那些段永久脱离可用池。

### 别名：为什么"撤粗粒度前缀"这个显而易见的修法不成立

第四次尝试（撤掉 `split_block` 下降前的粗粒度前缀 + 由 `allocate` 在拆分后写最终块首）
**没炸、也没修好**：孤儿 9 → 10。这一版的负结果比前三次更有信息量 —— 它证明产出者不是
"漏了一次 clear"。

四个链操作挂索引轨迹后的读数（1280 的完整一生）：

```
push_link 1280 p=4 → remove_link 1280 p=4 → clear_head 1280
push_link 1280 p=5 → remove_link 1280 p=5 → clear_head 1280
push_link 1280 p=6                      ← 最后一次入链
[bucket] p=6 idx=1280 target=0x84109000 len=3 found=false pagemeta=Some((true, 6))
[chain] free_heads_at_p=5               ← 5 条表项自称 p=6 的块首，链上只有 3 个
```

**最后一次入链之后再无任何摘链**（`pull_link` / `remove_link` / `clear_head` 全无），
而它不在桶里。唯一自洽的解释是**别名**：同一个帧索引同时属于两个 order 的块，别的块在
**更高 order 的桶**里把它摘走（`remove_link(1280, 7)`），而 `pagemeta[1280]` 那条
"p=6 空闲"没跟着撤 —— 因为没有读者知道它指的是同一个帧。

`remove_link` 本身是对的（维护 `prev`/`next`、有 `check_in_chain` 护栏）。

**所以真修法是让"一块一表项"成为构造性保证**（别名不可表达），而不是在若干写点补 clear：
- 块缩小时把**跨度内所有**非块首表项一次性唯一化（代价 O(跨度)，正是早年"跨度清"被撤掉
  的那颗 50 倍慢 —— 故必须只对**实际被拆的那段**做，不能对整个 `2^power` 扫）；
- 或让 `pagemeta` 的索引语义与链节点的语义**对齐**（链只认一种真身份，表不再是"多值覆盖"）。

两者都动核心不变量，是独立一轮的事。

### 更根本的一层：`pagemeta` **自我不一致**（第 5 轮，判据已立）

上表那句"5 条表项自称 p=6 块首、链上只有 3 个"是从**计数器**读的，不可信（见下）。改用
两条**独立扫表法**对质，结果比"表 vs 链"更根本：

```
[scan] 表项：步进法=54 逐条法=197（差 143）错位=0 首个错位=(0, 0)
```

`pagemeta` 里实际有 **197** 条表项，而"按块首步进扫"（每条跳 `2^power`，这是本文件注释里
写明的**正确读法**）只数到 **54** 条 ⇒ **143 条表项落在别条声明的跨度里**（块内部）。
错位数为 0：每条表项对**自身**大小都是对齐的，所以不是对齐错误。

两种读法必有其一错。这条一旦成立，此前所有"表 vs 链"的对质都是在拿这 197 条里的
**不同子集**比较 —— 这就是为什么两个探针会互相矛盾（`freeent=32`，而逐 order 扫表求和得
34）。**缺自洽约束**正是本会话前五个探针全部落空的那个毛病，这里是它第五次出现。

`health/stress.rs::chain()` 现在断**这条差值的增量**（起点 143，churn 后不许增长）——
与孤儿那条同形：挡住回归，不假装旧账已清。

### 根因：这张表**容许重叠**（第 6 轮，把判据装到写点）

事后扫只能看到结果。把判据装到**写点**上（每次写 `pagemeta[index]` 之前先问"它是不是已经
落在别人的跨度里"），前几条现场直接指出来了：

```
[covered] push_link idx=512    落在 base=0     (free=false power=10) 的跨度内
[covered] push_link idx=17397  落在 base=17396 (free=false power=1)  的跨度内
[covered] pull_link idx=17397  落在 base=17396 (free=false power=1)  的跨度内
```

**行号——读法**：`base=0, power=10` 声明 [0,1024) 整块在手，而 512 是它的**中点**却被当块首
入链；`base=17396, power=1` 声明 [17396,17398) 在手，而 17397 是它的**中点**同样入链。
其中 `free=false` 覆盖是更重的一档：入链的那个索引**已被算进另一笔在手分配**。

**量级说明它不是边界情况**：启动期一次运行累计 **31376** 次，多次运行稳定。

⇒ `pagemeta` **是一张容许重叠的块首图，不是"块首 → 块"的函数**。这就是两条扫表法差 143 条的
根：它们（以及 `held()`、`in_freelist` 的自证读法）都建立在"表项互不重叠"这个**表并不满足**
的假设上。

**这也解释了修法 A/B 为什么都撞墙**：A 想靠"块缩小时唯一化"消灭别名 —— 但重叠是常态，
逐个撤无法穷尽；B 想靠"这个索引有没有自己的表项"回答"在谁手里" —— 同样以不重叠为前提。

**真修法只有一条**：先决定这张表是"块首 → 块的函数"还是"容许重叠的覆盖图"，然后让**写侧
与读侧同守那一个决定**。现状是两种假设混用 —— 写侧按覆盖图写（`push_link` 从不检查覆盖），
读侧按函数读（`held()` 假定唯一答案、`free_entry_orphans` 假定每条表项都是块首）。

第三个哨兵已立：`[covered] 累计 31376 → 31376`（churn 后不许增长）。

### 产出者抓到了：**释放"在手块的中间帧"**（第 7 轮）

`[covered]` 的分类计数把方向钉死了：

```
累计 31376：push→free=0  push→held=10570  pull→free=0  pull→held=8711  clear→free=0  其它=12095
```

**被覆盖者一律是 `free=false`（在手块）**，而"空闲块被切开的残留"这个我先前以为的主要来源
是 **0**。也就是说：写入发生在**已分配块的跨度的中点**。

顺着这条，给 `deallocate` 加了一条判据（"释放的是不是某在手块的中间帧"）：

```
[interior] 释放在手块中间帧 累计 1811 → 1811     ← 仅启动期
[interior] freeing interior frame idx=4078 addr=0x84bf7000 power=0
           → 落在 base=4076 (power=2) 的在手块跨度内
```

**仅启动期就 1811 次**。机制闭环：每释放一个中间帧，分配器按帧处理 ⇒ 把中点当 `power=0` 的
块首入链 ⇒ 表里多一个不存在的块首 ⇒ 正是那 31376 次覆盖写入，也解释了两条扫表法为何差 143 条。

**为什么现有护栏全都放行**：`check_frame_held` 问的是"在不在手"，而"在手"的判据 `held(pa)`
按**覆盖**回答 —— 中间帧照样答"在手"。判据问错了问题，所以它一直沉默。

**还没查到的**：是**谁**在放中间帧。`index 4078` 落在一个 4 帧（16 KiB = `TASK_STACK_SIZE`）
的分配里，在 `pool init` 之后立刻发生。`#[track_caller]` 逐层加上去只能到
`core::alloc` 的 trait 默认转发（`alloc/mod.rs:593`），落不到调用者。

判据已立成**增量哨兵**（`[interior] 累计 1811 → 1811`）。找到调用者后这一条应升回 panic。

#### 形态：**嵌套升级**，不是"丢一页不管"

四条样本（跨运行稳定）：

```
idx=17397 addr=0x87fff000 power=0  base=17396 bpower=1
idx=4081  addr=0x84bfb000 power=0  base=4080  bpower=4
idx=4079  addr=0x84bf9000 power=0  base=4078  bpower=1
idx=4084  addr=0x84bfe000 power=1  base=4080  bpower=4
idx=4088  addr=0x84c02000 power=2  base=4080  bpower=4
```

同一个 `0x84bf…` 区间**逐步升级**：4079 落在 4078 的 2 帧块里，4081/4084/4088 落在 4080 的
16 帧块里。所以这几笔像是**一串连续的嵌套释放**，而非"丢一页就不管"。但按语义，释放一个
在手更大块的中间帧会**丢掉该块余下的帧**（1811 次 × 平均 ≥3 帧 ≈ 20 MiB）—— 那个量级的泄漏
在 boot 期就该把池子吃穿。**两者矛盾，我尚未解释**，故不把"1811"读作泄漏速率。

#### 定案读数：判据是准的，**丢帧型 1811 / 块首误判 0**

把命中拆成两类（本征信号：调用者报的 `power` 与该块实际 `bpower` 的关系）：

```
[interior] 释放在手块中间帧 1811 → 1811；其中丢帧(power<bpower)=1811、块首误判=0
```

**1811 次全部是"丢帧"型**，块首误判为 0 ⇒ 判据没有把块首误认成中间帧。

再判别"是不是正在切分的中间态"（读表时那个大块是否已切出子块入链）：

```
[iprobe]   idx=17396 free=false power=1 在链=false
[iprobe] 判别：段内子块 0 个在链、1 个在手（base=17396 bpower=1）
```

段内**没有**子块在链上 ⇒ **不是**切分中间态。span 里只有一条表项且 `free=false` ⇒ 那确实是
一笔在手的块，而释放的是它的中间帧。

#### 仍未解决的矛盾（下一步的钥匙）

若 1811 次各丢 ≥1 帧，约 20 MiB 从未归还 —— **而 boot 正常、`stress` 与 `pagetable` 用例
照过**。两者不能同时为真。已排除的解释：

| 试过的解释 | 排除依据 |
|---|---|
| 判据把"块首说空闲而体内有在手帧"误读 | `iprobe` 显示 span 里只有一条 `free=false` 表项 |
| 在读"正在切分"的中间态 | 判别读数：段内 0 个子块在链上 |
| `GlobalAlloc` 的 size 路由缝隙 | `PortalAllocator` 原样转发 `layout`，`HybridAllocator` 按 `size ≤ 半页` 分流，两侧同源 |

**唯一能定案的一步**（建议下一轮先做）：在用例结束时核对**守恒量** ——
`累计分配帧 − 累计释放帧 == 在手帧 + 空闲帧`（`frame_ledger()` 与 `pagemeta` 两口径）。
若空闲帧恰好少 1811×k，则真丢帧、且泄漏速率与"boot 未 OOM"的矛盾说明池子远大于我的估计；
若守恒成立，则这 1811 次释放**确实归还了帧**，我的判据在某个尚未看清的语义上误报。

#### 调用者地址（线索，非结论）

用 `fence::alloc_site`（帧指针链逐层读 `ra`，与 `on_alloc` 的分配点捕获同源）取前几层，
跨运行稳定：

| 样本 | 前几层 `ra` |
|---|---|
| `base=17396 bpower=1` | `0x80235ae0` |
| `base=4080 bpower=4` | `0x8020e48c` / `0x8023e8cc` |

`addr2line` 指向 `kernel/src/main.rs` 中 `allocator::init` 闭包附近。**只作线索**：
符号化用的二进制与产出日志的那份布局不同，位置只能近似 —— 要定案得在同一份 ELF 上取地址。

#### 顺带修掉一处读数骗局

`free_block_census` 原读 `META_FREE_BLOCKS` 计数器，而它**只在 `push_link` 加、只在
`clear_head` 减**，`pull_link` 取出时**不减**（直接覆写成 `free=false`）⇒ 计数器单调虚高。
已改为**直接扫表**。教训同 docs §3 首条："计数器会骗人，判据只能直接问权威结构。"

#### 另修一处判据盲区

`chain_meta_mismatch`（正向）原先在三处 `break`：一遇到"表里没有它"或地址出池，就**丢掉
整条链的余下部分**，于是那条节点之后的一切都不计入 `bad`。实测正是这么漏掉过一条
（逐 order 扫表出现"链=1、表=0"而它报 `mismatch=0`）。已改为**记违规并继续走**。
判据的盲区比判据的错更坏：错会响，盲区只会沉默。

### 早期记的（已由上面取代，留作史料）

`pagemeta[X] = Some(free=true)` 全仓只有 **三处**写点：

| 行 | 内容 |
|---|---|
| 845 | `pull_link`：`Some(false, power)`（取出） |
| 907 | `push_link`：`Some(true, power)`（入链）——**无条件**执行 `freelist[power] = Some(addr)` |
| 1028 | `merge_block`：`pagemeta[buddy] = None`（清伙伴） |

`init` 也走 `push_link`。所以"有 `free=true` 表项而其块不在链上"**按代码不该存在**。

实测反证：
```
in_freelist MISS idx=10626 power=0 target=0x82c52000
  seen=[0x83ec9000, 0x83ecc000, 0x83ee4000]（freelist[0] 的全部节点）
```
`freelist[0]` 实际只有 1~4 个节点，且都是 `0x83ec…` 段；而 `pagemeta` 里同一时刻
有成千上万条 `free=true`（`power=0`）表项指向 `0x82c…` 段。**两边说的是不同的地址**。

要么 `push_link` 在某些路径上没被调用（但已确认它无条件入链），要么链在中途被
成批摘除而表项未同步。**未定位**。

### 下一步（窄且明确）

把 `freelist[o]` 与 `pagemeta` 的 `free=true` 表项**按地址逐一比对**（两个方向都打），
看差异的集合形状：若 `pagemeta` 多出的地址是**整段连续**的，则说明某次拆分/合并把
一整段摘出了链却没清表项；若是**散点**，则是逐块的路径问题。

## 9. 第 8 轮：低位段在高/低 order 间反复转换（机制已见形状）

### 观测

`pull_link` 记录"池子起步处（index < 4）被取出"的现场：

```
low pull: index=0 power=12 (1)    ← 起始处先按 order-12（4096 帧 = 16 MiB）取出
low pull: index=0 power=12 (2)
low pull: index=0 power=3  (3)    ← 同一个 index 0 又按 order-3 取出
low pull: index=1 power=0  (5)
low pull: index=2 power=1  (6)
low pull: index=3 power=0  (7)
```

### 含义

池子**起步处的帧在反复分配-释放-再分配**，且**同一个 `index` 在不同 order 上反复出现**
（index 0 先 12 后 3）。而最终 `pagemeta[0]` 留下的是 `power=1` 的 `free=true` 表项，
`freelist[0]` 里却没有它（第 6 轮 `p0chain=0 / p0entry=6097`）。

所以 (a) 的性质**不是"某一步漏清表项"**，而是**低位段在高 order 与低 order 之间反复
转换**：每次转换都可能在 `pagemeta` 上留下与链不一致的痕迹（大块被拆成小块、
小块又合并回大块的过程中，表项与链的更新不同步）。这是"`walk` 塌陷而 `meta` 不变"
的机制来源。

### 仍未定位

具体是哪一次"转换"留下了不一致的表项（`split_block` 的逐级 `push_link`？
`merge_block` 的 `index = min(index, buddy)` 换首？）。两者都读过，静态看都自洽，
所以需要**在转换发生的那一刻**核对，而不是事后看终态。

## 10. 第 10 轮：结案 —— 帧没丢，两处写点缺陷（守恒核对定案）

### 10.1 判据：守恒快照（单锁一次取全 + 自洽残差）

`FrameAllocator::conserve()` 在**一次持锁**内算完全部读数，并给出**残差**这一条自洽
约束 —— 这是本会话前九个探针都缺的东西（它们缺少自洽约束，于是总能产出一个看似有
信息量的数，然后被下一个探针推翻）。

```
(S) 总 == 在手 + 空闲 + 洞 + 无主        无主 = 步进踩到、不属于任何块也不是保留区的帧
(C) 链 == 空闲 + 跨度内空闲 − 未入链表项
(L) 累计分配 − 累计释放 == 在手
(R) 残差 == 0                            模型若完整覆盖池子则恒为 0
```

判决读数（框架档，128 M，churn 前后各一次）：

```
总 17397 = 在手 735 + 空闲 14128 + 洞 2534 + 无主 0（账 15211-14476=735）
链 walk=14128 表空闲=14128 差=0 跨度内空闲帧=0 残差=0（链上不符=0）
账↔表：表在手=735 vs 账在手=735 ⇒ 差=0
表项 步进=178 逐条=178；未入链 0/0 帧；链节点=17
```

`无主 = 0` 且 (S) 精确成立 ⇒ **帧一帧都没丢**。上一轮那对矛盾（"1811 次释放中间帧
× 每次 ≥1 帧 ⇒ 该少 ~7 MiB" vs "boot / 用例无恙"）由 `(L)` 裁决：修前
`表在手 − 账在手 = +528`，而它**恰好等于**同一快照里"跨度内空闲帧"的 528 帧 ——
表把已经归还的帧仍算作在手，账是对的。**没有帧丢失，也没有二次记账。**

### 10.2 根因一：`split_block` 的粗表项

旧版 `pull_link` 按**取出时的桶号** `k` 写 `pagemeta[index] = Some(free=false, power=k)`，
而 `split_block` 每降一级就把伙伴推回空闲桶 ⇒ 那条表项声明的跨度**比本分配大**，
覆盖了已经空闲的伙伴。后果：

- `held()` 按**覆盖**回答 ⇒ 对已归还的伙伴帧答"在手"（护栏被绕过）；
- 两条扫表法必然不等（步进 50 / 逐条 201，差 151）；
- `interior_frees` 命中 1811 次 —— **全部是它造成的误判**（释放"邻居块的首帧"时，
  `interior_of_held` 命中了那条粗表项）。上一轮据此写下的"释放在手块中间帧"结论
  **是错的**。

修法（O(1)）：`pull_link` **只撤不立**，`split_block` 在拆分收尾按**最终 order**
写一次块首表项。

### 10.3 根因二：静默失效的指针写（`x.read().prev = …`）

```rust
// 错：`NonNull::read()` 按值返回，赋值落在**临时副本**上 —— 编译通过、不报错
n.read().prev = None;                 // pull_link
head.read().prev = Some(addr);        // push_link
// 对：写回原处
(*n.as_ptr()).prev = None;
(*head.as_ptr()).prev = Some(addr);
```

于是双向链表的 `prev` **从未维护**。而 `remove_link` 正是用 `prev == None` 判
"我是桶头"：摘除一个**链中间**节点时走错分支，执行 `freelist[power] = <该节点的陈旧 next>`，
把**真正的桶头**覆盖掉 ⇒ 链头那几个块从桶头再也走不到，却仍留着 `free=true` 表项。

取证（新增 `[stale-rm]`）：启动期 **8 次**覆盖，逐条打印"自称桶头 idx=X、真桶头=Y"。
被顶掉的正是那些孤儿：`11 条表项 / 622 帧（2.4 MiB）` 声称空闲却不在任何链上 ——
即"表说空闲、链上找不到"的 622 帧。这也是 `nomerge` 28 次（"伙伴说空闲但不在链上
⇒ 放弃合并"）的来源。

修法：两处都写回原处。修后 `stale-rm = 0`、`orphan = 0`、`nomerge = 0`。

### 10.4 附：删掉为补偿根因一而加的"跨度清扫"

`push_link` 曾遍历 `2^power` 个槽、清掉跨度内的空闲表项 —— 它是为根因一的粗表项打的
补丁。根因一在源头修好后它**零命中**，而它每次入链都要扫 `2^power` 槽（历史实测把
`churn` 拖慢约 50 倍），故删。删后全部不变量仍为 0。

### 10.5 修前 → 修后

| 读数 | 修前 | 修后 |
|---|---|---|
| 步进法 / 逐条法 | 50 / 201（差 151） | **178 / 178（差 0）** |
| 链 − 空闲 | −114 → −642 | **0** |
| 未入链表项（孤儿） | 11~13 条 / 622~642 帧 | **0** |
| 跨度内空闲帧（粗表项痕迹） | 528 | **0** |
| 表在手 vs 账在手 | +528 | **735 = 735（差 0）** |
| `covered` 写点 | 31376 | **0** |
| `interior` 命中 | 1811 | **0** |
| `nomerge` / `stale-rm` | 28 / 8 | **0 / 0** |
| 自由链节点数（同一池子） | 27 个节点 / 13487 帧 | **17 个节点 / 14128 帧**（合并正常了） |

### 10.5b 用户可见的读数：`churn 1000 2 2` @128 M（release 档）

池子水位那行（`reap.rs` 每 200 笔打一次）是**改前改后差别最大的地方**：

| 读数 | 改前 | 改后 |
|---|---|---|
| `walk` / `meta`（走链 / 读表） | **902 / 27974**（差 **27072**） | **27974 / 27974**（差 **0**） |
| `orphan`（表说空闲却不在链上） | **15049** 条 | **0** |
| `p0chain` / `p0entry`（order-0 链节点 / 表项） | **2 / 12183** | **22 / 22** |
| 同一命令的完成度 | 3 次里 2 次在 **982~990 轮 OOM**（`-4`） | **3/3 次跑完 1000/1000** |

改前 `walk=902` 而 `meta=27974`：**99% 的空闲帧在链上根本走不到**（15049 条表项不在
任何链上）。池子并非真的空了 —— 分配器是"看不见自己有的内存"，于是 churn 跑到九百多轮
就以 OOM 收场（历史读到的"OOM 出现的轮数飘忽"正是这个）。改后两口径逐帧相等，
同一命令稳定跑完。

（OOM 仍然**不 panic** —— 那是上一轮已交付的判据；本轮只是让它不再被触发。）

### 10.6 方法教训（写在这里最值钱）

- **探针必须有自洽约束**。`conserve` 的残差 (R) 一加，前九个探针里"看似有信息量"
  的数当场失效 —— 它们量的是同一现象的不同侧面，却各自缺一条把它们钉在一起的恒等式。
- **不要把"我造出来的现象"当成"系统的性质"**。我曾据 `covered = 31376` 断言
  "`pagemeta` 是一张容许重叠的覆盖图、不是块首到块的函数"，并推出"修法只有一条：
  先决定这张表是什么"。**那个前提是错的** —— 31376 次覆盖全部来自**一个写点**。
  迎合现象设计的大修法（A/B 两条）都是多余的。
- **静默失效比 panic 危险**。`x.read().prev = v` 编译通过、不报警、不崩溃，症状要等
  上千次操作后才以"桶头被覆盖"的形式出现。故新增 `chain_audit`：逐桶走链，核对
  `prev`/`next` 与前后邻居互相回指 —— 这才是能**当场**抓住它的判据。

### 10.7 长期判据（框架档，绝对零）

`health/stress.rs::chain` 的断言从"断增量"升格为**绝对零**（此前做不到，因为池里
确实躺着旧账）：步进=逐条、链=空闲、表在手=账在手、未入链=0、无主=0、残差=0、
`covered`=0、`interior`=0，另加 `chain_audit` 逐环核对双向链表。
