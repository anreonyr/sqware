# System 的职责与编排

`runtime::schedule` 提供资源注入、依赖图构建、顺序编排和可恢复执行。
协议定义不依赖这些执行机制。生命周期阶段由程序定义，不在通用调度层中列举。

`system/{identity,operator,loader,control}/serve` 处理本域请求。
Identity 的安装、发现和查询位于 `identity/client`；Operator 的私有通道操作位于
`operator/client`。公开的 Placement 和 revision 类型不依赖服务端实现。

`system/run` 是监督器：拥有启动与关闭流程、发布、账户、资源安装、身份别名、
客户端连接集合和事件等待策略。Control 保留任务及实例生命周期；监督器拥有
整机的 Flow、等待期限和回收策略。请求服务端不导入 `run`。

## 使用 schedule

顺序流程使用 `Schedule::sequence()`：`system(name, f)`、`plan(name, child)`、
`subplans(name, select, children, finish)` 按声明顺序执行，不填写数字阶段。
需要局部依赖图时使用 `Schedule::new()`、`add_system`、`add_plan` 和 `before`。
同阶段无依赖节点以注册顺序排序；名称用于标识节点，改名不改变顺序。

`Plan::advance` 在 Pending 或失败处保留 Cursor；完成的步骤不会重复执行。
一份 Plan 可以服务多个 invocation，每个 invocation 分别拥有 Cursor。
更换计划或开始下一轮时显式 reset；固定子计划的恢复位置也属于该 Cursor。
prepare 缓存资源位置，实际执行时仍核对类型，因此资源登记顺序变化时会重新绑定。

子计划分发通过 `Dispatch::begin` 设置本轮预算，`select` 提交 invocation，
`take_result` 一次消费 invocation 和结果。已完成的队列项使用 skip 占用预算；
提前结束使用 stop。框架每次推进一个子计划后占用一个预算，并执行 finish。
Pending 子计划由业务重新入队，下一轮从自己的 Cursor 恢复。finish 自身 Pending
时先恢复 finish，不重复选择或推进子计划。未消费结果的 finish 属于分发协议错误。

## Mark 声明

每个协议域的 `marks.rs` 用 `marks!` 同时生成常量和带名称的声明清单。
Grant 的表声明也生成同样的清单。协议根层汇总所有域并做编译期碰撞检查；
`common/marks.rs` 只定义声明和检查工具。已有帧模块继续重导出原常量名与原值。

## 验证

- `cargo check -p programs --all-targets --offline`
- `cargo test --manifest-path crates/runtime/src/schedule/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline`
- `cargo test --manifest-path crates/protocol/src/common/marks/tests/Cargo.toml --target x86_64-unknown-linux-gnu --offline`
- `cargo test --manifest-path programs/tests/system-shape/Cargo.toml --target x86_64-unknown-linux-gnu --offline`
- `nu scripts/qtest.nu --package kernel --scene accept`
- `nu scripts/qtest.nu --package kernel --scene product`
- `nu scripts/qtest.nu --package kernel --scene system-fault`

宿主测试覆盖编排顺序、恢复、公平性、补偿、借用、资源位置重新绑定、mark 碰撞
和模块依赖边界。整机 accept 验证真实 IPC、生命周期、发布及关闭行为。
