# Pole / Tole 与 Mail 统一设计

状态：已按评审确认的方案实施，并完成宿主、内核与整机回归。内核与用户镜像须一起重编译。

## 范围与约束

处理 PR #5 的 Rack 通知设计，并一起审视 Pole 共享页、Tole 等待组、Pipe 端点寿命。Launch 的创建流程重整另行设计。

保留 Pie、Mail、Pole、Tole、Ring、Hush、Wait、Attach、Detach、Await、Join、Inspect 等原有词汇。删除公开的 CapabilitiesChanged，不通过换名字保留能力表变化广播。Tole 不再拥有独立调用域；其创建、权限与寿命归 Pie，多路等待归 Mail。

避免两处事实：共享环决定是否可读写；通知位只要求复核；TaskState 决定退出；能力树决定引用存活和独占移交关系；Tole 只保存登记关系和等待所需的进度。

## 迁移前实现的问题

1. Pole 保存带锁的 usize 通知位图。Ring/Hush 接受 Bits，Wait 只接受 Signal(Bit)。Hole/Nole 只接受第一位。
2. Tole.cells 按资源身份保存登记，Await 在调用者能力表中重新寻找一个引用。因此返回的 token 未必是原登记的 token。
3. Detach 要重新取得目标引用的权限；引用失效后可能无法撤销登记。
4. Tole.subs 保存任务完成和能力变化。Await 的 NONE 同时代表挂起后的返回、超时、状态通知或普通复核提示。
5. Tole.cells() 每次扫描复制并分配；分配失败会伪装成空组。
6. Pipe 每个方向创建一个空独占 Tole，授给端点任务，再通过服务端祖先引用的 HandedOver 判断该端是否仍存在。共享 Pole 仍活着不能替代这项判断。
7. Terminal 把 authority 独占授回客户端；客户端释放后需要唤醒 Terminal，即使 authority 资源本身仍活着。只通知 Seal/资源销毁不能覆盖这一流程。
8. System 等待组包含 TaskCompleted 和自身能力变化。移除广播前必须明确现有每个复核需求，尤其是引用到达、撤销、缩权和独占归还，不能把等待偷偷改成周期轮询。
9. Join 的 Task/Team 观察和领取已有权限与状态契约，多路等待必须复用它，不另存退出结果。Team 观察表示有可观察的成员结果，不应改成整个子树全部完成。

## 调用归属

| 调用域 | 保留/迁入的调用 | 责任 |
| --- | --- | --- |
| PieCall | Unseal、Open/Shut、Seal、Accord、Narrow、Revoke、Release、Inspect、Collect、Same | 资源、映射及能力树 |
| MailCall | Push/Pull、Peek、Ring/Hush、Wait、Attach/Detach、Await | 消息、通知和等待 |
| UnitCall | Join 等现有调用 | 任务/Team 结果的权威观察与领取 |

删除 ToleCall。Subscribe/Unsubscribe 的现有用途通过 Attach/Detach 表达，不保留第二套登记表。迁移时重编译内核与全部镜像，旧 ABI 明确拒绝，不让旧调用误落到新操作。

## Pole 与单通知操作

```rust
Ring { token: PieToken, bit: Bit }
Hush { token: PieToken, bit: Bit }
Wait { token: PieToken, condition: MailCondition, millis: Wait }
```

Pole 的 Ring 设置指定的一位；Hush 清除指定的一位。重复设置或清除幂等成功。未知位索引解码失败，不能静默截断。删除公开 Bits；内部位图仍可以是 usize，不将 API 参数类型与存储表示绑定。

Wait 不清位，返回的就绪必须由调用者复核。Hole 的 Pull/Push/Empty 和 Nole 的 Pull 保留；Pole 使用 Signal(bit)。Hole/Nole 的 Ring/Hush 仅接受 FIRST。Hole 保留累积事件、一次 Hush 应答一枚的既有语义，队列满或没有待应事件仍返回 Busy；Nole 保留中断应答的既有 Busy 契约。统一 Bit 参数不统一这些资源的事件计数语义。这种限制需要写在契约、类型封装和非法组合测试中，不能仅靠调用时 Denied 暗示。

