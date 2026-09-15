# sqware docs — 机制设计文档

本目录只写**地板**：结构与撑住结构的内核底座。

上层（协议 / 服务 / 程序）**目前是空的**——旧的那一整套在 tag `proto-v1-baseline` 里，
连同它的四份文档（`console.md` / `dispatch.md` / `root.md` / `driver.md`）与**根
`README.md`** 一起归档。新协议成形时**按新形状重写**，不从那一套里搬：搬结构就是把上一版
的病带进下一版。本树的第一眼因此从 `docs/` 进。

阅读顺序建议按下面的表往下走——它也是依赖顺序：先说清"东西存在哪"（space / memory），
再说"谁在动它"（task），再说"凭什么能动"（pie / mail）。

## 结构

| 文档 | 机制 | 一句话 |
|---|---|---|
| [space.md](space.md) | Space / Map / Window / Seg | 地址空间：一段 VA 怎么被登记、物化、借入与回收 |
| [task.md](task.md) | Task / Team / 血缘 / 调度 | 执行单元：产、放行、运行、等、死（两相）与回收 |
| [pie.md](pie.md) | Pie（权柄） | 门闩：权限位、子集转让、派生边、级联撤销、寿命 |
| [mail.md](mail.md) | Mail（Hole / Pole / Nole） | 数据面三件套：有槽、有页、什么都没有 |
| [port.md](port.md) | Port（Hole 的通讯协议） | 授出（`Access` × `Policy`）、坐标 `To`、**两枚孔的配对** `Port`（`open`/`push`/`pull`/`shut`）；往返与帧格式归各协议；**已实现**（含 §5 舍弃 `mtu` 的变长孔、§10 的分层裁决）；判据见其 §9 |
| [bell.md](bell.md) | Bell（Nole 的 runtime 封装） | 空载荷门铃：内核一位「有待取之事」+ 听者面；建域权那条判据为什么不用改；**已实现** |
| [dock.md](dock.md) | Dock（Pole 的 runtime 封装） | 借映 → **视图**（起点与长度成对）；`Open` 返两件；`Shut` 不过存活闸；**已实现**，判据见其 §9 |
| [memory.md](memory.md) | 帧与页表 | 帧分配器、pagemeta 一份账、页表 / ASID、缺页 |

## 底座

| 文档 | 机制 | 一句话 |
|---|---|---|
| [switcher.md](switcher.md) | 陷阱 / 切换 / envcall 入口 | 用户态怎么进来、怎么回去、怎么退场 |
| [abi.md](abi.md) | ABI 面与分层 | 8 个 class 的划分轴、codec、五层依赖方向 |
| [lock.md](lock.md) | 锁与锁序检查 | `SpinLock` / `RwLock` / `RelLock` + lockdep |
| [diagnose.md](diagnose.md) | 诊断与现场转储 | panic 只报一次；现场、事件流、结构化导出 |

## 约定

- **"判据"一节**写的是「怎么知道它是对的」：自检命令、验收门的断言、审计档的计数。
  没有判据的机制不算完成——这是本仓的一贯口径。
- **"已知边界"一节**写没解决的事，不包装成待办，也不藏。
- 代码锚点写作 `路径/文件.rs:行` 或模块名，对应写下时的仓库状态。
- 术语以代码注释为准：门闩 / 孔 / 槽 / 站点 / 信标 / 躯壳 / 名册 / 窗 / 段 / 簿记 /
  停摆 / 窄尾。**不要**用外来词替换它们（它们各自都指一件很具体的事）。
- 本目录里的文档若与代码不一致，**以代码为准**；上层那四份的引用（散在 `abi.md` /
  `bell.md` / `dock.md` / `pie.md` / `port.md` 里共 13 处）指向的是 tag，不是本树。
