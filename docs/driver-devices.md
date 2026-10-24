# 多设备 Driver 集成

本检查点位于 `codex/device-domains`，基于 PR21 的 Domain 与显式资源交付实现。
代码已取得双设备四核、八核与 1024 轮原生证据。当前检查点仍有两项既有基线失败，
且协调程序命名尚未收口，不能据此宣布整轮完成或允许合并。

## 功能与权威

Driver 扫描通用 PCI 配置快照，收集所有匹配的 VirtIO Block profile，在本域铸造独立
于 requester 的不透明 key。每个实际 requester 先取得持久 Domain，再以唯一名字登记到
Hub。PCI 不解释 VirtIO 能力，Hub 不解释 requester、BAR 或 IOVA。

每个物理设备各有一个 `Coordinator`，持有 profile、Domain、Core、当前 Operation 和
逻辑完成水位。`serve` 持有集合以及唯一一组经过认证的 PCI/IOMMU Peers。Core 的水位
保护 BAR 附着与回收边界；Coordinator 的水位保护完整协议关闭，二者不互相替代。

启动期间的快照、Domain 和登记调用使用有界 RPC；传输失败保持重试。永久拒绝属于
关键 Driver 启动失败，不能静默删除该设备后宣称全部就绪。全部 profile 建立后才进入
集合事件循环。运行期每个设备独立保留 Pending，逐拍轮询，其他设备可以继续推进。

## 异步推进与关闭

Operation 的 typed Exchange 保留实际编码的请求、原始 deadline 和响应通道。
Exchange 未结束时 Stage 不变。Release 在准备 RPC 未决时只记录关闭意图；随后必须
接收或确认该次尝试的终态，记录已取得的 BAR/范围，再进入完整清理。

没有收到 ACK 不能证明没有副作用。迟到的 Enable ACK 不能再次交付坐标或激活绑定。
坐标交付同样使用 POLL，保留同一 writer 和原始等待预算；背压不能同步阻塞邻居。
已 Active 或 Done 的操作保持静止，Done 不重建客户端入口。

Control 是整个 Work Team 停止的权威。Driver 不以本地失败代替停止证明。可重试的
准备失败保留首错和责任，等待调用方关闭或 Work 超时退出，再由 Control 证明实际停止。
关闭顺序仍为 Invalidate → Disable → 设备 reset/回读 → Quiesce → Reference Release
→ Detach → 关闭通道 → Unplace。每一步失败保留原资源与当前进度。

reset 写入与实际 status/queue-enable 回读分两拍；没有得到真实静止回读时不能 Detach。
共享引擎允许 A 关闭后 B 继续运行。每个已释放 Domain 都须有自己的 DC 清零与 retire
cookie；最后一个保留的绑定还须在实际 Off、队列静止且无错误后释放最后的数据引用。

## PCI aperture 与独立 BAR

带 ONLY 的授权分支只能交付一个 heir。从整段 aperture 直接派生并交付第一个 BAR 后，
不能再从同一分支交付第二个 BAR。单设备验收不会暴露这个限制。

PCI 现在通过既有独立子资源机制分割实际范围：取得精确、非独立保留的父 Reference，
用 Pole Unseal 建立该子段自己的根，保留子根，再释放临时父 Reference，最后交给 Driver。
每个 BAR 都有独立根，范围互不重叠；ONLY 和权限上限保持不变，没有新增内核接口。
若唯一 BAR 恰好覆盖整段声明 aperture，则保留既有精确 Pages 派生路径；整段不能作为
严格子资源分割，而且其后已经没有第二个可分配窗口。

失败时，Bar 保留临时 Reference、已建立的根和地址预留；重试不能重复分割。已发布的
根及 Driver 本地 token 跨绑定保留。Work 每次获得自己的新子授权，不能把其本地数值
与 Driver 的 token 或其他 Work 的 token 混同。
未发布的回滚先释放临时父 Reference，再释放子根；失败保留尚未释放的同一句柄。

## 已取得的原生证据

以下结果来自源码指纹
`02334d7df07d1c313b66ef4684b47af99b2d2a6b0c1ec142793a24de1545e151`，
是集成检查点，不能替代后续最终冻结验收：

- `iommu-dual` 四核、八核均退出 0：requester 8/16、不同 Domain、相同数值 IOVA
  `0x200000000`，两个不重叠 BAR。两张真实盘分别使用原花纹和 XOR 0xa5 花纹。
