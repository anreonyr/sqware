# Team 字段与方法对偶评审

> 已被 [Unit 域统一重设计](unit-domain-redesign.md) 取代。以下保留为局部评审历史；Team、Task 与调用契约请以新主稿共同评审。

状态：设计建议，未修改 Team、调度器或 ABI。依据 PR #5 的 `kernel/src/work/unit/team.rs`，并结合 Task 发布、收尾和子树控制的调用点。与 [UnitCall 优化评估](unit-call-design-optimization.md) 一起评审。

## 设计约束

Team 表达资源域、成员关系和域级运行约束。Shell 的主任务选择、作业失败判断和测试期望不成为 Team 的字段。

方法命名要表达作用对象、范围和动作：相反动作使用对偶动词；查询与修改必须针对同一个状态；一次性提交不伪造可逆操作；可失败操作用返回类型说明失败，不能靠模糊后缀或静默降级。

## 字段逐项决策

| 字段 | 建议 | 理由与约束 |
| --- | --- | --- |
| `id/space/sire` | 保留既有身份、资源与创建者关系 | `sire` 是 TaskWeak，不能直接充当永久有效的暂停继承链 |
| `tasks/held` | 保留两种所有权，统一成员发布与移除协议 | tasks 是弱引用成员表；held 是首次放行前的强引用保活表，不是重复成员状态 |
| `default_entry/ready/staged` | 作为构造职责一起评审 | 默认入口与 staging 在首次提交建立；ready 表示构造提交，不表示调度就绪 |
| `operating` | 保留互斥职责，明确覆盖的操作 | 与 ready 不可合并：已提交的域仍可进入互斥操作；当前 guard 叫 Construction，但 operation 也用于构造以外的调用 |
| `paused` | 保留必要的本地暂停门，候选名 `paused_local` | 仅本域显式设置，不缓存祖先状态，不替代 Task 的 stopped 或 Held |
| `revision` | 暂列待证明字段，确有需要时改为 `membership_version` | 泛称 revision 无法说明哪些变化被保护；计数器不是成员发布和恢复的原子性证明 |
| `completion` | 删除当前 Team 级聚合模型 | 首个 Spawn 自动成为 representative、首个非零退出变为 fault，均包含上层策略；退出事实按 Task 记录，所有权依 UnitCall 评审另定 |
| `observed/observer` | 删除 | observed 的本地位、祖先查询和创建时保存的 observer 同时表示“被观察”，会混淆事实与故障处理策略 |

不把这些字段压缩成通用 flags，也不把所有状态装入一个大锁来追求字段少。构造、成员、调度门和结果记录的锁域与寿命不同。是否引入 ConstructionState/Membership 子结构，应由锁顺序和提交边界决定，而不是仅把字段搬到嵌套结构中。

## 新增方法的命名与去留

| PR 方法 | 建议契约与命名 | 对偶或对称关系 |
| --- | --- | --- |
| `set_paused(bool)` | 拆为 `pause()` / `resume()`；仅修改本地门，保持内部可见 | pause ↔ resume；调用点直接表达动作，不传含义不明的 bool |
| `paused()` | 本地查询 `paused_local()`；继承后的查询 `paused_effective()` | 与字段及修改范围一致；两查询使用相同词根、明确不同范围 |
| `revision()` / `changed()` | 若保留，采用 `membership_version()` / `advance_membership_version()` | 前者读成员版本，后者推进相同版本；changed 是动作却用过去分词，且未表达对象 |
| `representative(task)` | 删除 | 主任务由 Control 显式持有，不在 Team 中用首次发布隐式选择 |
| `completed(task, reason)` | 删除 Team 级聚合；在退出记录对象评审 `complete(exit)` / `completion()` | complete 是动作，completion 是事实查询；必须区分开始退出与 hooks 完成，不直接改名保留当前调用时机 |
| `status()` | 删除当前 `(usize, Reason)` 聚合查询 | 由有类型的 TaskExit/JoinReply 表达事实，不保留魔数状态 |
| `observe()` / `observed()` | 删除 | 不为不应存在的政策状态补 unobserve；删除该职责比补形式上的对偶更合适 |
| `tasks_checked()` | 与既有 `tasks_snapshot()` 合并为 `snapshot_tasks() -> Result<…>` | 与 Task 的成员枚举统一为 `snapshot_heirs() -> Result<…>`；快照不是 checked，后者未说明检查什么 |

