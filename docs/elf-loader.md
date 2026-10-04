# ELF 装载与 backing 实现

实施基线为 `5df4ed5`，保留此前计划的原始基线 `dcc6e33` 与审查基线 `15fc47e` 作为差异说明。Hub 的批量 Activate 协议没有回退。

## ABI

调用变体编号及既有错误码不变；以下参数变更要求内核、env、runtime 与程序同步构建。

| 调用 | 参数与结果 |
|---|---|
| Build | `kind → TeamId`，持有 Build authority 后创建空 Constructing Team |
| Spawn | `team, entry, args, count, stack → TaskId`，返回 Held Task；首次调用提交构造 |
| Mmap | `team, at, size, backing, offset, flags → VA`，六个标量按此顺序打包 |
| Munmap | `team, addr, size → ()` |
| Mprotect | `team, addr, size, flags → ()` |
| UnsealPole | `size, shared → PieToken`，shared=false 添加 ONLY |

Memory 的 team=0 为当前域；非零必须是调用者自己的 Constructing 子域。at=0 自动选址，backing=NONE 为匿名页。size 非零，size/at/offset 页对齐；公开权限使用 R=2、RW=6、RX=10，拒绝其他 PTE 位、无 R 及 W+X。Allocate/Deallocate、Accord/Hatch/Oust 的参数与编号保持。

`MemoryFail::Busy=-7`、`PieFail::Busy=-6` 沿用原码；构造操作门冲突返回 Busy。`UnitFail::BadEntry=-5` 追加在末尾，BadImage=-4 保留供用户态 loader 使用。无效 capability/目标及非法权限返回 Denied，Mprotect 超出上限返回 WidenDenied，准备或容量预留失败返回 OoM。U 调用者请求 Supervisor Build 返回 Denied。

## 装载路径

`crates/loader` 提供无内核依赖的 ELF64 RISC-V 静态镜像解析、页计划和 capsule 编码。宿主与用户态共享页化及入口校验：入口先限制在原文件的 executable 内容内，再补零。RX 的整个区域都成为 payload；R/RW 的纯零尾保持对应权限，使用 lazy-zero。

宿主保留普通 ELF 清单，另外生成引导 capsule。内核启动只安装 capsule；普通 Build 只创建空的 Constructing Team，内核 ELF parser/loader 已移除。`system::loader::Unit` 持有构造和私有源枚的清理责任，首次 Spawn 成功后解除守卫。服务启动及 group 场景已迁移到此路径。

`Control` 持有 `system::loader::Loader`，通过 `build(Image)` 构造 `Unit`，再调用 `Unit::spawn` 创建 Held Task。只读缓存由 Loader 实例持有，以权限及实际补零 payload 匹配，忽略 VA 和文件名；同一实例的重复装载复用 backing。Loader 析构时释放缓存映射与源枚，已安装程序 Map 继续持有 backing。Loader 与 Unit 不跨任务传递，runtime 不再包含装载执行或退出清理钩子。

## 运行时程序

`/svc/sys/loader/build` 是 operator 上的一面独立入口，使用既有 `table!` 和 `Frame` 宏定义。入口默认只授予 control 身份；持有入口副本的请求者可以提交自己的 ELF Pole，不需要清单名字或 Build authority。外部请求只能构造 User 程序，内部 `Loader::build(Image)` 保留 Supervisor 装载能力。

```rust
let loader = protocol::system::loader::Face::of(entry)?;
let built = loader.build(&image, offset, len, &args, stack, wait)?;
control.instance(built.task).embark(wait)?;
```

`control` 必须取自对应的 embark 面；state、debark、ruin 同样使用各自的面。现有按名字管理服务的协议继续使用原报文；新增的实例报文以 TaskId 定位。system control 是实例的内核父方，实例记录绑定请求者；其他请求者即使知道 TaskId 也不能操作或查询它。

服务端校验转授来源、镜像范围和参数数量，通过现有 Mail Push/Pull 的内核复制取得自有快照，随后解析和装载。复制持有映射锁；调用方撤销页映射会使复制失败，不会让服务端直接解引用失效地址。调用方后续修改镜像不会改变已构造程序。参数最多 64 个，只搬运标量；镜像上限 16 MiB。

成功返回 `Built { team, task }`，Task 仍 Held，协议状态为 Debarked。客户端在同一 build 入口确认领取，并等待领取回复；未确认结果在 3 秒后回收，回复失败直接进入回收。请求者退出会清理它的全部实例。Embark、Debark、Ruin 和 State 通过 control 实例接口完成；Ruin 等待 Team 清理完成才回复。身份继承请求者，运行时资源接入现有登记和退休路径。记录最多 256 个，满时先淘汰已回收记录，否则返回 Full；被淘汰的旧 TaskId 查询返回 Unknown。

只读缓存预算为 256 页。FIFO 淘汰释放源 token 与本地映射；已经安装的程序 Map 独立持有 backing。超过预算的单段不入缓存，但仍可成功装载。可写段和 BSS 不共享；文件名、目标 VA 与共享库名字均不参与匹配。共享库的只读页复用遵循同一规则；动态链接、符号解析和重定位尚未实现。