- A、B 各完成 DMA；B 写入后 A 的数据、used ring 和状态仍保持正确。A 关闭时共享引擎
  保持运行，B 再完成一次真实读取；B 最后关闭并完成实际 Off。两组 Work/辅助成员
  及全部静态任务均实际回收，四核帧 85→85，八核帧 153→153。
- `iommu-fail-readback` 八核退出 0：qtest 从实际 DTB/PCI 配置取得物理寄存器地址，
  将真实 VirtIO status 保持为 1。生产 Driver 返回 Stop Unavailable，并保留已失效
  但未释放的六页责任；恢复实际 status=0 后，同一绑定完成停链，首错保持
  party=4/op=2/code=4。后续重绑与旧 IOVA 负证也通过，帧 153→153。

首次双设备运行真实发现第二个 BAR 被拒绝，已保留失败现场。修复后的第一次运行又
揭示判定器误把累计 cull 分支计数当成活任务计数；现改为核对实际 reaps、根最后退出
和物理帧回归，保留原失败结果并重新运行，未把失败记录改称通过。

## 最终冻结验收进度

当前原生运行使用固定客体源码指纹
`79b06726d22dcaa42f40c1c11dbc14d92f466316de0341ec14a39481b9456c8a`。
完整结果分别保存在 `target/driver-composition/device-domains/` 下的
`native-device-final2-matrix-current.json` 与
`native-device-final2-remaining-current.json`；原失败记录保留。

- 已通过双设备与真实 reset 回读拒绝各四/八核、产品四/八核、旧绑定地址负证、
  发布前后中止、真实 CQ 故障、Reference Release Busy、失效/停机/释放确认门、
  Ruin、forced 和基础 DMA 四/八核。停机确认门拒绝实际 PCI Disable 请求，
  在移除注入前仍严格要求两个实际保留引用。
- 普通产品四/八核、真实 Shell 八核、Daemon 的 RTC/目标死亡/显式停止八核、
  accept 四核，以及真实八核内存转移竞态均已通过完整包装器验收。
- PCI/IOMMU/Recovery 三个关键 Driver 各四/八核的真实在途 DMA 死亡均通过独立
  判定。原始 qtest 退出码仍为预期的 101，独立判定器为 0；实际 NBD 请求保持
  未回复，引用清理未开始，其他三个/七个核完成静止确认。这是保护性终止证据，
  不宣称完成 DMA 排空或回收。
- 宿主 Rust 65 组、774 项通过；设备证据判定器 Python 126 项、竞态脚本 27 项通过。
  独立 Flash 审查未发现当前集成差异中已确认的生产阻断问题。

竞态批次首次遗漏 `--feed-script`，第二次暴露宿主脚本依赖输出目录深度的旧假设。
修正后脚本从自身所在仓库取得本轮结构化导出，嵌套输出目录回归与原生严格验收均
通过。qtest 在启动虚拟机前清除旧导出；没有放宽等待、唤醒、任务回收或帧检查。

八核 1024 轮双设备长测已完整通过，客体用时 3038.54 秒。判定器核验了 2048 个实际
绑定、4096 个主/辅助任务的回收、3072 次真实读取、每轮能力总数 17、相同数值 IOVA
的域间隔离、逐域新地址、服务最后退役和物理帧 153→153。原生结果与判定器结果均为 0。

九个既有四核基线已有七个通过，另外两项实际失败并保留现场：

- `system-fault` 原样复跑仍在 Loader IPC 夹具的 12 秒断言超时。该夹具手写排程只接收
  Control 请求，却未执行生产 `control::poll` 中的查询应答阶段。卡住的 18 字节帧是
  构造子任务的 bootstrap claim，其内部认证查询无人处理；不能把它误认成 Publisher
  请求本身。须复用生产排程处理，并保留原期限、所有权和最终回收断言。
- `iommu-product-start-before` 客体退出 0，包装器仍因缺少真实 `detached` 记录失败。
  异步推进允许执行者在 BAR Attach 前关闭，生产清理没有伪造一次不存在的映射回收。
  该夹具要验证映射回收，应先等待真实 BAR 授权到达，再在不交付 DMA 窗口的前提下
  中止，继续保留严格的 detach 与最终回收检查。

上述两项将在后续集成收尾中修复并重跑。协调程序当前仍名为 `recovery`；问题单 #15 的
完整生命周期命名及模块边界也须结清，之后以最终源码再次验收。当前是可审查检查点，
尚未宣布整轮重构完成。
