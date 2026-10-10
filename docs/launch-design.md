# Launch 创建与交付

处理 PR #5 在 `programs/src/system/launch/mod.rs` 的“打补丁，不行”评论。

原流程先让 Loader 构建 Task/Team，再向 Launch 加入 `registered = false` 的记录。app 另行调用 register，由 Control 检查实例容量；失败时临时 Slay/Oust。交付和准备 hook 又根据 registered 判断记录是否有效。这样既留下未归 Control 管理的已构建实例，也把组件内部顺序暴露给 app。

## 所有权

* Loader 拥有映像读取、ELF 构建、缓存和构建期间的失败清理。
* Control 唯一拥有实例表、状态、Claim 截止时间与退场进度。
* Launch 拥有请求接入、交付策略和回复。Pending 只有 requests 与 launches 两个队列；它们表示请求处理前后两个阶段，不保存实例状态。

```rust
struct Pending {
    requests: Vec<Request>,
    launches: Vec<Launch>,
}
struct Launch {
    task: TaskId,
    delivery: Delivery,
}
```

Launch 不再保存 built.team 或 registered。task 只是查询键，Team 与实例状态从 Control 取得。Delivery 保留原有 owner、identity、constructor、back；constructor 是授权策略，不是准备完成标记。

## 创建契约

```rust
fn create_instance(
    &mut self,
    owner: TaskId,
    construct: impl FnOnce() -> Result<Built, Fail>,
) -> Result<Built, Fail>
```

Control 在调用 construct 前检查实例上限并预留表容量。construct 成功后立即登记 Starting 实例；返回成功就代表实例已由 Control 管理。reserve/register 是这个入口内部的私有步骤，其他组件不能绕过它们。

Launch 在调用该入口前预留交付队列容量。于是成功构建之后，登记与加入交付队列都不再需要分配。Control 的可变借用覆盖创建过程，不另外引入 reservation、registered 或创建状态镜像。

失败处理由原来的资源拥有者负责：

| 失败点 | 行为 |
| --- | --- |
| 请求队列或交付队列容量不足 | 释放输入 image，回复 Full，不开始构建 |
| Control 实例表容量不足 | 不调用 construct，释放 image 并回复 Full |
| 映像验证或 ELF/Task 构建失败 | Loader 清理未提交的 Unit；Control 不发布实例记录；Launch 释放 image 并回复原失败 |
| 身份、授权或命名空间 Prepare hook 失败 | 已登记实例由 Control 进入 Stopping，再通过原有 Retire hook 回收 |
| 成功交付时回复失败 | Launch 请求 Control 停止实例；不在 Launch 中临时 Slay/Oust |
| 所有者退出或未及时 Claim | 仍由 Control 的既有实例回收流程处理 |

## 调度

app 用 `launch::frame()` 组合创建阶段，不再排序 dispatch、Loader build 和 register：

1. 接入经 Control 授权的 Construction 请求。
2. Loader 服务接收普通 Build/Claim；Build 进入同一 Pending 接口。
3. 停止期拒绝尚未构建的请求。
4. 预留容量、构建并登记。

两种 Build 入口共用 Pending::push 的队列上限和失败回复。Loader 服务不再推进 Launch 构建或保存交付阶段。实例 Prepare/Retire 仍注入 Control 的 hook 子计划；之后 Launch completed 查询 Control 并交付结果。准备期间 Pending 保留 identity/constructor 策略供 hook 使用。

本次保留 Claim 协议和原有截止时间语义，不增加系统调用或公开构造协议。

## 验证

新增生产 create 模块的宿主检查：实例表满时构建器不会执行，构建失败不发布记录，成功返回前已登记正确 owner、Task/Team 和初始状态。既有实例操作、失败 hook 与回收检查继续运行。

整机验证覆盖 Loader 创建、Claim、拒绝越权操作、未领取回收、Preparation/Retirement hook 失败，以及 Shell 失败回滚与管道作业。

实际结果：全目标编译通过；control-instance 13 项、system-shape 18 项通过；system-fault 场景、Shell edges 和完整 smoke 的交互驱动通过，包含 1 MiB 数据长度检查。

完整 smoke 首轮在登录阶段提前退场，交互驱动没有完成；同一代码复跑完成全部交互。两轮退场日志都记录 Router 访问失效 PLIC 映射（VA 0x225004，PC 0x11358），后一次发生在驱动主动结束 Login 之后。这是需要继续核对的退场问题，不能因为 scene 返回成功而忽略。日志分别保存在 /tmp/launch-smoke-console.log 和 /tmp/launch-smoke2-console.log；本次创建流程的检查不声称已修复这项问题。

## 退场顺序修复

后续定位到整体退场按登记顺序同时提交 Ruin。Hub 比 Router 先退出，设备引用的回收会撤掉 Router 的 PLIC 映射；Router 仍然存活并处理通知，读 claim 寄存器时触发页错误。问题出在服务退场顺序，不能用吞掉 Await 错误或增加延时来保证映射存活。

Control 的 closing_service_names 现在直接读取原有 UnitFile.relation.after：仍有已启动的后继服务时，该服务不能进入整体退场队列。后继处于 Stopping 时也继续阻止前驱退场，直到原有 Ruin 的 Oust 步骤完成并将其标为 Dead。互不依赖的服务可以同批停止；不另存反向依赖图或退场游标。scene 目标保留起手规则的反向顺序：等待 scene 的服务先停止，其余服务随后停止；同批等待 scene 的服务不互相阻塞。

整体退场还先等待动态实例的 Team 全部经 Retire hook 回收，再停止静态服务。动态任务可能使用任意静态服务的资源，其依赖没有静态 after 声明，不能与静态提供者同时销毁。停止和回收仍使用既有 Slay、Join、Oust，没有增加系统调用或额外生命周期字段。

这项顺序约束用于整体退场；显式单独请求 Ruin 服务以及服务异常退出，仍遵守原有资源撤销契约。

宿主回归直接编译生产 observe 模块，覆盖与登记顺序无关的 Hub→Router→UART 依赖链、独立服务并行停止、Stopping 和 Debarked 后继仍持有前驱、多个 scene 后继退场。新增 Shell shutdown 驱动在两个作业及共享页仍活着时退出 Shell 和 Login，以检查动态实例回收及系统退场。

修复后验证：kernel/programs 全目标编译、control-query 5 项及 system-shape 18 项通过。QEMU 完整 smoke（171.98 秒）、shutdown（11.72 秒）、system-fault（23.00 秒）通过；两次交互驱动结果均为 ok。smoke 包含 1 MiB 长度检查；shutdown 在 EOF 前确认 spin 与 workers 均处于 running。两次 product 退场均正常打印 internal tasks stopped、system: done 和 all tasks exited，未记录 user fault 或缺失映射。system-fault 保留预期的 Preparation/Retirement hook 注入失败并完成回收。

日志：/tmp/router-check.log、/tmp/router-query.log、/tmp/router-shape.log、/tmp/router-smoke.log、/tmp/router-smoke-console.log、/tmp/router-shutdown.log、/tmp/router-shutdown-console.log、/tmp/router-fault.log、/tmp/router-fault-console.log。交互结果与串口记录在 /tmp/router-smoke-trace 和 /tmp/router-shutdown-trace。
