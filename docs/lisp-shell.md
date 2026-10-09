# sqware Lisp Shell

登录 `product` 镜像，使用示例账户 `anran` / `sqware`，进入 `lisp>`。Ctrl-D 退出 Shell，取消活跃作业后归还终端；Login 再次显示登录提示。Ctrl-C 取消前台作业或当前求值，Ctrl-Z 暂停前台作业。只有 `#f` 为假，整数是检查溢出的 i64。

## 依赖与职责

| 模块 | 职责 | 验证边界 |
|---|---|---|
| `crates/lisp` | Source、Logos lexer、栈式 Reader、Lower、词法绑定、值、GC、Machine、纯原语 | 独立宿主测试和 RISC-V 编译 |
| `crates/stream` | 原子有界字节环、部分进度、EOF、断流 | 安全 Rust 宿主并发测试 |
| `service/pipe` | 根能力、授权与端点寿命 | 模型测试和真实映射 |
| `shell/core` | 命令、显式连接、Plan、Job 状态及成员结果 | 无 IPC／终端的模型测试 |
| `shell/native` | 参数检查、外部对象、语言标准库和可暂停操作 | 生产原语层加模拟平台 |
| `shell/adapt` | 非阻塞 RPC、清单、流适配、真实作业推进 | QEMU |
| `shell/repl` | 会话装配、输入、求值和事件循环 | 真实登录与交互脚本 |

Lisp 库只依赖 `core`、`alloc` 和固定版本 Logos 0.16.1，禁止 unsafe。语法节点是独立 arena，Reader 不创建运行时对象。Lower 将 let 和函数 define 简写转为少量核心表达式；局部引用使用词法层级与槽位，全局保留符号查找。解释器不按列表头猜测语义。

Machine 使用显式续体栈，尾调用替换状态。引用列表、列表构造、变长算术和字符串／字节复制分步推进，复制每步最多 256 字节。闭包持有代码与环境；绑定单元可变，列表和其他语言值不可变。非移动 GC 使用带代数的句柄和显式标记栈。全局、执行状态、暂停参数和宿主 RootValue 都是根。源码随存活代码持有，REPL 不永久保存历史。

## 语言与宿主 API

特殊形式：`quote if begin define lambda let set!`。if 要求三个操作数；define 限顶层；固定形参；let 初始化按从左到右在外层环境求值。错误保留已完成的顶层定义。原语提供检查算术、比较、序对、列表、字符串和字节串操作，统一登记参数数量。

Reader 返回 `Complete { form, consumed }`、`More` 或 `End`。括号／字符串不完整且未 EOF 时为 More，非法转义、错误点对和多余闭括号报错。第一项完整时立即提交，后续不完整项留给下一次 read。

```rust
engine.start(form)?;
match engine.step(1024) {
    Step::Yielded => { /* 推进其他事件 */ }
    Step::Request(call) => { /* 宿主保留请求，完成后 resume(call.id, result) */ }
    Step::Done(value) => { /* value 是受控的 RootValue */ }
    Step::Failed(error) => { /* 带 Source、Span 和调用位置的诊断 */ }
}
```

cancel 丢弃当前求值，迟到的恢复得到 StaleCall。外部对象 GC 只排队释放，宿主执行实际关闭。多个 Lisp 引用别名共享关闭状态，重复关闭幂等；活跃 Job 由作业表持有，回收 Lisp Job 引用不会取消它。

默认 Limits：输入 64 KiB、结构嵌套 1024、非尾调用续体 4096、语言堆 4 MiB。代码、对象和全局名称计入保守资源计费；全局名达到配额时可诊断且 REPL 继续。宿主外部对象载荷、作业计划和累计缓冲分别有 4 MiB 配额，外部对象上限 4096、Job 上限 128。

## 显式管道

程序自行声明端口名和方向，所有端口必须显式连接。没有固定三口或默认终端绑定。首版拒绝循环、重复端点和缺失绑定。字符串参数必须是字符串列表，端口是外部对象。

```lisp
(define sink (buffer 'write))
(define a (command 'emit '("hello")))
(define b (command 'upper '()))
(define plan
  (connect (list a b)
    (list
      (list (list a 'records) (list b 'source))
      (list (list b 'result) sink))))
(run plan)      ; (completed (0 0))
(bytes sink)   ; #u8(72 69 76 76 79 10)
```

示例镜像能力由 Account 提供并保留：cat 使用 source/copy，emit 使用 records，upper 使用 source/result，workers 使用 ticks；fail 和 spin 无端口。子程序继承用户 Principal。增加命令需要系统装配镜像能力并登记命令的端口描述。

| 原语 | 用法 |
|---|---|
| buffer | `(buffer 'read #u8(...))` 或 `(buffer 'write)` |
| terminal | `(terminal 'read)` 或 `(terminal 'write)`，随后显式连接 |
| read | `(read port [count [milliseconds]])` → `(data bytes)`／`(eof)`／`(pending)` |
| write | `(write port bytes)` → 字节进度；字符串用 string->bytes 显式编码 |
| wait | `(wait port-or-job [milliseconds])` → 就绪布尔值／Job 状态／pending |
| close / bytes | 幂等关闭／内存缓冲快照 |
| prepare / start | 所有实例暂停、认领、授权、清单完成后才放行 |
| pause / resume / cancel | 等待真实转换确认；超时不伪造完成 |
| foreground / background | 分配／归还 Shell 内部终端归属 |
| jobs / status | Job 引用列表／状态及全部成员结果 |

