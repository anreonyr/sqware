# churn 下 OOM 的诊断记录

> 本文只写**有判据支撑**的部分。被推翻的假设也列出（避免下次重走）。

## 0. 结论摘要

- **核心诉求已达成**：OOM 以 `EnvError(-4)` 返回用户态，**不再 panic 内核**。
  `churn 500 2 2`（原始失败用例，原版死在第 185 轮）现于 **~1.5 s 跑完并干净停机**。
- **两个未修的深层缺陷**（见 §3），都在 `--features audit` 或深档才显形。

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

### 3.1 `walk` 与 `meta` 长期背离（**未定位；下面的早期记述已部分被推翻**）

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