## 授权与资源

启动资源统一使用 `Name::{Trap, Call, Page}` 与 `Entry { name, kind, token }`。内核按 `register → freeze → grant` 安装初始任务的资源，再发布只读目录。`Call(Build)` 对应内核保存的 NoleMeta；Build 检查调用任务是否实际持有其 FETCH 权限，普通 Nole 与相同标记均不能替代。system 持有 FETCH | VEST，可用既有 Accord 派发 FETCH；收方不带 VEST 时不能再派发。

- 未增加 Freeze、Frozen、Write、BuilderToken、Finish 或 Abort。
- `UnsealPole(size, shared)` 保留自动 Open；shared=false 在创建时提供 ONLY。初始化统一使用 UnsealPole → Shut → 当前域 RW Mmap → 写入 → Munmap。
- 共享程序页通过 Narrow 原地撤 STORE。实际权限表中的 STORE holder、可恢复 W 的映射上限及在途操作都进入 backing 登记；临时 Pie clone 不新增 holder，且看到同一份权限收窄/失效。
- Map 持有 backing Arc、offset、授权上限和来源关系；部分保护、裁剪与拆分保留这些信息。FETCH-only 的映射不能经 Mprotect 得到 W；私有 R 安装也保留 R 上限。匿名 Mmap 禁止 X，Allocate 保持原有契约。
- ONLY 私有根必须由创建者持有、无其他 holder 或本地视图，且整段暂存到一个 Constructing 子域。预留中访问/转授被阻止，Narrow 可同步收窄目标。取消先拆映射并完成 shootdown，再解除预留；源权限不会恢复。
- 首次 Spawn 原子消费全部私有根枚。之后旧 token 已离开权限表；私有 Map 由目标 Space 独立持有。共享程序 Map 同样独立于源 token 寿命。
- 本地 Open/Mmap 视图仍由 token 关联并受 Narrow、Shut、Release、Revoke 清理。撤映后的 backing 引用与别名登记经 Salvage 保留到 shootdown 完成；虚拟区间通过内部退休票据继续占位，完成同步后才释放，防止提前复用及地址 ABA。

## 首次 Spawn 发布

内部 PreparedTask 拥有栈、trap frame 和预分配任务对象，不进入名册或 conductor 计数。任何准备失败只释放准备资源。

Team 容器容量在发布前准备。全局名册锁内执行可失败预留，并保持锁直到插入完成；其他 Spawn 无法消耗这次容量。提交消费私有根、设置默认入口与 Ready、安装 Team tasks/held、登记一次 conductor 计数，再插入名册。待析构的源对象转移到已预留的容器，解锁后清理。debug 发布守卫在分配器入口和 ASID shootdown 入口检查提交段禁分配、禁等待。

构造操作门与 backing 操作门是持有 Arc 的原子门，冲突直接返回 Busy，不持 SpinGuard 等待 shootdown。首次发布依次持 staging、GRAPH、caller pies、Team tasks、Team held、roster；后续发布只需最后三个容器锁。锁等级为 TeamTasks=3、held/L3=4、Roster=5，随后分配器等级递增。GRAPH 只串行化权限树的最终核验与变更；页表回收和降权同步在 GRAPH 外进行。任务对 muster 可见的线性化点是最后的 roster 插入。

只有首次 Spawn 能关闭 Constructing 状态；内部 hold 仅接受 Ready Team。后续 Spawn 验证 executable 入口，不重新开放构造。Oust 和父方退出覆盖没有 Task 的构造对象。

指令发布执行写屏障、设置各 hart 的待同步标记、当前 hart fence.i 与活跃 hart 的远程 fence.i。返回任务时只在标记非零时执行 fence.i，覆盖发布时未活跃的 hart。发布端先设置标记再经屏障读取 lease；恢复端先登记 lease 再经屏障读取标记，避免双方同时漏掉对方。恢复端以 acquire 原子交换清除标记后执行 fence.i；并发的新发布仍可留下后续同步请求。PerHart 继续保持 64 字节，没有用户可见 ABI 变更。

Operator 的 Tree::connect 每轮只枚举一次权限表，为各客户端保留最后一枚 owner/mark 匹配的 LINK。新建或替换连接前仍用 current_request 重新扫描、核验当前最新枚与 Reserve；能力撤销、失效及 LINK 替换的复核保持。

另修正 frame allocator 初始化：先为分配器自身保留存储，再计算可分配帧边界，防止该对象跨入第一帧后被后续帧分配覆盖。

## 验证

