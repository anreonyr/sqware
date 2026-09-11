# sqware docs — 机制设计文档

本目录按根 `README.md` 的**脉络**组织：先结构（Space / Map / Task / Team / Pie / Mail），
再协议与服务，再撑住这些机制的内核底座（陷阱、ABI、锁、诊断）。

阅读顺序建议按下面的表往下走——它也是依赖顺序：先说清"东西存在哪"（space / memory），
再说"谁在动它"（task），再说"凭什么能动"（pie / mail），最后才是"怎么组织成系统"
（dispatch / console / root）。

## 结构（README 的 Core Structures）

| 文档 | 机制 | 一句话 |
|---|---|---|
| [space.md](space.md) | Space / Map / Window / Seg | 地址空间：一段 VA 怎么被登记、物化、借入与回收 |
| [task.md](task.md) | Task / Team / 血缘 / 调度 | 执行单元：产、放行、运行、等、死（两相）与回收 |
| [pie.md](pie.md) | Pie（权柄） | 门闩：权限位、子集转让、派生边、级联撤销、寿命 |
| [mail.md](mail.md) | Mail（Hole / Pole / Nole） | 数据面三件套：有槽、有页、什么都没有 |
| [memory.md](memory.md) | 帧与页表 | 帧分配器、pagemeta 一份账、页表 / ASID、缺页 |

## 语义（README 的 Protocol & Service）

| 文档 | 机制 | 一句话 |
|---|---|---|
| [dispatch.md](dispatch.md) | 服务目录协议 | 名字 → 预约者 + 实例；`Connect` 就是转授门闩 |
| [console.md](console.md) | 控制台协议与服务 | 唯一读 UART 的服务；会话、行编辑、回信孔 |
| [driver.md](driver.md) | 设备与驱动的基础 | 设备 = 一段有主的、可映射的内存；中断门；驱动/服务边界；**线的权威与收线**（**已实现**：三步 + §12 两条都走完，门绿；实现期 13 条裁决待裁，见其 §8.1） |
| [root.md](root.md) | 根服务域 | boot 只装 root；其余子域由它建；退出即自然停机；另提供 `kill` 的他杀服务 |

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
