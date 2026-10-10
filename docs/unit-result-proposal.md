# Unit 收尾接口历史提案

> 已由 [统一评审稿](unit-domain-redesign.md) 取代。最新方案将 Hear 并入 Join，并用 Team.tasks 成员记录替代 Task.results，不新增 Oust(Task)；以下消费式 Join 与“没有 Oust(Task)”均已撤回，不作为实现依据。

# Unit 收尾接口新方案

状态：待评审的替代方案，未改实现。替代 unit-domain-redesign.md 第 6–13 节的旧 Collect → Join → Oust(Task) 结果协议；不据此固定 ABI 编号。保留 TaskState/TeamState 单一事实设计方向。

## 核心决定

保留两个职责清楚的等待入口，返回同一种完成结果：Join 等指定任务，Hear 等指定 owned Team 子树的下一份成员结果。成功即领取调用者的结果项，不返回还要另查的结果句柄，不移交新的任务权限。

```rust
struct TaskExit {
    task: TaskId,
    cause: ExitCause,
    reason: Reason,
}

enum JoinReply {
    Pending,
    Reaped(TaskExit),
}

Join { task: TaskId, millis: Wait } -> UnitResult<JoinReply>
Hear { team: TeamId, millis: Wait } -> UnitResult<JoinReply>
Oust { team: TeamId } -> UnitResult<()>
```

没有 Oust(Task)，没有 Hear 返回 TaskId 后再建立 results 读取权的步骤。TaskExit 是调用者拿到的值，不需要内核继续替它保留结果。TaskState::Reaped 保存唯一内核退出事实，等待接口只生成返回快照。

## 一个例子

Shell 拥有 Team 10，它有主任务 101 和辅助任务 102。102 先以 reason=17 退出：

```text
Hear(Team 10, Forever) → Reaped { task: 102, cause: Reap, reason: 17 }
```

这一次调用就知道谁完成、怎样退出，并领取这份结果。调用者不再 Join(102)，也不再 Oust(Task 102)。

只想等主任务时：

```text
Join(Task 101, Forever) → Reaped { task: 101, cause: Reap, reason: 0 }
```

101 的结果已被这个 owner 领取，后续 Hear 不再向同一个 owner 返回 101。其他辅助结果仍可 Hear。子树全部收尾后，Oust(Team 10) 放下域和它尚未领取的结果项，不等物理回收。

## 结果寿命与消费

- Spawn 前预留 TaskState 与必要的收尾记录链接。Reaped 无 Task/Team/Space 的资源强引用，任务先埋葬也不会让未领取结果消失。
- Spawn 创建者具有该任务的结果项；owned 根的 owner 具有后代的完成视图。多个 owner 引用同一 TaskState，不复制 cause/reason，不共享一个全局消费位。
- 同一个 owner 对同一 task 最多持有一个结果项。Join 按 ID 找它，Hear 按 owned 根筛选它；不同入口使用索引定位同一项，不能各持有一份待领取结果。
- 若同一 owner 同时因 Spawn 和根视图拥有该任务，创建时合并为一项，消费时同时移除其查询/队列索引。不同 owner 仍可以各自领取自己的项。
- 返回 Reaped 的提交点摘掉这个 owner 的项并写回三字返回；Pending、超时或失败不消费。不向用户缓冲写回，避免写失败导致领取丢失。
- 多等待者竞争同一 owner 的同一项只有一个成功领取，其余被唤醒并返回 Denied，不能冒充 Pending 或 reason=0；重复 Join 亦为 Denied。不同结果项可并发领取。
- owner 死亡丢弃它尚未领取的项；Oust(Team) 丢弃该 owner 在该子树的剩余项，不删除其他 owner 的项。
- 长期存活的 owner 必须领取已创建任务的结果，或最终放下对应 Team；未领取项受额度限制，满时拒绝后续创建，不覆盖旧结果。不提供活 task 的单项 detach/放弃接口，本版不支持这个用法；若现有调用方需要它，必须在实施前补正式契约，不能借 Join(POLL) 偷偷放弃未完成结果。

未领取状态的有界保留依然是内部资源机制，但不要求调用者在领取以后再释放一个内核结果句柄。

## 权限与职责