语言标准库提供 run、spawn、fg、bg。后台读终端会暂停整个作业，wait 返回 `(paused (...) background-read)`。fg 恢复并等待，bg 恢复但不占终端。后台输出按作业公平交错，终端保留 UART 背压下的待发送帧。UART RX 满时保留一批并等待 Rack 的空间通知，交付后才归还 IRQ，长命令不会被队列覆盖。

```lisp
(define j (spawn (connect (list (command 'spin '())) '())))
(pause j)
(status j)     ; (paused (#f) requested)
(bg j)
(cancel j)
(run (connect (list (command 'fail '("17"))) '())) ; (completed (17))
```

单个成员失败只关闭该成员端点，其他成员继续；Job 汇总所有结果。未启动成员取消记为 not-started。准备失败回收已知实例和授权；丢失 Build 回复由未认领期限回收。放行失败停止已放行成员，已发生的程序行为无法回滚。

## Pipe 与内核控制

Pipe 数据面是一枚共享 Pole，默认载荷 16 KiB，可指定 1..1 MiB，映射按页取整。双方直接访问原子游标、关闭标志和字节载荷，服务不转发普通读写。协议是协作式单读者／单写者，端点角色由服务独占登记；共享映射不提供恶意程序间的读写隔离。

Pole 的 ring/hush 使用非零 Bits 掩码，wait 使用 Signal(Bit)。等待键是 PoleId+Bit，Pile 可订阅同页多位，销毁唤醒全部位。现有 Pole 通知迁到第 0 位；Hole／Nole 仍单铃。Pipe 第 0 位通知读者，第 1 位通知写者，第 2 位报告读请求以实现后台终端读暂停。发布状态再响铃，等待按检查→清本方向位→复查→等待，部分进度优先返回。写者关闭后排空再 EOF，读者关闭后写者断流。

Pipe 服务持有根、创建者租约和每端独占 Tole 寿命凭证；Tole 不承载字节。当接收任务退出或释放端点，凭证回到服务，服务关闭对应端并扫除无主资源。Shell 取消准备时显式 release，以回收回复尚未被导入的授权。

Control 暂停 Team 子树，暂停门覆盖已存在任务、新任务与唤醒路径，保留单任务停止和阻塞状态。所有运行任务离开调度槽且分支版本稳定才确认暂停。创建时预留结果；主任务及辅助／子 Team 的失败汇入实例结果，退场 hooks 完成后才报告 Dead。owner 死亡复用实例回收。

托管应用由 Job 判定结果。System 对拥有的应用 Team 显式 Observe；内核仍记录诊断，整机失败账单独保存未托管失败，应用非零退出不能覆盖系统故障记录。

## 验证

```sh
cargo test -p lisp -p stream -p env --target x86_64-unknown-linux-gnu
cargo test --manifest-path programs/tests/shell/Cargo.toml --target x86_64-unknown-linux-gnu
cargo check -p kernel -p programs --all-targets
nu scripts/qtest.nu --package kernel --scene accept
nu scripts/qtest.nu --package kernel --scene system-fault
nu scripts/qtest.nu --package kernel --scene product --feed-script programs/tests/terminal/session.py
nu scripts/qtest.nu --package kernel --scene product --feed-script programs/tests/shell/smoke.py
nu scripts/qtest.nu --package kernel --scene product --feed-script programs/tests/shell/edges.py
```

Shell 模拟宿主测试执行生产原语层，验证参数错误、别名关闭、Plan 根、等待时控制推进、取消准备和错误恢复；清单测试逐字截断并检查版本、尾随数据及 UTF-8。语言测试覆盖十万次尾调用、捕获环回收、暂停根、迟到回复、长 UTF-8 字符串、长操作中 GC／取消及资源限额。

真实 Shell 脚本覆盖 emit→upper→buffer、1 MiB 跨域环绕与背压、非零退出不判坏整机、辅助任务暂停恢复、后台读暂停、fg/bg、Ctrl-C/Ctrl-Z 和无限求值取消。终端脚本覆盖密码编辑与不回显、重复登录的不同 Task／同 Principal 和终端归还。accept 保留现有 IPC、通知与装配探针；system-fault 覆盖实例准备失败、认领超时、owner 死亡和子树回收。

内核健康面运行 `nu scripts/qtest.nu --package kernel`；不提供镜像时 scene 哨兵预期失败，健康用例另行检查。新增 branch_control_and_results 验证已有和新建子 Team 继承暂停门、独立子树暂停及失败向托管根传播。

首版不含宏、字节码、数值塔、可变序对、循环管道和方言兼容。

## 本轮验收记录

2026-10-10：35 个独立宿主套件 322 项通过，Lisp 18、Stream 3、Env 6、mold 11 项通过，合计 360 项；mold 既有 6 个 ignored 示例不计入。kernel／programs 全目标、独立 Lisp RISC-V 目标和 diff 检查通过。

最终内核健康面 36 项通过，无镜像 scene 哨兵按设计失败。QEMU release：accept 5.58 秒、system-fault 11.80 秒、完整登录 3.38 秒、Shell 完整脚本 34.25 秒、边界脚本 7.09 秒通过。边界脚本还重复通过一次，6.99 秒，最后一次内核结果归属修改后再次通过，7.09 秒。重复结果描述此次观察，不保证所有运行条件下均无时序问题。

完整脚本验证 1 MiB 管道和作业控制；边界脚本验证连续长 UTF-8 输入、真实清单准备失败回滚、成员 17/0 独立结果、20000 字节终端输出和断流退出 1。UART RX 覆盖竞态修复后这些脚本通过，Rack 的保留发送／空间通知也由 accept 中的真实探针检查。

Pole Mail ABI 和 Control 回包布局已改变，内核与镜像必须一并重编译。