保留现有权限族：Ring 要 STORE，Hush/Signal 等待要 FETCH；Open 的读写映射继续由 FETCH/STORE 决定。本轮不新增逐 bit 权限、不修改共享页的信任边界：获得 FETCH 的持有者能清该 Pole 的任意合法通知位，协议必须依赖可信的双方，不能把不同 bit 当成权限隔离。

页数据和通知不复制状态。接收方遵循“检查数据、清通知、再次检查数据、等待”；生产方先发布数据再 Ring。内核不会用通知位判断环容量、EOF 或 broken pipe。

## 统一等待来源：逻辑接口提案

使用原有 Source 类型，以现有操作词汇描述登记对象：

```rust
enum Source {
    Mail { pie: PieToken, condition: MailCondition },
    Join { target: UnitTarget },
    Inspect { task: TaskId, token: PieToken },
}

Attach { tole: PieToken, source: Source }
Detach { tole: PieToken, source: Source }
Await { tole: PieToken, millis: Wait }
```

没有 CapabilitiesChanged，也没有任意任务能力表变化订阅。Mail 来源统一等待已有资源条件；Join 来源只观察；Inspect 来源限定到具体引用。Inspect 来源按评审确认的直接授予关系授权，属于权限契约扩展。

### Mail 来源

Attach 需要组 STORE，并校验来源在调用者表中可用、满足对应 FETCH/STORE。保存原始 token、条件、登记者及稳定能力身份，不能在 Await 中按相同资源挑一个别名替代。两个别名是两条登记，撤销其中一个不影响另一个。

资源条件按实际资源状态复核。引用被撤销、失去所需权限、发生独占移交或资源封印时，同一来源报告现有 Denied/HandedOver/Dead 原因。报告后不自动撤销；调用者 Detach，或等原引用恢复。

### Join 来源

复用 Join(target, receive=false) 的授权与观察判据，不领取 TaskExit，不复制 cause/reason。任务登记引用共享 Member/TaskState 元数据，不强持有 Task 执行躯壳或 Space。多路等待不扩大可观察任务范围，不能替 Pipe 服务直接观察任意无关任务。

Team 来源必须跟随现有可领取成员记录；没有成员结果和整个 Team 终止不是同一个条件。消费者通过 Join 领取结果，领取后重新评估来源是否就绪。Oust 后报告失效，不保持已删除 Team 的观察权。

### Inspect 来源与端点寿命

本来源只用于两个精确关系：

* task 为登记者自己：观察自身持有的指定引用何时不可用，或独占祖先引用何时恢复可用。
* task 为 Accord 的接收者：登记者必须仍持有该引用的直接授予祖先，并证明指定 (task, token) 正是其直接子引用。只观察这个子引用的终止/失效，不暴露目标任务的能力表或其他引用。

第二种关系是权限设计的实质扩展，应与 Accord/Revoke 的现有直接关系核对，不作为任意跨任务 Inspect。不得以知道 TaskId/PieToken 代替授权证明。目标失效后仍保留登记描述以便 Detach；登记本身不能延长目标能力、任务或空间寿命。

本轮限制到协议所需的状态等待，不广播每次能力操作：

* 对独占祖先，Attach 时确认处于 HandedOver，等待它恢复可用或彻底失效；不是一登记就把当前 HandedOver 连续报出来。
* 对直接子引用，等待该引用失效；只要任务仍活着但该端主动释放，照样可报告。
* 对自身可用引用，等待其失效；单纯有别的能力到达不触发。

登记保存的初始观察及已经交付的进度属于等待进度，不是另一份能力存活状态。当前事实始终从能力树/资源状态读取。反复移交必须重新登记，不能把一次端点等待自动扩展为永久能力变化监视。

Attach 必须在验证、安装唤醒边和复核之间关闭竞争窗口；失效/归还发生在登记时也不能漏报。引用 token 由内核全局单调编号分配，释放后不回收或复用；登记同时绑定具体任务和直接授予关系。编号耗尽时必须停止分配，不能回绕到旧身份。

## Await 返回和挂起恢复

AwaitReply 返回 Pending 或 Source { source, fail: Option<MailFail> }，区分三种含义：