pause/resume 是 Team 内部本地门动作；UnitCall 的 Embark/Debark 仍承担授权、首次放行或整个子树停止确认。内部 resume 返回不等于 Embark(Tree) 已完成所有恢复动作，内部 pause 返回也不证明运行成员全部离槽。不要让名称相同掩盖不同完成边界。

不是每个方法都要有反义方法。完成记录是一次性事实，只有 complete/completion，没有 uncomplete；版本只能推进，没有撤回；首次构造提交也不提供假想的“取消提交”。对偶用于可逆动作，对称用于相同对象的读写及相同种类的枚举。

## 既有方法一起检查，但不捆绑无关改名

- `operation()` 返回的 RAII guard 在 Drop 解锁，释放路径已有对偶。若保护构造、映射与 Oust 等域操作，建议命名 `try_operate()` / `Operation`；若只保护构造，则改为 `try_construct()` / `Construction` 并另定其他操作同步。应先固定锁域再选其中一组，不能仅重命名 guard。
- `default_entry()` / `set_default_entry()` 当前是 OnceLock，set 不是任意覆盖。建议 `default_entry()` / `init_default_entry()`，并显式处理重复初始化，不增加 clear_default_entry。
- `cancel_staging()` 要与实际暂存入口成组定义为 `stage()` / `unstage()`：这里只表示资源预留与撤销。首次提交是不可逆的所有权转移，另用 `commit_staging()`，不能把 commit 和 cancel 当作互逆。
- 成员发布/移除可统一 `attach_task()` / `detach_task()`，取代外部直接 push 与 `prune_tasks()` 的分散修改；移除指定成员与清扫失效弱引用应分开，后者可保留 `prune_tasks()`。这是候选边界，不要求在持有调度锁时调用会分配或再次取锁的封装。
- `release_held()` 是单向首次放行，不能新增 hold() 把运行任务重新变成 Held 来凑对偶。暂停走独立门。

## 字段成立所需的并发与寿命证据

1. **暂停继承链**：目前 paused/changed 经 `sire.upgrade()` 沿父 Task 向上走。父 Task 的强引用消失时继承链会断，除非生命周期契约已经保证后代在此之前全部结束。需证明该契约，或设计寿命稳定的域级祖先关系；不能未经设计就增添强 parent 引用并引入资源保留。创建者身份和控制祖先不是同一关系。
2. **成员版本覆盖**：当前 changed 在 Task 发布和 Team 创建后调用，Task 清扫及 heir Oust 没有相同更新。若版本表示成员结构，发布、移除、收养、放下都需纳入同一个提交协议；祖先传播的起点、终点与溢出规则也要确定。
3. **版本校验不是事务**：成员加入与版本推进之间有窗口，校验之后也可能再发生发布。暂停门如何阻止入槽、恢复如何接纳新成员、parked 取出和启动之间如何同步，必须有明确线性化点；不能仅增加 revision 再检查就认为完整。
4. **快照失败**：现有 tasks_snapshot 在 OOM 时返回空集合，新增 tasks_checked 则返回错误。同一事实不应有两种真假语义。合并后逐个迁移调用者：控制操作返回失败；诊断可展示“快照不可用”；销毁路径若必须无分配完成，需要专门的遍历协议，不能把 OOM 当作空域。
5. **退出完成时机**：当前 completed 在 hooks 之前调用，正常 quit 后 reap 又会调用一次。新的完成记录必须准确记录真实 cause，并在承诺的 hooks 完成边界发布；不能将现有聚合函数原样移到新结构。

## 本轮评审结论

确定方向：移除 Team 中的主任务/故障政策字段；暂停动作使用 pause/resume 对偶并区分本地和继承查询；统一成员快照名称和失败语义；一次性动作不强造反向方法。

仍需决定：暂停祖先的寿命契约、成员版本是否必要及其同步协议、构造互斥范围、退出结果记录的所有者与交付路径。上述决策关闭后，才给出可落地的最终 Team 结构与方法签名，并与 UnitCall 同步改实现。