- `cargo check -p kernel -p programs`：通过。
- `cargo check -p kernel -p programs --release`：通过；两个未使用项警告仍保留。
- `cargo test -p loader --target x86_64-unknown-linux-gnu`：3 个测试通过，包含原始入口边界、RX 补零 payload、R/RW 零尾、权限及畸形布局。
- `nu scripts/qtest.nu --package kernel -- --timeout 15`：20 个健康用例通过；未提供 initrd 的 scene 按脚本契约失败。
- 健康用例覆盖同 PA 共享与最后 Arc 回收、FETCH-only 分拆后拒绝 RW、实际 holder 与旧快照权限、四个准备资源边界的失败注入、名册预留失败、不改变默认入口/Ready/计数、多个私有根一起消费、暂存 Narrow 与取消、待回收别名登记及虚拟地址退休票据的 ABA 防护。
- `health::backing::sharing` 直接比较两个 Space 的 translate PA；共享三页 backing 的 PA 与拆分偏移一致。两份私有 backing 的 PA 不同，向 A 写入 0x5a 后 B 保持零；逐个销毁 Space 后用 Weak 确认各自最后引用回收。
- accept、product、again、load、group、beat、rig：七个 release 整机场景通过。
- accept 随后追加三轮复测均通过，最终版本合计四轮通过。
- group 增加两个并发创建者，每人保留 32 个 Held 子域后再回收，穿过名册扩容边界，并检查源枚数量不随私有页消费累积。
- 四 hart debug group 实际输出 `concurrent builders=64`、`group: PASS` 和正常停机；开启发布禁分配检查。trace 中 hart 1 记录 33 次 Spawn，hart 2 记录 32 次，覆盖不同 Team 在不同 hart 的提交。

复现 debug 整机时，`cargo image group debug` 后运行 `QEMU_SEMI=1 QEMU_TIMEOUT=30 QEMU_SETTLE=0 cargo run --features semihosting`；开启导出 feature 时也必须给 QEMU 开启 semihosting。

当前 parser 支持静态 ELF64 little-endian RISC-V ET_EXEC；动态链接与 ET_DYN 不在本次范围。没有保留内核 ELF 格式解释代码；检索中的 SELF_ADDR 是 IPI 自检变量，与 ELF 无关。可信 Supervisor 域仍遵循原内核信任边界。

## 模拟器 CPU 对比

在同一宿主上运行 product/release，内核开启 semihosting，四 hart、固定 seed=12345，不传 -icount。启动两秒后采样四秒，随后发送 exit 并检查正常停机；旧版 9298951 与修正版交替各三轮。QEMU 进程 CPU 占用以一个逻辑核=100% 计算：旧版 77.25/79.00/78.25%，修正版 56.75/56.50/55.00%，均值 78.17→56.08%，下降约 28.3%。六轮均正常退出，无内核 panic。

单独调整 fence.i 的三轮均值仅从 78.25→77.74%，差异很小。主要改善来自减少重复扫表；结构化记录同一稳定窗口内的 Collect 为 83,498→23,231 次。结果仅表示上述场景与采样窗口的进程 CPU 时间，其他场景及不导出记录时的收益需分别测量。

修正后关闭 icount 的回归：20 个 debug 健康用例通过（无 initrd 的 scene 仍按契约报错）；accept/product/again/load/group/beat/rig 与 identity-replacement 八个 release 场景通过。四 hart debug group 输出 concurrent builders=64、group: PASS 并正常停机，发布禁分配检查未触发。

### 监督循环的后续修正

按普通 `cargo run --release` 配置关闭内核 semihosting 导出，以 `1755b19` 为本轮基线。Hierarchy 清理每轮只采样一次各任务的存活状态，重复记录与目录复用这一轮的结果；已登记的 runtime 在 prepare 中不再重复查询。请求处理中的来源、存活与授权检查仍即时执行，清理快照不跨轮保留。

Control 请求面、Hub 激活入口与发布入口加入同一只独占 Pile，请求到达时唤醒监督线程。入口发生替换时先完整安装新组，再封印并释放旧组，撤掉旧入口的等待登记；安装失败释放临时组，继续使用旧组及 10 ms 兜底。无事件时的任务退出、新 LINK 与身份别名检查改为 100 ms 间隔，因此这些后台变化的发现可能比此前晚，实际耗时还受调度及本轮工作影响。没有新增 ABI 或关闭诊断记录。

沿用四 hart、seed=12345、无 icount、启动两秒后采样四秒的交替三轮测量：基线 47.25/45.75/45.50%，本轮 3.75/3.75/4.00%，均值 46.17→3.83%，进程 CPU 时间减少约 91.7%。六轮均正常退出且无 panic。仅合并重复存活查询、保持 10 ms 间隔时，另外两轮为 28.50/27.75%，说明剩余空闲开销主要来自周期性监督查询。上述比例仍以一个逻辑核=100% 计，仅代表 product 场景的稳定采样窗口。

本轮 `cargo check -p kernel -p programs` 通过；关闭 icount 的 accept/product/again/load/group/beat/rig/identity-replacement 八个 release 整机场景全部通过。

### Loader 服务验证

- 工作区编译检查通过。
- system-fault 整机验收包含 loader 的真实 RPC：operator 入口发布、提交清单外 ELF、多个独立实例、Held 后启停、按请求者隔离、畸形镜像及范围拒绝、未领取结果超时回收、请求者退出清理及运行时资源登记。
- copy 探针验证内容相同但 VA 不同的缓存命中、256 页容量淘汰、超大段不入缓存，以及淘汰源 token 后已经构造的程序仍能 Spawn。