| 结果 | 意义 |
| --- | --- |
| 有 Source，来源成功 | 指定 Mail/Join 就绪，或指定 Inspect 等待关系已满足；调用者仍复核 |
| 有 Source，来源失败 | 该登记已经 Denied/Dead/HandedOver；可按同描述 Detach |
| 无 Source | 截止时间已到且没有可返回来源；POLL 则是当前无命中 |

组本身不可用作为 Await 调用失败，与某一个来源失效分开。不得用组级 Dead 混淆来源级 Dead。来源的错误不得擅自合成 Join 成功结果。

ABI 使用现有 ret3：a0 低八位为 Source 判别，上位为来源失败码的绝对值；a1/a2 是来源载荷。Pending 为三个零；组级失败仍是负 a0。Mail slots 使用 32–40，拒绝旧的掩码参数调用和旧 class 9，避免旧 Bit 掩码被当作索引。

内核为 Await 保存唯一绝对 deadline 及组观察身份；唤醒后恢复执行前重新扫描。普通/伪唤醒后无命中且未到期，继续等待原 deadline，不能当作 NONE 返回。实现复用 Join 的等待恢复思路，但组内来源检查与能力锁顺序需单独核对。

就绪是提示，其他任务在返回后可能消费数据；结果不保证下一次操作一定成功。公平游标只决定扫描起点，不复制成员状态。空组 FOREVER 可以等待未来 Attach 或组封印；关闭和 Detach 都需通知正在等待的调用，恢复后复核。

共享 Tole 的登记权限仍由 STORE 控制；Await 只能返回当前调用者有权观察的来源。同一组转授不转授来源权限，不能借共享组跨任务读取原登记者的引用。共享组仅扫描当前登记者的来源；非空组里没有当前调用者的登记时返回 Denied，不能默默跳过后永远睡眠。空组仍可等待后续登记。

## 内部字段与同步

Tole 保留 state、id、life、owner、cursor；用一份登记表替代 cells/subs。登记保存描述、授权身份和必要等待进度，不保存 ready/completed/alive 镜像。

扫描不复制整张表、不临时分配。登记容量有明确上限，容量/分配失败由 Attach 返回，不能在 Await 中把失败表现成空组。检查时遵守 gate 与 Unit 提交锁的既有次序，不持 Tole 表锁进入冲突的能力操作或调度切换。

唤醒转发边由登记描述派生，Attach 安装失败完整回滚，Detach 与清理恰好一次移除。现有同一 WakeKey 多登记/别名共享转发边的情况必须有引用计数或由整表判断；不能摘掉一个别名便切断另一个的唤醒。转发边只承担等待关联，不成为第二份成员事实。

Detach 只校验组 STORE 和原登记描述，不再次要求来源仍存在。组 Seal/Release 清理所有边并唤醒等待者；来源寿命结束保留可报告的失效描述，不能直接过滤掉登记。

## Rack、Terminal、Pipe 的迁移流程

### Rack

Bell 绑定 (Pole token, Bit)，统一 ring/hush/wait；读写两种通知由 Rack 私有分配。Writer 的背压发送、等待及不再有保留帧时的应铃统一封装，Reader 不向调用者暴露写端通知编号。UART 和 Terminal 的等待组通过封装取得各自的等待描述，不直接引用 SPACE_BIT。

Writer 等待的是读端推进提示，环的 capacity/depth 才决定能否发送。普通丢弃模式仍保留，背压发送不得调用会丢弃数据的路径。覆盖清通知与对端推进交错，以及取消保留帧后的残留通知。

### Terminal

将自身能力变化订阅替换为具体 authority 独占归还等待，以及已有 console/endpoints 的精确失效等待。前台变更更新对应登记，失败回滚时摘除登记并撤销已授出的能力。不能靠任意能力变化唤醒偶然推动 UART TX；TX 背压等待必须显式接入 Writer。

### Pipe

只有在上述直接子引用等待契约成立时才删除空 Tole。每个方向 Grant 保存既有目标 TaskId 和数据 seed，服务端通过它自己创建的 (target, seed) 直接子关系登记失效，不再生成 life/watch Tole。