Join 只读取调用者有结果项的目标；Hear 只读取调用者直接 owned Team 子树中的结果项。不能凭 TaskId 猜中号码领取结果，也不能把同 Team 的执行控制权自动当成另一 task 的结果消费权。祖先 owner 的结果视图仅授予结果领取，不授予后代执行控制。

Oust 保持原来放下直接子 Team 的契约；Held、未收尾成员或构造提交仍阻止成功。Hear 队列暂空不能证明子树结束，主 task 退出也不能证明所有辅助 task 已退出。

Fall 继续只通知 Accord 到达。Join/Hear 的等待登记、唤醒、固定 deadline 与状态提交共用基础机制，但 Fall 不扩大成通用生命周期通知。

## 用 Scan 统一子域枚举

撤回 Heir/HeirCount 两个入口，使用 Scan 表示批量枚举；Hear 独立表示等待并领取完成结果。Unit::Scan 采用“按稳定 ID 游标，读取当前集合的一页”的形式。Pie::Collect 的名称与契约保持原样。

```rust
Scan {
    after: TeamId,
    buf: VirtAddr,
    capacity: usize,
} -> UnitResult<usize> // written
```

仅枚举调用者自己的直接 heir 表，不收集任意 Team 的成员，不遍历子树，不消费、不转移所有权、不等待退出。保留内部 sire/heir 词汇，替换的是公共查询入口。

按 TeamId 递增写出严格大于 after 的项，返回本次实际写入数。after=0 开始；下一次 after 使用上页最后一个 TeamId，不要求该项仍在 heir 表。written=0 表示本次查询时没有更多；没有独立 total/count 真值，也无需“先计数、再按索引逐项读取”。

例如当前直接子域为 [10, 20, 30]，capacity=2：

```text
Scan(after=0)  → written=2，buf=[10, 20]
Oust(Team 20)
Scan(after=20) → written=1，buf=[30]
```

移除上页游标项仍可继续；不以整数 index 作为稳定身份。分页观察各次调用时的当前集合，不是跨页冻结快照。调用者需要统计时可以数本次写入项或汇总遍历；汇总值不保证是变化期间某一瞬间的精确数量。若将来必须同时获得瞬间总数，再明确增加同一读取提交点派生的返回值，不能恢复一份独立可写 count。

建议每次 1..64 项，用显式 u64 LE 写 TeamId，最多 512 字节；验证容量、地址范围和长度算术，拒绝非法输入，不复制 Rust 容器内存。先在域结构同步保护下将一页 ID 拷入固定内核数组，再在锁外写用户缓冲。写回失败返回错误，不修改 heir 表或消费游标；调用者丢弃错误调用的缓冲内容。一次页快照不延伸为全表冻结，不新增快照句柄或动态分配。

保留单项批量读取的 bounded buffer，不按 HeirCount 的结果申请一份全表空间；减少接口也不能隐藏分页、变化及写回失败的契约。Heir/HeirCount 旧编码应拒绝，确切新编号仍待完整 ABI 评审。

## 返回布局与迁移风险

建议统一三字：a0=0 表示 Pending，a1/a2=0；完成时 a0=cause(1..4)，a1=TaskId，a2=reason。未知 cause/非法 ID 拒绝，错误使用负码。复用已有 ret3，无需五字返回或新结果句柄类型。

这是对原 Join 的显式行为变更：非消费观察变为领取，授权和重复等待也改变。原来用 Join 探测权限主体是否存活的调用者不能直接迁移；需要逐个区分“消费子任务结果”与“检查权柄主体有效性”，后者应由相应身份/能力契约判断，不以一次领取替代存活检查，也不把 Denied 解释成正常退出。

实施前必须核对：现有多等待者/重复 Join、Rust Join<T> 的 Drop、长期线程创建是否需要未完成单项放弃、祖先视图预算与无分配去重、所有存活探测调用方的替代路径。若这些调用方不能接受消费模型，应退回非消费等待设计并正式设计放弃机制，不能强行删除释放入口假装闭合。

本方案解决的是前一稿接口绕行和领取后的记录释放问题；它不声称所有旧调用方已具备可直接迁移的语义。未改实现，也未固定操作号。