* BIND：预留 Grant 与登记容量，Accord 数据引用，登记具体子引用，提交 Ledger；任一步失败撤销新引用并回滚登记。
* 端点 Release 或任务退出：子引用失效，服务端关闭对应方向，更新共享 header，并 Ring 对端的既有通知。
* CLOSE：关闭方向并通知对端；保持与释放数据引用的区别，不提前破坏仍需读尽的已写数据。
* RELEASE/owner 退出：关闭双方、撤销数据引用、摘除全部登记，释放根页。
* 客户端 Port Drop：关闭自身方向，释放数据 seed；不再释放 life。
* 所有者现有 Hole lease 另有明确的创建/释放协议，本轮不因为删端点空 Tole 就擅自删除它；将其作为具体引用登记，不扫描整个能力表。

Pipe 服务需要把原请求接收和端点失效放进同一 Tole，而非依赖每 10ms sweep。端点已关闭且对方仍持引用时不提前释放根页；双方引用已失效时按现有释放策略收尾。header 的 closed 是流协议事实，能力树的引用存在是授权事实，两者不能互相替代。

Reply/Endpoint 删除 life 属于协议变更，须同步迁移 Shell 的只读启动清单、端点导入、失败回滚和所有测试，不能只改服务端结构体。

## 实施步骤与验收

1. 评审 Source 的三类对象、具体引用观察权限和 Await 返回布局；这些契约冻结前不改实现。
2. 将 Tole 操作迁入 Mail；统一单 Bit Ring/Hush，删除 Bits 和 CapabilitiesChanged 的公开 ABI，同步更新拒绝旧调用的测试。
3. 建立统一登记表、精确引用唤醒、失效可摘除及 Await 恢复复核；移除扫描分配和无命中伪装。
4. 迁移 Pile/System/Terminal；核对 System 的每一项等待需求，替换实际引用变化，能力到达继续由现有协议/Fall 处理，不能改变 Fall 为能力变化广播。
5. 迁移 Rack/UART/Terminal 背压；核验通知的方向、清除和等待对偶。
6. 最后迁移 Pipe 直接子引用寿命、Reply/Endpoint 和 Shell 清单，去掉端点空 Tole 及周期 sweep。
7. 编译全部内核/镜像，运行实际生产模块的宿主检查及整机交互。

验收必须包括：重复 Ring/Hush；同时登记多个 bit；清除与发布交错；未到 deadline 的普通唤醒；真实超时；撤销/缩权/移交后 Detach；别名登记与摘除；直接子引用提前 Release 和 Task 退出；独占 authority 归还；注册中发生失效；已完成 Task 与领取后的 Team 来源；组封印、来源封印、共享组权限；一端消失而页仍存活；EOF 读尽与 broken pipe；Shell 失败回滚、前后台与 UART 背压。

## 已确认的设计边界

* Inspect 允许观察自身或直接授出的具体引用，沿用 Accord/Revoke 的关系证明。
* Await 使用三寄存器结果，同时携带来源与来源状态。
* 共享组按登记者隔离并逐调用复核原 token，不按资源查任意别名。

以上三项是明确的设计边界，不是实现遇到困难时可任意补调用的余地。

## 验证记录

* 全目标检查：kernel / programs 的 all-targets 编译通过。
* 宿主检查：ABI 10 项、gate 30 项、resource 13 项、Shell 13 项、连接等待 6 项、System 结构 18 项通过。
* 内核健康检查：38 项通过。无镜像运行时整机 scene 因缺少 initrd 返回失败；带镜像的 scene 单独验证。
* system-fault 整机场景通过，包括真实挂起下 Attach/Detach 普通唤醒不缩短 Await 截止时间、同页多位、重复 Pole Ring/Hush 和来源失效后 Detach。
* Shell edges 和完整 smoke 的场景及交互驱动均通过；完整 smoke 保留 1 MiB 数据检查，仅在临时驱动中延长 QEMU 环境下的传输等待时间。

回归中发现并修复了登记表锁与 messenger 站点锁同级嵌套的问题。并行运行两个 QEMU 时，system-fault 曾出现一次静态单元启动失败；随后单独运行通过，未放宽产品启动契约或断言。

Hole/Nole 的 FIRST 调用保留原有 Busy 契约，避免破坏事件应答与中断路径；Pole 才使用幂等通知位语义。Terminal 原有空 Tole authority 仍承担独占授权，按具体引用等待归还；删除的是 Pipe 每端额外创建的寿命 Tole。
